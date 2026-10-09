# V2.5 implementation and validation result

## Outcome

V2.5 completed the requested whole-framework survey and safety reviews. No
production allocator, ownership, collection, hashing, or frozen-layout
optimization cleared the evidence and safety gates, so those runtime contracts
remain unchanged. The checked-in changes add reproducible profiling and B10
worker-scaling tools, record the memory/layout baseline, and test the V2.4
owner and descriptor sizes. The temporary `PLAN*.md` handoff files are removed
in this final commit; their decisions and open work are preserved in the
workstream reports below.

The source was fast-forwarded to `ec68fb7` on `main` before this work. The last
source commit before this report was `d891adc`; this report and plan-file
cleanup are committed together.

## Workstream decisions

| Workstream | Result |
| --- | --- |
| Profiling | Captured two clean, alternating 16-scenario suites, nine paired runs for eight noisy scenarios, 31 symbol reports, all five available hardware events, and 1/2/4/8-worker B10 weak-scaling data. The prioritized experiment matrix is in [PROFILE_V2_5_BASELINE.md](PROFILE_V2_5_BASELINE.md). |
| Allocator | B10 remains the largest ordinary API gap: 5.03x compact/native in the clean default suite; its compact profile shows acquire/release atomics and mutex contention. B10 scaling on this two-vCPU host measured 4.90x–6.06x latency and 5.67x–6.91x cycles from two through eight workers. A bounded owner-affine chunk design is documented, but remains blocked on remote-free routing, TLS exit/reaper coordination, duplicate-free handling, coherent accounting, and concurrent proof. No unsafe allocator implementation was added. |
| Ownership | The proposed generic cross-crate resolved-view API is deferred. Existing owner-borrowed slice methods, `CompactVec::as_mut_slice`, and `CompactVecDeque::with_view` already provide the measured batch-resolution benefit; a new public type showed no incremental gain. See [OWNERSHIP_DECISION.md](OWNERSHIP_DECISION.md). |
| Collections | The focused A4 `with_view` path was 8.45x faster than compact per-operation calls with matching checksums, but it is opt-in and its isolated RSS was not measured. A5/B8 hash-layout changes remain rejected; randomized hashing stays intact. No collection source change was accepted. See [COLLECTIONS_V2_5.md](COLLECTIONS_V2_5.md). |
| Memory | Four-byte owners, the 12-byte deque, 8-byte frozen descriptors, and the 16-byte allocation header remain unchanged. The 1/2/4/8-worker B10 capture scales retained cage bytes with the fixed per-worker input; it found no evidence of a new unbounded cache. Virtual mappings, page commitment, and post-thread-exit idle RSS remain unmeasured. See [PROFILE_V2_5_MEMORY.md](PROFILE_V2_5_MEMORY.md). |

No memory-budget exception is requested. The measured B8 diagnostic reached a
5,211,232-byte cursor during cache churn; [PROFILE_V2_4_ROUND4.md](PROFILE_V2_4_ROUND4.md)
now distinguishes that trace from its lower per-phase high-water figure.

## Performance summary

On the captured two-vCPU Neoverse-N1 host, compact remained faster in A1, A3,
B3, B5, B7, and B9. The largest clean compact/native median ratios were B10
5.03x, A4 3.28x, A5 2.60x, B8 2.09x, and B6 2.04x. Nine-run noisy samples
confirmed the A4/A5/B8/B10 gaps and showed wider native B10 tail variation.
These independent V2.4/V2.5 captures use unchanged runtime code; ratio
differences between captures are not attributed to a source change. Full
latencies, p95s, counters, symbols, checksums, and the experiment matrix are in
the profiling report.

The B10 worker override is benchmark-only: `CSL_B10_WORKERS=1..8`, default 2,
with 8,000 records per worker. The 4/8-worker captures oversubscribe this
two-vCPU host and do not claim physical multicore throughput. Compact/native
checksums matched at every worker count.

## Validation

Passed on the final runtime and harness source:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`, including the new
  `memory_layout` test (2/2)
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- All seven steps in `.github/workflows/miri.yml`: 82 tests passed across
  `compact_core` (3), `cage_collections` (22), `deque_view` (8),
  `collection_batch_access` (9), backend integration (1), backend library
  (32), and V2.4 behavior (7)
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- `.github/workflows/harness.yml` checksum parity command; all 16 native,
  compact, measure, and profile checksums matched with the default two B10
  workers
- `bash -n scripts/profile_cpu.sh` and the saved-run summary command; checksums
  verified for the 16-scenario suite and eight-scenario noisy subset
- Independent review found that the counter tool default did not match the
  three-repeat report; the default is now three and the shell syntax check
  passes.

Clippy first stopped with `ENOSPC` while writing Cargo's query cache. After
clearing the authorized Cargo target caches, the same strict Clippy command
passed. The isolated profiling binaries and temporary collection probe were
also removed through Cargo cache cleanup after their captures were complete.

## Remaining limits

- This host has two vCPUs; allocator behavior on a four/eight-core machine is
  not established.
- Perf did not collect task-scoped native B10 samples. The 99 Hz bpftrace
  fallback produced 432 weighted samples, mostly unresolved addresses; it
  supports process attribution but not function-level native comparison.
- The benchmark harness does not record `VmSize`, committed pages, or idle RSS
  after thread exit. No TLS chunk/cache design was implemented, so those are
  prerequisites for a future allocator experiment rather than unverified
  V2.5 changes.
- The collection batch path's isolated RSS was not captured; it remains an
  opt-in API usage pattern, not a new default.
