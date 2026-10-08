# PLAN_ALLOCATOR.md — B10 Contention and B8 Release

## Scope
Own crates/compact_backend_std/src/cage.rs and allocator tests. Start lock, AllocatorTransaction, CageAllocation<T>::allocate, allocate_block, allocate_from_cursor, take_pending_reuse, ReleaseCollector::{take_compatible,flush}, release_many_locked, read_header. Read B10/B8/A2 profiles.

## Evidence
Current B10 sampling ~62% sync/atomic/futex self samples under system-wide capture; exact production contention/lock wait must be remeasured. B8 hash is larger than release cost. A2 is ~1.53x accounting-free baseline, so protect its timing. Global mutex serializes allocator state.

## Phase 1
Capture low-overhead all-thread profiles, lock acquisitions per allocation, wait/hold versus scheduler preemption and CPU count scaling. Compare native at equal thread counts.

## Phase 2
Measure critical-section operations and safely move invariant-independent work outside lock. Preserve uniqueness/coalescing/accounting, mutex-poison handling, no destructors inside mutex, cross-thread frees and allocator correctness. A/B test.

## Phase 3: gated thread-local chunks
Prototype bounded per-thread chunks only after documenting ownership/accounting contract and central approval of design. Define reservation, individual live allocation accounting, unused slack, publication, remote frees, cross-thread transfer, thread exit, exhaustion/replenishment, fragmentation, panic and reclamation. Never free/reuse a chunk while any suballocation live. Compare against simpler locking changes. Do not implement lock-free reclamation absent separate approval.

## Phase 4: B8 release
Reprofile after hash changes. Assess ordered batches eliminating unnecessary sort only with proof, general free-list traversal, coalescing and tail contraction. Maintain extent validation, overlap detection and exact bytes. Keep pending exact reuse.

## Tests/acceptance
Deterministic barrier-based concurrency, allocation/free/reuse, remote thread drop, panic, near-full exhaustion and recovery, fragmentation, Miri. Repeat B10 across load/core counts, protect A2/B3/B5 and B8. Report SHA, design, timings, memory, failures and rejected trials.
