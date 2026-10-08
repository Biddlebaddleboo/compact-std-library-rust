# PLAN_ALLOCATOR.md — Allocation, Release and Concurrency Performance

## Objective
Reduce A2/B8 allocator overhead and B10 concurrency gap only where low-overhead evidence proves a bottleneck.

## Mandatory prerequisite
Inspect failed GitHub Actions Miri run 37717178933 (intrusive allocator invariants step). Reproduce exact failure against pinned main, diagnose root cause, repair and add deterministic regression. Do not modify other unsafe allocator internals until passing Miri baseline is established. Do not skip or weaken tests.

## Implementation scope
Own `crates/compact_backend_std/src/cage.rs`. Start with CageAllocation<T>::allocate, take_pending_reuse, ReleaseCollector::{take_compatible,flush}, allocate_block, allocate_from_cursor, release_many_locked, AllocatorTransaction, read_header, lock, AllocatorStats. Benchmark references: scenarios.rs::box_objects, cache_churn and B10 concurrent case (read-only shared files).

## Verified evidence
A2 read_header/owner access prominent in samples. B8 active pending-release lookup averages 3.25 candidates and exact reuse often succeeds; millions of free-list visits do NOT by themselves prove time dominance. B10 prior uprobes slowed execution ~25–27x, and lock boundary pairing was invalid.

## Phase 1: A2
Low-overhead attribution of read_header, as_slice, initialization, release, uncontended mutex and cursor fast path. Assess reuse of already-validated ephemeral borrowed views within one operation; never remove global offset validation or change unsafe reconstruction contracts. Only modify after verifying perf and Miri soundness.

## Phase 2: B8
After hash integration, remeasure residual B8 cost separating pending reuse, ordered free-list traversal, cursor path, batching/coalescing, tail contraction and mutex acquisition. Prefer low-overhead sampling; never infer time from raw visit counts. Explore reduced duplicate traversal/better batching only if exclusive cost is measured. Preserve existing pending exact reuse and fast teardown; no unproven new free-list indexing, regions or pools.

## Phase 3: B10
First capture symbolized worker stacks with a low-overhead method and valid lock wait versus lock hold/scheduler preemption attribution. If contention is dominant, assess shorter critical sections or safe batched allocator state mutation. Preserve cross-thread ownership and prompt visibility of global frees. No lock-free allocation/reclamation redesign.

## State invariants and recovery
Each extent is exactly live, pending or globally free. Reused extent gets new header before owner published. Maintain exact live_bytes, uniqueness, alignment, OOM/offset errors, nested release, panic/unwind, destructor boundaries, non-poisoned recovery policy and thread isolation. Do not run user Drop under mutex. No persistent native pointers, layout changes, prefetch, new inline asm.

## Deterministic tests
Single/multithread allocations, contention via barriers not sleeps, nested scopes, destructors with panic, active pending reuse, coalescing, fragmentation, near-full/exhaustion and recovery, no double release, state-model validation. Run Miri plus allocator tests before and after.

## Benchmarks
A2, B8, B10 primary; B3/B5 and A4/A5/B6 sentinels. Compare native/pinned/optimized no-telemetry release medians/p95, allocation/free phases, contention, RSS, high-water, retained bytes and checksums. Keep only robust wins without harmful B10, memory or teardown regressions.

## Handoff
List changed files/symbols, commit SHA, original/final Miri results, deterministic tests, measured outcomes, unsuccessful experiments, possible contention limitations and unresolved assumptions.
