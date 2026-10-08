# PROFILE_V2_4_ROUND3.md — Round-three investigation and integrated results

Baseline: `main` at `fbff055` (round-three plan). Verified pre-round head:
`1246961`. Host: AArch64 Neoverse-N1, 2 vCPU, Linux, aarch64; default cargo
1.95.0, nightly cargo 1.101.0. All timing is accounting-free
`benchmark_profile` release; `benchmark_compare --mode measure` is used only for
allocation statistics. Logical checksums are compared between variants.

## 1. Benchmark CI repair (gate)

GitHub Actions run `37725406606` (job `113142322037`) failed at the harness
contract step, not at checksum comparison:

```
error: cannot update the lock file .../Cargo.lock because --locked was passed to prevent this
```

Root cause (reproduced): the committed `Cargo.lock` contained seven
`[[patch.unused]]` blocks for `compact_core`/`compact_backend_std`/
`compact_collections`/`compact_frozen`/`compact_macros`/`compact_serde`/
`compact_std` v2.2.0. Those blocks only exist because the developer host has a
`[patch.crates-io]` configuration mapping those crates to a local path; CI has
no such configuration, so cargo must remove the blocks and `--locked` aborts.

Fix: remove the 28 `[[patch.unused]]` lines (commit `75d4d59`). The lock stays at
format version 3 and dependency resolution is byte-identical otherwise.

Verification in a config-free shell (host patch config temporarily set aside,
restored afterwards):

- `cargo metadata --locked` → exit 0.
- `cargo +nightly tree --locked -p compact_std --features json,toml` → exit 0.
- `cargo +nightly tree --locked --workspace --all-features` → exit 0.
- `cargo run --release -p compact_std --example benchmark_compare --features
  json,toml -- --self-check` → 16/16 scenarios `ok`.

Lockfile policy is documented in `BENCHMARKS.md`. Note for local devs: with a
host `[patch.crates-io]` active, any cargo invocation re-writes the phantom
entries into the tracked lock; restore with `git checkout -- Cargo.lock` before
committing.

## 2. Hash lookup probe — accepted

`find_slot_in` tracks the earliest tombstone so `insert` can reuse an
insertion slot. Read-only `get`/`get_key_value`/`get_mut`/`remove_entry` do not
need a slot, so they now use `find_index_in` (commit `23aa608`), which skips
tombstones and stops at the first true `EMPTY`.

Correctness argument: the probe order, group classification, key equality and
the stop condition (first `EMPTY`) are identical to `find_slot_in`; only the
tombstone bookkeeping is dropped. A key is always reachable before the first
`EMPTY` under the open-addressing/insertion invariant. `find_slot_in` is
unchanged and still backs `insert`. The randomized collision-resistant default
hash and the hash-flood defense are untouched.

Measured (two independent A/B captures, interleaved, accounting-free profile
mode, 5–9 rounds):

| Phase (A5) | baseline | candidate |
|---|---|---|
| `lookup_scan` | 0.459–0.465 ms | 0.445–0.456 ms |
| `mutation` | 0.774–0.790 ms | 0.757–0.763 ms |
| `end_to_end` | 3.230–3.266 ms | 3.119–3.165 ms |

B8 `cache_lookup_and_scan` and B5 were neutral; logical checksum identical.
Gates: `fmt`, strict Clippy, `cargo test --workspace --all-features`, and Miri
(`cage_collections` 22/22, plus `hash_map` unit tests) all green.

## 3. Deque borrow-scoped batch view — accepted, opt-in

`CompactVecDeque::{push_back,pop_front}` call `resolved_mut()`, which performs a
full header read plus two validation passes; on steady-state FIFO churn that
re-validation dominates the memory move. `CompactVecDeque::with_view` (commit
`8bda8aa`) resolves the ring once for a batch of non-growing operations.

Design constraints held: the view stores only a borrow-scoped slot slice plus
head/len and writes metadata back on return / early return / unwind; the deque
keeps its frozen 12-byte layout and never retains a native pointer. Growth while
borrowed is disallowed — reserve first; an over-capacity `push_*` returns
`AllocationExhausted` and leaves the deque unchanged and valid.

Measured with the batched call pattern (the shared A4 arm was wired to it only
for measurement, then reverted):

- A4 `mutation`: 1.252 ms → 0.119 ms (native per-op `VecDeque` churn: 0.356 ms).
- A4 `end_to_end`: 1.292 ms → 0.157 ms.
- Logical checksum identical; repeatable across three runs.

**Not wired into the shared benchmark.** A batched compact arm measured against
a per-op native arm would report an API advantage, not a per-op cost, and would
break like-for-like comparison. The pattern is documented in `BENCHMARKS.md`.
The additive API means existing per-operation methods — and their single-op
cost — are unchanged. B4 (`event_history`) sees no gain (construction dominates
there) and is untouched. Tests: `deque_view` 8/8 and `cage_collections` 22/22
under Miri; `deque_view` was added to the Miri workflow.

## 4. Allocator (B10/B8) — analysis, no landed change

- B10 is dominated by lock/atomic/futex machinery (system-wide sampling, not a
  single number). Safe, local critical-section shortenings do not reach the
  target; the local host is noisy (compact medians span roughly 0.9–2.8 ms).
- The only closed-form fix — bounded thread-local chunks or lock-free
  reclamation — is gated by the plan and was **not** implemented. Before any
  such change, a contract is required covering: chunk reservation, per-live-
  allocation accounting, unused-slack accounting, publication, remote frees,
  cross-thread transfer, thread exit, exhaustion/replenishment, fragmentation,
  panic and reclamation, plus proof that no chunk is freed/reused while any
  suballocation is live. Recommendation: do not implement without that contract
  and central approval.
- B8's allocator share is smaller than its hash share; no free-list/coalescing/
  tail change showed a repeatable win with an exact-accounting proof, so none
  was landed. Pending exact reuse is preserved.

## 5. Vector (B6) and header validation — analysis, no landed change

- B6: batching `CompactVec::as_mut_slice()` once per order-book update round is
  a caller pattern (consistent with round two), not a new API and not a
  benchmark-only change. The frozen 4-byte `CompactVec` layout is unchanged.
- Header validation: optimized codegen shows `read_header` / `validate_typed_header`
  fully inlined; no safe redundant range check could be removed without
  weakening typed-header or lifetime validation. Any cage.rs change belongs to
  the allocator owner; none was made.

## 6. Accepted round-three changes

- `75d4d59` — drop host-only `[[patch.unused]]` entries from `Cargo.lock` (CI fix).
- `23aa608` — hash lookup probe without tombstone tracking.
- `8bda8aa` — deque borrow-scoped batch view (opt-in).

Rejected this round (with reasons above): hash fingerprints (security/memory
tradeoff not proven), deque harness rewire (metric integrity), thread-local
allocator chunks (gated, no approved contract), B8 free-list changes (no proven
win), header-validation eliminations (no safe reduction).

Validation for the integrated tree: `cargo fmt --check`, `cargo check --workspace
--all-features`, `cargo test --workspace --all-features`, strict Clippy, the full
`miri.yml` step set, and `benchmark_compare --self-check` parity.
