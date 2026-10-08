# PLAN_ALLOCATOR_PROFILING.md — A2/B8 allocator diagnosis

## Objective
Explain remaining A2 (1.41–1.44x native) and B8 (2.51–2.55x native) CPU cost without changing allocator behavior.

## Implementation scope
Start at `crates/compact_backend_std/src/cage.rs`: `CageAllocation<T>::allocate`, `take_pending_reuse`, `ReleaseCollector::take_compatible`, `ReleaseCollector::flush`, `allocate_block`, `release_many_locked`, `AllocatorTransaction`, and `AllocatorStats`. Read `crates/compact_backend_std/Cargo.toml` feature flags and benchmark harness telemetry output.

## Verified baseline
Earlier B8 telemetry reported 524,340 pending exact-reuse hits, 3.25 candidates scanned on average during an active lookup, 178,375 mutex acquisitions and 3,836,500 free-list node visits in the benchmark child (warm-up and measured repetitions). These are workload totals, not proof of a dominant wall-clock cause.

## Measurements
- Classify allocations as pending-reuse hits, free-list reuse, cursor growth or failure.
- Quantify size histogram, call count, alignments, collector scan length, successful and unsuccessful lookups, lock acquisitions/wait time/critical section duration, free-list visits and tail contractions.
- Distinguish time spent allocating, constructing, releasing, batching, validating and coalescing extents. Profile A2 uncontended mutex cost separately from B10 multithread contention.
- For B8, attribute cost between allocator and surrounding collection/object logic.
- Quantify fragmentation via high-water cursor, free bytes and blocks, largest free block, near-exhaustion cases and peak resident memory.
- Capture telemetry in a diagnostic build, and compare performance only with a separate no-telemetry release build. Explain overlap and instrumentation overhead in cumulative timing counters.
- Use bounded diagnostic/test probes, not new general allocator features; do not attempt speculative region or size-index changes in this profiling pass.

## Test and data integrity
Use deterministic sizes, alternating patterns, near-full cage and contention cases; verify logical equivalence, precise byte accounting and no safety-invariant regressions. If unsafe production code must be instrumented, isolate the diagnostic changes and run Miri. Never introduce new retained fields, prefetch or allocator policy changes.

## Write scope and handoff
Produce `PROFILE_ALLOCATOR.md` with exact measured hotspots, distributions, retained/RSS distinction, hypotheses supported and rejected, and ranked candidates. Avoid changes to shared benchmark files without orchestrator ownership; production allocator remains unchanged. Report commands, SHA, measurements, tests, uncertainty and deviations.
