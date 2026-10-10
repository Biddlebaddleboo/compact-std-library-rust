# PLAN.md — V2.5 Deterministic Memory Architecture

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 030531ae4f6cedc9f5830f8cd4c4c9f051472443

## Objective and verified facts
Make deterministic ownership, individual-object reclamation and bounded local allocation the foundation; shrink the global allocator to cage-region reservation/return, capacity and global free-space coordination. Add one optional *dedicated low-priority OS thread* only for nonessential incremental maintenance. No tracing GC, moving objects or stop-the-world pauses. Existing `CageState` contains `Mutex<Allocator>`; `Allocator` owns cursor, live accounting, global free and size-class lists; `ReleaseCollector` batches releases via TLS; `CageAllocation<T>` is four bytes; `AllocationHeader` sixteen; `CompactVec` four; `CompactVecDeque` twelve. `read_owner_header<T>` is already optimized. `allocator_model.rs` models chunk generations/quarantine only sequentially; B10 global synchronization remains expensive; borrowed slices/views already amortize access validation.

## Invariants
Every live allocation has exactly one authority to release it. Cage intervals must be disjoint and accounted in exactly one state: GlobalFree, ReservedUnused, Live, ReleasePending, LocallyReusable, GloballyReclaimable (plus explicitly accounted Quarantined on unrecoverable failure). Live and pending blocks cannot be reused globally. Destructors run by normal Rust Drop, not the worker. A successful drop/release must commit to reusable storage or retained, bounded authoritative pending state; no lost descriptors. Remote drop after home-thread exit must reclaim independently of thread scheduling. Public used_bytes and stats semantics must remain coherent. Repeated full allocation/drop cycles must actually reuse capacity, not only report zero live bytes. Process termination is handled by OS, not destructor guarantees.

## Performance and memory
No application access path touches maintenance queue; no global STW; no unbounded per-operation scan or maintenance-held global lock. Local compatible allocation/recycling should avoid global lock; shared manager reserved for region acquisition/return/capacity/coalescing and fallback. Foreground synchronous recovery has a defined bounded work budget and must return documented allocation error rather than wait indefinitely. Default baseline budgets: retained +2%, peak RSS +5%, bounded reservations/metadata, stable post-quiescence footprint and no serious fragmentation regression; explicit approval for exceptions. Preserve existing owner/header/deque representations absent separately approved value.

## Authoritative workstreams, ownership and dependencies
- `PLAN_DETERMINISTIC_MEMORY.md`: defines state transitions, release correctness, local reuse and new private `crates/compact_backend_std/src/deterministic_memory.rs` plus model/tests. **Does not write cage.rs**.
- `PLAN_REGION_ALLOCATOR.md`: exclusive owner of `crates/compact_backend_std/src/cage.rs`, integration, runtime public API/stats/legacy fallback; implements centrally approved deterministic interface and shrinks global allocator.
- `PLAN_MAINTENANCE.md`: **dependent on successful synchronous implementation**; owns proposed `crates/compact_backend_std/src/memory_maintenance.rs` and worker tests, not cage.rs. Dedicated OS thread, best-effort lower priority, event driven; must work when worker disabled/never scheduled.
- `PLAN_VALIDATION.md`: profiling, benchmarks and final independent proof; owns reports and standalone scripts. Orchestrator owns shared benchmark scenarios, lib.rs registration, integration interface approvals and any overlapping tests/config.
No overlapping file/symbol writes; isolated worktrees for parallel-safe modeling and baseline measurements. Deterministic and region work are interface-dependent, not independently merged implementations. Worker after both pass.

## Integration order / handoff
1. Verify latest remote main and reconcile relevant changes; pin clean baseline and all-16 checksum/perf/memory.
2. Audit current release errors, descriptors, Drop/collector, owner/offset provenance; establish exact deterministic contract and concurrent model.
3. Central review of interfaces, publication and race proofs, failure semantics; integrate deterministic private layer and region-owner cage.rs changes with legacy fallback.
4. Prove fully synchronous reclamation, cross-thread/thread-exit recovery, memory bounds and correct operation with worker disabled.
5. Compare deterministic-only against baseline on A2/B8/B10 plus 16 workloads; review tail latencies and rollback if safety or memory regressions.
6. Implement dedicated optional maintenance worker after approval; test disabled, idle, active and intentionally unscheduled states. No correctness dependency on worker.
7. Test full workspace, strict Clippy, Miri, differential/concurrent schedules, optional sanitizers, supported Apple cross-target and harness CI.
8. Executors report exact files/symbols, SHA, tests, timings, memory, deviations and assumptions. Central orchestrator resolves contradictions, reviews final diff, deletes **all PLAN*.md** and commits implementation without temporary plans.

Two-vCPU host is sufficient; 4/8 oversubscribed threads only stress, not physical multicore evidence. No new hardware required. Non-goals: tracing GC, GC pauses, live object movement, background-dependent progress, unsafe premature region reuse, wholesale collection rewrite or unbounded caches.
