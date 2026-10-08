# PLAN_ALLOCATOR.md — A2/B8 allocation, recycling and fragmentation

## Verified baseline
At 4a57dc713f1158347b2c912b6d374ea4dd7213f7, A2 is 1.47–1.50x native and B8 2.73–2.76x native. Pending exact reuse already occurs before the allocator lock. ReleaseCollector holds up to 64 pending extents and checks candidates in reverse order. Four global exact-size classes remain disabled by default after unsuccessful measured hit rates. Preserve 4-byte owner, 16-byte header and present B8 drop gains.

## Implementation scope and ownership
Start at crates/compact_backend_std/src/cage.rs: CageAllocation<T>::allocate, take_pending_reuse, ReleaseCollector::{take_compatible,push,flush}, ACTIVE_RELEASE_COLLECTOR_COUNT, allocate_block, release_many_locked, AllocatorTransaction, AllocatorStats. Own cage.rs and allocator-specific test files; benchmark_compare source and documentation are read-only/shared central surfaces.

## A2
Instrument phase-level TLS check, layout, locking/contended waits, free-list scanning, bump allocation and fresh header initialization. Use build- or test-gated counters; do not perturb timed runs. Assess narrow mutex-protected empty-free-list bump fast path, eliminating redundant computations and bounds checks where safely possible. No lock-free cursor.

## B8
Profile pending collector exact-match lookup cost, hits/misses, scan length and extent sizes. Evaluate a bounded last-freed slot or size-indexed search inside the collector; ensure entries cannot appear twice and reused pending extents never flush. Test carefully against existing reverse bounded scan, including cache-cold and diverse-size patterns. Evaluate batching remove/insert where semantics and ownership permit; do not change user API behavior. Measure B8 churn, drop, end-to-end, free scans, locks, high-water, free bytes and fragmentation.

## Regions and pools (measurement-gated)
Benchmark an optional temporary lifetime-scoped allocation policy for objects released together; do not ship a general region allocator without proof of benefit, an enforceable lifetime contract and no live individually owned values when a region releases. Avoid global allocator and object representation redesign.

## Invariants and recovery
An extent is exclusively live, pending or globally free. Reuse must reinitialize header before owner publication, preserve live_bytes, respect capacity/alignment and cross-thread exclusion. Never run user Drop under allocator lock. Panic/unwind, nested teardown, reentrancy, exhausted cage and error return must preserve ownership. Preserve free-list sorted/coalesced/tail-contracted state and class-policy correctness.

## Deterministic tests
Exact and nonmatching releases, alignment, collector 64-item saturation, nested destructor panic, reuse then flush, double release prevention, same-thread and cross-thread isolation, heavy contention, tiny and large sizes, ZSTs where supported, near-exhaustion and fragmented reuse. Add state-model/property tests for live/pending/free exclusivity, byte accounting and maximal non-overlap; prefer deterministic fake state to timing/sleeps.

## Acceptance and handoff
Compare with pinned A2/B8 and B10 and real-world B3/B5. Retain only measured gains with no meaningful B10 or density regression. Report files/SHA, before/after phase timings, locks, scans, bytes and fragmentation, test/Miri outcomes, rejected experiments, remaining risks. Never add prefetch, asm, native retained pointers or new per-object header fields.
