# PLAN_ALLOCATOR_CONCURRENCY.md — B10 synchronization and allocation state

## Implementation scope
Own crates/compact_backend_std/src/cage.rs and allocator-specific tests. Inspect exact symbols: lock, AllocatorTransaction, CageAllocation<T>::allocate, allocate_block, allocate_from_cursor, take_pending_reuse, release_many_locked, read_header, ReleaseCollector::{take_compatible,flush}. Read PROFILE_ALLOCATOR.md, PROFILE_V2_4_ROUND2.md and PROFILE_V2_4_ROUND3.md. Shared benchmark and collection code is read-only.

## Verified facts
Shared allocator state is mutex-guarded. B10 system-wide sampling attributes substantial self CPU to CAS/swap/futex; host was two vCPU and timing noisy. Thread-local chunk caching/lock-free reclamation was explicitly deferred pending safety design. B8 hash overhead exceeds allocator release in earlier sampling. A2 ~1.5x native accounting-free.

## Phase 1: reliable contention attribution
Profile 1/2/4/8 workers where CPU count permits; repeat on another multicore host if available. Record throughput, median/p95, lock acquisitions, wait/hold time, scheduler preemption, atomic/futex samples, allocation size classes, thread-local reuse and remote-free rate. Don't use old high-overhead uprobe timings as production lock latency.

## Phase 2: isolated alternatives
A: Move invariant-independent layout/calculation outside mutex and test shorter hold time.
B: Prototype bounded thread-local cage reservations reducing per-object global lock acquisitions.
C: Prototype bounded local compatible free-extent reuse, independently.
Measure each separately against frozen baseline and against each other; do not merge speculative complexity.

## Mandatory state-machine design gate
Define ownership and legal transitions among globally free, per-thread reserved (unused slack), live suballocation, pending release, and reclaimable. Identify authoritative state for individual live bytes and reserved bytes; preserve existing public accounting or document new separate counters. Specify publication ordering, remote destruction, cross-thread transfer, thread exit, cache teardown, nested collectors, panic/unwind, interrupted operations, exhaustion/replenishment, fragmentation/coalescing and recovery. Never make chunk globally reusable while any suballocation is live. Prevent duplicate allocation, double frees, ABA-like reuse and orphaned extents. Provide concrete state-transition/model tests and central safety review **before** any new unsafe chunk implementation; a full lock-free reclamation redesign needs separate approval.

## Tests
Deterministic barriers/coordination rather than real sleeps; randomized allocation/free/reuse, remote free, thread exit, panic/drop, exhausted/near-full cage, fragmented reuse, accounting, alignment, concurrent stress and Miri-supported invariants. No user destructor while mutex held.

## Acceptance/handoff
Repeatably improve B10 throughput/latency across worker counts without materially harming A2/B8/B3/B5 or ballooning slack/RSS. Report architecture contract, source/commit SHA, tests and medians/p95, memory/fragmentation, failures, rejected experiments and residual risk.
