# PLAN.md — V2.5 Deterministic Buffer Growth and Reuse Experiments

Repository: `Biddlebaddleboo/compact-std-library-rust`; target branch: `main`; verified baseline: `3448c8ce313d840012a6498386d760966e68881b`.

## Objective and boundaries

Experiment with faster **deterministic buffer growth, in-place resize, capacity shrinkage, replacement allocation and ownership transfer** in the current single cage allocator. Seek fewer allocations, copies, free-list scans and synchronization events while preserving safe reclaimability, existing compact layouts, and the successful pre-contention 32-byte reuse rule. Treat this as an experimental optimization: benchmark and reject unsuccessful candidates; do not force architectural change, add an allocator hierarchy, add background GC, or move live objects without safe exclusive ownership.

Before implementation, verify latest remote main and reconcile relevant changes. This temporary plan is for execution handoff only; remove PLAN*.md before the final code commit.

## Verified repository facts

- `crates/compact_backend_std/src/cage.rs::CageAllocation<T>::try_resize` verifies requested capacity/initialized length, obtains `lock_for_allocation`, and calls `try_resize_locked`. On failure when local cached bytes exist, it flushes the calling thread's cache, relocks and retries once.
- `try_resize_locked` merges global size-class free ranges when size-class counts are nonzero; shrink/unchanged-physical-size operations update capacity/header and return unused tail via `insert_free`; growth succeeds at the global cursor or by consuming **immediately adjacent** free space (`free_node_at`, `consume_free_prefix`), otherwise returns `false`.
- `crates/compact_collections/src/vec.rs::CompactVec::reserve` uses geometric growth, tries `try_resize`, then allocates replacement `CageAllocation<T>` and `move_into`; `shrink_to_fit` follows the same try-resize/fallback structure.
- `PROFILE_V2_5_ALLOCATION_POLICIES.md` reports B2 250 resize attempts: 10 no-growth, 10 cursor growth, 20 free-range growth and **210 cannot grow in place**. This is a hypothesis for investigation, not proof that these failures are avoidable or a large fraction of end-to-end time.
- Current local exact 32-byte pre-contention reuse is accepted. B10 two-worker improvements must be protected; B4/A2/B7 and B3/B5 need regression coverage. Prior Miri/workspace validation passed, but sequential allocator models/Miri alone do not prove concurrent ordering.
- `CageAllocation<T>` uses a four-byte owner, `AllocationHeader` is sixteen bytes, `CompactVec<T>` is four bytes, `CompactVecDeque<T>` is twelve bytes. Do not change these default layouts.

## Implementation scope — exact starting symbols

**Exclusive production owner:** `crates/compact_backend_std/src/cage.rs`
- `CageAllocation<T>::try_resize`, `try_resize_locked`, `allocate`, `move_into`
- `allocate_block`, `allocate_from_cursor`, `block_layout`
- `free_node_at`, `consume_free_prefix`, `insert_free`, `merge_free_ranges_locked`, `release_many_locked`, `lock_for_allocation`
- `CompactRuntime::allocator_stats` and gated resize telemetry.

**Conditional collection write scope:** `crates/compact_collections/src/vec.rs` `CompactVec::{reserve,push,shrink_to_fit}`. Read first; write only if actual measured improvement and safety evidence require it.

**Read-only initially:** `crates/compact_collections/src/deque.rs`, `crates/compact_collections/src/hash_map.rs`, `crates/compact_backend_std/src/deterministic_memory.rs`, `crates/compact_backend_std/src/allocator_model.rs`.

**Tests/harness:** `crates/compact_backend_std/tests/integration.rs`, allocator tests in `cage.rs`, `crates/compact_std/examples/benchmark_compare/{scenarios.rs,main.rs}`, `crates/compact_std/examples/memory_profile.rs`. Proposed results report: `PROFILE_V2_5_BUFFER_REUSE.md`.

Avoid broad exploration unless a symbol moved, architecture changed, compilation/tests point elsewhere, compatibility requires examining another caller, or correctness/safety requires expansion.

## Phase 1 — Diagnose actual resize failure reasons

Add opt-in telemetry, with **no expensive diagnostics in telemetry-free timing builds**, to distinguish:
- Live next allocation blocks adjacent growth.
- Adjacent free extent too short.
- Free space available elsewhere but noncontiguous.
- Adjacent released extent pending publication or in size-class/local cache.
- Cursor/cage capacity or alignment/layout prevents growth.
- Invalid/overflow capacity.

Measure old and requested physical length, incremental bytes, replacement allocations, bytes moved, lock acquisitions, merges/free-list visits, retry/flush benefit and in-place outcomes. Distinguish `try_resize` returning false from a true `AllocationExhausted` error. In B2, classify all 210 failures and determine which are inherently blocked by a live neighbor; do not claim that scattered free bytes make in-place growth possible.

## Phase 2 — Independent reversible candidates

**A. Avoid unnecessary global merging.** `try_resize_locked` currently calls `merge_free_ranges_locked` whenever any size-class cache is populated. Evaluate a cheap authoritative test to skip merges that cannot help, measuring miss cost vs saved merging. No speculative unsafe metadata reads or extra costly scans.

**B. Cheap adjacent extent consumption.** Optimize identification/consumption of `end = start + old_len` neighbor when it is actually free. Preserve exact/free-larger cases, alignment, header size, split minimum, sub-FREE_NODE_BYTES remainders, cursor and accounting. No overlap with another live or pending owner.

**C. Faster negative resize result/retry.** Measure whether the local-cache flush and second global lock can be omitted when adjacent local capacity cannot contribute. Recheck under the authoritative lock when races are possible; preserve correct concurrent publication and later retries. Do not introduce unsynchronized free-list inspection.

**D. Replacement allocation reuse.** When contiguous growth is impossible, measure whether `allocate_block` can obtain suitable existing shared free extents efficiently. Bounded scan and correct alignment/layout; avoid unbounded fragmentation or special allocator layers. Preserve old valid owner until replacement capacity and transfer succeed.

**E. Cheaper ownership transfer.** Profile `CompactVec::reserve → try_resize → alloc_owned_slice → move_into → old release` for metadata duplication, lock traffic and actual bytes copied. Optimize only where source/destination exclusive ownership and exact initialized count are proved. For non-Copy values use movement, not cloning or duplicate Drop; if allocation/transfer errors or unwinding occur, preserve valid prefix exactly once.

**F. Capacity-growth policy experiment.** Compare incumbent geometric growth to size-bucket or alternate factor only in isolated tests; quantify fewer reallocations versus extra retained capacity. No collection-name/benchmark-ID branching. Abandon candidates exceeding memory budget.

**G. Shrink/tail reuse.** Evaluate fast deterministic tail split/coalescing and repeated grow/shrink cycles. Retain valid references/initialized entries and reclaim independent free tails; optimize only if demonstrated worth.

Test candidates individually; record accepted/rejected results. Avoid silently coupling several changes into one unmeasurable candidate.

## Safety, concurrency and failure invariants

- Every live allocation has exactly one owning release path; all live extents are disjoint and properly aligned with accurate header `block_len`, `capacity`, `initialized`, `prefix`.
- Resize failure must leave the old allocation and initialized elements intact, unless prior successful explicit operation already changed it. Avoid partial metadata writes if an error can occur; order validation and publication transactionally.
- In-place growth consumes only genuinely free **contiguous** bytes, under the existing authoritative synchronization; never consume a pending remote release or another thread's cache/extent without verified publication.
- Shrink publishes only non-live uninitialized tail storage; no double-free, descriptor loss, dangling references or ABA reuse.
- Replacement move destroys initialized values exactly once, with sound panic/unwind and allocation-exhaustion rollback. No user destructors under allocator locks.
- Thread exit and remote frees remain safe and reclaimable without a worker. No unbounded foreground scan, cache or metadata growth, or background thread dependencies.
- Preserve safe poison/fault-closed behavior and no lost release descriptors. Test cross-thread release simultaneously with adjacent growth, reentrant batching, panicking destructor, invalid layout, allocation failure and repeated churn.

## Benchmarks and acceptance

Primary B2, plus isolated deterministic cases:
- Geometric vector growth.
- End-of-cursor growth.
- Adjacent eligible free extent (exact/larger).
- Live neighbor blocking growth.
- Adjacent pending/cached release and retry.
- Shrink and immediate compatible reuse.
- Alternating medium/large allocations.
- Multiple long-lived owners and fragmented free space.

Run **all sixteen** existing scenarios and protect B10 one/two workers, B4, A2, B7, B3/B5. Compare against `3448c8ce` candidate baseline and use `PROFILE_V2_5_ALLOCATION_POLICIES.md` as measurement context. Measure median/p95 and p99/p99.9 only with adequate samples, in-place success fraction, replacement allocs, bytes moved, locks, scans, cursor high-water, fragmentation, retained/peak RSS, checksums. Use repeated paired counterbalanced measurements; separate telemetry builds from timing builds. Existing two-vCPU AArch64 machine sufficient. Four/eight workers optional oversubscription only.

Default memory budget vs current baseline: **≤+2% retained memory and ≤+5% peak RSS**; no silent exceptions. Preserve existing layouts, APIs, default 32-byte reuse, single cage allocator and synchronous deterministic reclamation. Only accept reproducibly favorable improvements with no serious regression; reject complexity with no demonstrated gain. If B2 failures are mostly live-neighbor blockers, focus on reducing false-path work and replacement transfer rather than unsafe in-place growth.

## Validation commands and evidence

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```

Run complete repository Miri and benchmark-harness workflows, supported Apple cross-target checks, deterministic concurrent/barrier tests and independent diff inspection. Do not mistake Miri or a sequential model for full concurrency proof. Prefer injected schedules and barriers, not real sleeps.

## Workstream ownership / execution handoff

This is **one tightly coupled workstream** because resize, free extent management and replacement allocation modify shared `cage.rs` metadata. Baseline measurements or independent review can run separately with non-overlapping writes, but do not split production symbols across parallel executors.

1. Verify latest remote main; reconcile relevant changes and pin baseline.
2. Read this PLAN.md first and inspect named symbols; collect phase-1 failure classifications.
3. Prototype each candidate in isolation; validate/benchmark before acceptance.
4. Integrate accepted candidates only; document precisely why other candidates were rejected.
5. Run all safety, full-suite performance and memory gates.
6. Write `PROFILE_V2_5_BUFFER_REUSE.md` containing source/benchmark SHAs, changes, counts, distributions, regression and correctness evidence, deviations, unresolved issues.
7. Independently verify final diff against this plan; resolve contradictions centrally.
8. **Delete all PLAN*.md before final implementation commit**. Report changed files, implementation commit SHA, tests and remaining assumptions.

No implementation is performed by committing this planning artifact.
