# PLAN_ALLOCATOR.md — B8 allocator residual and B10 concurrency

## Scope
Own crates/compact_backend_std/src/cage.rs and allocator-focused tests. Start read_header, CageAllocation<T>::allocate, take_pending_reuse, ReleaseCollector::{take_compatible,flush}, allocate_block, allocate_from_cursor, release_many_locked, AllocatorTransaction, lock, AllocatorStats. Benchmark scripts/harness and collection source are read-only. Latest 8d11b1c Miri CI passed.

## Evidence and caveats
A2 near-native at 1.04x must not regress. Prior B8 pending reuse averaged 3.25 candidates with high exact-hit rate and logged millions of free-list visits; counts do not establish time dominance. B10 old probes slowed workload ~25–27x, failed correct critical-section pairing, and missed worker symbols; no reliable production lock-share claim exists.

## B8 investigation and gated optimization
Following collection/hash integration, capture low-overhead production-representative B8 residual: pending hit/miss/search, general free-list traversal, release batching/coalescing, cursor tail contraction, lock acquisition/hold, hash-map overhead. Only pursue free-list or release changes with confirmed exclusive cost. Consider minimally-scoped shorter traversal or batching experiments; do not replace allocator with new pools/regions or add per-object metadata without separate design review. Preserve pending exact reuse gains.

## B10 investigation and gated optimization
Obtain symbolized native/compact worker stacks and valid low-overhead lock wait/hold data, separating scheduler preemption from actual critical-section work and atomic costs. Do not infer contention from intrusive uprobe latency bins. If supported, test small critical-section reductions or bounded batch state operations; no lock-free allocation/reclamation redesign. Use deterministic barriers, repeat tests and stress with instrumentation removed for speed comparison.

## A2 guardrail
Measure A2 cursor/owner/header access only to catch regressions; do not complicate fast path for trivial theoretical wins. Explicitly distinguish benchmark CountingAllocator overhead from System allocator itself.

## Invariants/tests
Each extent uniquely live, pending or free. Reinitialize header before reuse; exact live_bytes and alignment, no double release/overlap, correct coalescing/cursor/high-water, exhaustion and rollback, cross-thread visibility, nested release, destructor panic, no user Drop under mutex. Use deterministic single- and multithread state-model tests, near-full allocation/recovery, Miri and benchmark sentinels B3/B5. Preserve frozen size and no persistent native pointers. Report SHA, touched symbols, benchmark changes, validated lock attribution, errors/recovery, RSS and unsuccessful tests.
