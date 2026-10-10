# PLAN_REGION_ALLOCATOR.md — Minimal Shared Cage Manager

## Objective / implementation ownership
Make the existing allocator a small cage region manager: reserve/return regions, capacity limits, global free intervals/coalescing, authoritative stats and safe legacy fallback. Deterministic layer handles frequent small allocation/recycling. Exclusive owner `crates/compact_backend_std/src/cage.rs`. Start `Allocator`, `CageState`, `AllocatorTransaction`, `lock`, `allocate_block`, `allocate_from_cursor`, `insert_free`, `release_many_locked`, `size_class_index`, `release_many`, `CompactRuntime::{used_bytes,remaining_bytes,allocator_stats,validate_allocator_state}`. Read approved deterministic module and contract.

## Verified facts
Allocator maintains cursor, live_bytes, global/free-class links and shared mutex. Alloc/release paths frequently enter this lock, unlike normal borrowed element reads/writes. Size-class caches and batched collector already exist, so avoid merely moving locks into an equivalent hot path.

## Implementation phases
1. Agree region identity, generation, [start,end), alignment, lifecycle, single authoritative ownership and counters. Contract operations proposed: `reserve_region(size,alignment)`, `return_region(region)`, `acquire_global_extent(layout)`, `reclaim_eligible_regions(budget)`. Not existing APIs. Global reserve/return transactional, rollback on error; never return a region with live/pending/in-flight handles.
2. Integrate deterministic local policy with legacy global path as fallback for large/odd sizes, exhausted local budget and incompatible regions. Global lock only for region reserve/return, free-space/capacity coordination and coherent snapshots, not compatible ordinary local allocate/reuse.
3. Remove/move redundant per-object allocator responsibilities only once consumer states and safety tests pass. Avoid duplicated ownership between region manager and local layer; preserve `used_bytes`, `remaining_bytes`, diagnostic stat meanings or clearly document/validate compatibility changes.
4. Foreground has bounded deterministic reclamation and bounded coalescing; if unsuitable extent cannot be recovered within budget, return documented allocation exhaustion rather than unbounded stall. No user code or Drop under locks. Thread exit returns unused regions promptly while still-live remote owners pin their home interval.
5. Quantify region metadata, cache slack, fragmentation, high-water, idle RSS, lock acquisitions/hold, local/remote release latency including p95/p99/p99.9 and A2/B8/B10 wall time, B3/B5/A4/A5/B6 regression sentinels, complete 16-scenario suite.

## Acceptance
Four-byte `CageAllocation<T>`, 16-byte header and raw-offset checks preserved by default. ≤+2% retained, ≤+5% peak RSS unless approved. No leaks after repeated cycles, no stranded remote memory, no unbounded per-object work, valid release failures/recovery and fewer global transactions with meaningful latency/throughput gains. 1/2 threads mandatory on current two-vCPU host; 4/8-worker oversubscription optional. Integrate only after deterministic workstream proof/contract sign-off; report changed symbols, SHA, memory, tests and deviations.
