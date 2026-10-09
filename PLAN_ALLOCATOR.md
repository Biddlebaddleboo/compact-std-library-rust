# PLAN_ALLOCATOR.md — Allocator and Concurrency Contract Redesign

## Scope
Own `crates/compact_backend_std/src/cage.rs`, test-only `allocator_model.rs`, allocator tests. Start exact functions `lock`, `AllocatorTransaction`, `CageAllocation<T>::allocate`, `allocate_block`, `allocate_from_cursor`, `take_pending_reuse`, `release_many_locked`, `read_header`, `ReleaseCollector::{take_compatible,flush}`. Read Round 4 profiler and model. Collections and shared harness read-only.

## Facts and hypotheses
The global mutex is a measured B10 synchronization hotspot on two-vCPU Neoverse; timings noisy. Round 4 tested alignment precompute and header initialization outside lock, both rejected for A2/B3 or other regressions. Sequential model is not a concurrent proof. Analyze B10 with 1/2/4/8+ workers where supported, B8 reuse and A2 allocations, plus B3/B5 sentinels.

## Candidate contracts
A. Bounded demand-driven TLS allocation chunks within the same cage, with exact per-live and separate reserved/slack counters.
B. Local compatible free-extent reuse and remote-release queues.
C. Shorter or partitioned global critical sections / sharded metadata if sampled evidence supports.
D. Alternative synchronization/reclamation only after proven need; lock-free is not an objective by itself.
Test independent mechanisms in isolated variants. Compare per-thread lock acquisitions, median/p95, throughput, scaling, CPU samples, fragmentation, cache footprint, post-quiescence idle RSS.

## Mandatory design gate BEFORE unsafe TLS code
Formalize byte/extent transitions: global free → reserved → live → pending release → reclaimable → global free; include pending → live same-extent reuse and owner transfer. Define authoritative ownership of every extent, publication and memory order, remote free, thread exit/TLS destructor/reaper, reentrancy/nested collector, destructor panics, failed allocation, interrupted reclamation, exhaustion, alignment/coalescing, double-free/ABA prevention, per-object accounting and reserved slack. No reclaimed chunk reused with live suballocation. Prove synchronization and lifetime for reading/writing headers outside lock. Run deterministic transition/model checks and central review before implementing an unsafe allocator architecture.

## Memory
No eager large per-thread arenas; bounds scale with use, not cumulative threads; reclaim on idle/thread exit. Preserve overall default ≤2% retained and ≤5% peak RSS relative to v2.4 across equal workloads unless approved. Report actual bytes, virtual/committed, high-water, fragmentation and worker scaling.

## Tests / handoff
Deterministic barriers, remote release, transfer, exit/reaper, collector nesting, panic/drop, exhaustion, fragmented churn, thread contention, model checking where available, Miri, platform sanitizers where supported. Return chosen/rejected contracts, proof obligations, exact diff/commit, tests, memory/perf evidence. Do not land an unsafe scheme without closing gate questions.
