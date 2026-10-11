# PLAN.md — V2.5 Append-First Allocation and Background Memory Maintenance

## 1. Repository and objective

**Repository:** `Biddlebaddleboo/compact-std-library-rust`

**Branch:** `main`

**Verified baseline:** `ec0dc24d07b0424ae470c0469e33974bc84b907a`

Before implementation, verify the latest remote `main`, compare relevant changes and reconcile the plan where necessary.

### Objective

Experiment with a low-overhead allocation strategy that:

1. Prefers constant-time or bounded-cost foreground allocation.
2. Advances the existing cage cursor rather than searching fragmented free space unnecessarily.
3. Retains fast deterministic reuse for small allocations.
4. Uses a minimum-priority background maintenance worker to prepare already-released memory for future allocations.
5. Minimizes operating-system interactions and unnecessary copying.
6. Guarantees reclamation and exhaustion recovery even when the background worker is disabled or never scheduled.

This is an experimental optimization. Production changes require reproducible performance benefits and complete correctness validation.

Do not implement a tracing garbage collector, relocate live objects, or add a second allocator hierarchy.

## 2. Verified repository facts

The existing allocator is implemented primarily in:

`crates/compact_backend_std/src/cage.rs`

Its important structures are:

- `Allocator`: cursor, live-byte accounting, general free-list head, fixed size-class heads and counts.
- `CageState`: cage memory, allocator mutex, local reuse state, pending releases and allocator fault state.
- `AllocatorTransaction`: authoritative transaction holding the allocator mutex.
- `PendingReleaseQueue`: intrusive in-cage release descriptors protected by a separate mutex.

Important existing functions:

- `allocate_block`
- `allocate_from_cursor`
- `release_many_locked`
- `next_merge_extent`
- `insert_free`
- `merge_free_ranges_locked`
- `drain_pending_releases_locked`
- `lock_for_allocation`
- `CompactRuntime::allocator_stats`
- `CompactRuntime::validate_allocator_state`

### Current allocation behavior

`allocate_block` already has a cursor fast path when the global free structures are empty.

When free extents exist, it searches compatible size-class blocks and then the general free list before falling back to the cursor.

Consequently, fragmented free space can increase foreground allocation work.

The existing local 32-byte reuse optimization must remain intact.

### Existing reclamation behavior

Released extents are synchronously tracked through the shared allocator, release collectors, local caches or the pending-release queue.

`release_many_locked` already has an optimized path for reclaiming contiguous ranges at the cursor.

The new worker must coexist with these mechanisms rather than replacing their correctness responsibilities.

### Constraints

Preserve:

- Four-byte `CageAllocation<T>`.
- Sixteen-byte allocation header.
- Four-byte `CompactVec<T>`.
- Twelve-byte `CompactVecDeque<T>`.
- Existing public APIs.
- Existing ownership and `Drop` semantics.
- One authoritative cage allocator.

## 3. Implementation scope

### Primary write owner

`crates/compact_backend_std/src/cage.rs`

Inspect first:

- `Allocator`
- `CageState`
- `AllocatorTransaction`
- `allocate_block`
- `allocate_from_cursor`
- `block_layout`
- `release_many_locked`
- `next_merge_extent`
- `insert_free`
- `merge_free_ranges_locked`
- `drain_pending_releases_locked`
- `enqueue_pending_release`
- `lock_for_allocation`
- `CageAllocation<T>::allocate`
- `CageAllocation<T>::try_resize`
- `CompactRuntime::init`
- `CompactRuntime::allocator_stats`
- `CompactRuntime::validate_allocator_state`

### Proposed additions

A private allocation-policy selector supporting append-first experiments.

A bounded maintenance-work signal or generation counter.

A private worker lifecycle controller.

A private maintenance operation that processes free-space metadata under authoritative allocator synchronization.

Prefer a small separate module such as:

`crates/compact_backend_std/src/memory_maintenance.rs`

Only add this module if doing so meaningfully reduces complexity in `cage.rs`.

### Read-only dependencies initially

- `crates/compact_backend_std/src/deterministic_memory.rs`
- `crates/compact_backend_std/src/allocator_model.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/deque.rs`
- `crates/compact_collections/src/hash_map.rs`

Modify these only when compilation, testing or correctness requires it.

### Testing and benchmarks

- `crates/compact_backend_std/tests/integration.rs`
- Existing allocator unit tests.
- `crates/compact_std/examples/benchmark_compare/main.rs`
- `crates/compact_std/examples/benchmark_compare/scenarios.rs`
- Existing memory and CPU profiling harnesses.

Proposed result:

`PROFILE_V2_5_BACKGROUND_MAINTENANCE.md`

## 4. Phase 1 — Establish the experimental baseline

Measure the current reuse-first allocator before introducing any new allocation policy.

Capture:

- Allocator lock acquisitions and lock-wait time.
- General free-list visits.
- Size-class cache hits and misses.
- Cursor allocations.
- Bytes moved during buffer replacements.
- Free-space fragmentation.
- Cursor high-water mark.
- Peak and retained RSS.
- Page faults.
- End-to-end latency.

Run all sixteen scenarios.

Pay particular attention to B2, B4, B7 and B10.

Use separate telemetry-enabled builds for diagnostics and telemetry-free builds for performance measurements.

Determine how frequently the allocator currently searches fragmented free space before choosing a cursor allocation.

Do not assume this search is the dominant cost.

## 5. Phase 2 — Append-first allocation

Prototype a deterministic policy that prefers the existing cursor path after inexpensive local reuse attempts.

### Proposed selection order

1. Use existing capacity or in-place expansion when applicable.
2. Attempt the existing profitable exact local reuse path.
3. Attempt a very cheap compatible ready-free-block lookup where available.
4. If sufficient cage space remains, allocate from the cursor.
5. If the cursor cannot satisfy the request, perform synchronous reclamation and shared free-space search.
6. If no valid contiguous region exists, return the existing allocation-exhaustion error.

The exact ordering of steps 3 and 4 is experimental. Compare both.

### Important constraints

Do not create allocation-specific OS mappings.

Do not add new native allocations for ordinary cage memory requests.

Do not change how allocation headers or offsets are represented.

Avoid unbounded searches on the common allocation path.

Keep the previous reuse-first path selectable for A/B testing.

### Fragmentation control

Measure how quickly append-first allocation consumes never-before-used cage addresses.

A larger cursor value is not automatically a memory leak, but rapid cursor growth can reduce available contiguous address space and increase physical page faults.

The allocator must recover previously released space synchronously whenever necessary.

The background worker must never be required to prevent false allocation exhaustion.

## 6. Phase 3 — Background maintenance worker

Implement an optional, low-priority worker that operates on already-released memory.

The worker performs **memory maintenance, not object collection**.

### Permitted responsibilities

- Coalesce adjacent published free extents.
- Reorganize existing shared free-space metadata.
- Prepare reusable free extents for later allocations.
- Maintain useful bounded statistics.
- Optionally prepare larger contiguous free ranges using existing free memory.

### Explicit non-goals

The worker must not:

- Trace references or determine object reachability.
- Relocate live Rust objects.
- Invoke user destructors.
- Perform ordinary application allocations on behalf of foreground callers.
- Own free memory independently of the cage allocator.
- Change public collection semantics.
- Require garbage-collection pauses.
- Run continuously when no useful maintenance is available.

### Minimum-priority scheduling

Investigate platform-supported low-priority scheduling.

On Linux, consider process-relative scheduling niceness for the worker thread.

Do not assume the same scheduling mechanism exists on macOS and Windows.

Provide a portable fallback using ordinary thread scheduling and cooperative, bounded work.

Priority adjustment must be best-effort: failure to lower priority must not cause allocator initialization failure.

Do not busy-spin to simulate low priority.

### Thread lifecycle

The worker must:

- Start only when the feature is enabled and initialization is valid.
- Remain inactive when there is no work.
- Wake on meaningful free-space maintenance work.
- Stop cleanly when requested or during supported shutdown.
- Avoid creating multiple workers accidentally.
- Avoid holding references to deallocated cage memory.
- Avoid self-deadlock during shutdown.
- Not interfere with process termination.

Document how this interacts with the existing process-wide cage lifetime.

## 7. Phase 4 — Free-space maintenance design

Investigate a bounded incremental maintenance approach.

The worker should not perform an unbounded full-cage traversal every time an object is released.

### Suggested model

A release operation makes its extent authoritatively available through existing mechanisms.

It may signal that free-space organization would be worthwhile.

The worker wakes, acquires the appropriate allocator synchronization, processes a bounded amount of maintenance and releases the lock.

If more work remains, it yields and continues later.

### Critical design requirement

Do not create an additional authoritative free-space registry.

Any deferred work indicator must be reconstructible from the shared allocator state after a crash or interruption.

Avoid duplicated ownership metadata that can become inconsistent.

### Concurrency

Define:

- Lock acquisition order.
- Worker and foreground allocation interaction.
- Worker and release-collector interaction.
- Worker and pending-release queue interaction.
- Thread-exit behavior.
- Worker cancellation during maintenance.
- Recovery after panic or poisoned locks.

Never hold the global allocator lock while sleeping or waiting for another thread.

The worker must not retain raw pointers to mutable free-list nodes across unlocked operations unless lifetime and exclusivity are rigorously established.

### Reclamation correctness

Already-released memory must remain reclaimable whether the worker executes zero, one or many times.

A synchronous allocator request must be able to complete necessary free-space reconstruction when the background worker is unavailable.

## 8. Phase 5 — Worker scheduling and contention

Compare several maintenance policies:

**Policy A: Disabled**

No worker. Append-first allocation with synchronous fallback.

**Policy B: Opportunistic**

Wake the worker only after a configurable amount of released memory accumulates.

**Policy C: Pressure-aware**

Activate background organization when fragmentation or remaining cursor space crosses a cheap, deterministic threshold.

All policies must be evaluated using identical workloads.

Do not introduce frequent wakeups merely because a release happened.

Investigate whether maintaining free-space indexes incrementally costs less than background scanning.

Keep scheduling controls private and bounded.

## 9. Phase 6 — Optional physical-page reclamation

This phase is explicitly optional.

The initial experiment should perform no new page-reclamation syscalls.

Only investigate returning unused physical backing if measurements show retained RSS is a meaningful problem.

Any such optimization must:

- Operate exclusively on wholly free pages.
- Preserve cage address stability.
- Respect page boundaries.
- Avoid discarding live or pending-release data.
- Be disabled by default until independently validated.
- Be benchmarked for syscall and page-fault overhead.

Do not add periodic OS calls merely to reduce reported RSS.

## 10. Deterministic correctness tests

Add tests for:

- Append-first allocation with available cursor space.
- Cursor exhaustion with reusable free extents.
- Fragmentation with sufficient total free bytes but no sufficiently large contiguous interval.
- Adjacent free-extent coalescing.
- Exact-size local-cache reuse.
- Concurrent allocations and releases.
- Cross-thread ownership transfer and release.
- Thread exit while work is queued.
- Worker disabled permanently.
- Worker delayed indefinitely.
- Worker cancellation during maintenance.
- Worker shutdown and restart where supported.
- Pending-release queue recovery.
- Lock poisoning and maintenance failure.
- Panicking destructors.
- Repeated allocation/release churn.

Use barriers, fake work triggers and injected maintenance scheduling rather than real sleeps.

Test the same sequence with maintenance enabled and disabled.

Compare resulting allocator states and ensure that both remain correct and reclaimable.

### Concurrency proof obligations

Preserve:

- Unique ownership of every live allocation.
- No overlapping live extents.
- No double publication of released memory.
- Safe offset reuse.
- No use-after-free.
- Valid allocation headers.
- Correct live and pending byte accounting.
- Deterministic reclamation.
- No lost free-space descriptors.

Miri and sequential model tests do not establish complete concurrent safety.

Use concurrency model checking or sanitizer-based testing where practical.

## 11. Performance configurations

Benchmark three primary configurations:

| Configuration | Allocation policy | Maintenance |
|---|---|---|
| Baseline | Existing reuse-first | Disabled |
| Experiment A | Append-first | Disabled |
| Experiment B | Append-first | Low-priority worker |

Additionally, test reuse-first plus maintenance if it is cheap to configure, to isolate the worker's contribution.

### Performance metrics

Measure:

- Median allocation latency.
- p95 and statistically supported p99 latency.
- End-to-end workload time.
- Allocator lock-wait time.
- Number of lock acquisitions.
- Free-list traversal work.
- Cursor growth.
- Fragmentation.
- Peak RSS.
- Retained RSS.
- Page faults.
- Worker CPU time.
- Worker wakeups.
- Worker lock-hold duration.
- Maintenance work completed per wakeup.
- Synchronous fallback frequency.

Evaluate latency in both worker-active and worker-idle states.

The worker must not improve median latency by creating unacceptable tail-latency spikes.

### Hardware

Use the existing two-vCPU AArch64 benchmark machine.

Run one-worker and two-worker B10 scenarios.

Additional four/eight-worker oversubscription tests are optional, not required.

A background maintenance thread must be included in CPU utilization and contention comparisons.

## 12. Acceptance criteria

An experimental implementation may be accepted only when:

1. All correctness and memory-safety tests pass.
2. The shared allocator remains authoritative.
3. The allocator works correctly without background maintenance.
4. The original 32-byte reuse optimization is preserved.
5. The two-worker B10 benefit remains intact.
6. All sixteen benchmark checksums match.
7. No substantial latency regression appears in B2, B4, B7 or other workloads.
8. The worker demonstrates repeatable performance benefit beyond append-first alone.
9. The worker does not cause substantial CPU contention or scheduling interference.
10. The default retained-memory increase stays within 2%.
11. The default peak-RSS increase stays within 5%.
12. Public APIs and compact layouts remain unchanged.

If append-first performs worse than reuse-first, reject it.

If the worker adds more overhead than the maintenance work it saves, omit it.

A successfully implemented worker is not itself evidence of a successful optimization.

## 13. Validation commands

Run:

```bash
cargo fmt --all -- --check

cargo check --workspace --all-features --locked

cargo test --workspace --all-features --locked

cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

cargo run --locked --release -p compact_std \
  --example benchmark_compare \
  --features json,toml -- --self-check
```

Run the full repository Miri and benchmark-harness workflows.

Run supported Apple cross-target compilation checks.

Independently inspect the final source diff, especially synchronization, worker lifetime and allocation-exhaustion recovery.

## 14. Workstream structure

Use one integrated allocator implementation owner.

The append-first policy and maintenance worker both interact with allocator ownership, locks, free-space metadata and recovery.

Do not independently redesign these shared interfaces in parallel branches.

Independent benchmark capture and safety review may proceed without overlapping production writes.

### Integration order

1. Baseline telemetry and benchmarks.
2. Append-first allocation without worker.
3. Correctness and performance evaluation.
4. Only if justified, bounded maintenance worker.
5. Worker-on versus worker-off comparison.
6. Additional pressure-aware scheduling experiments if beneficial.
7. Full correctness and cross-workload validation.
8. Final report and independent diff review.

## 15. Execution handoff

The orchestrator must:

1. Verify latest remote `main`.
2. Read `PLAN.md` first.
3. Inspect the exact starting symbols listed in this plan.
4. Keep experiments reversible and separately measurable.
5. Use isolated development branches where helpful.
6. Document rejected as well as accepted optimizations.
7. Run the full safety, performance and memory validation.
8. Record implementation commits, test results, changed files and unresolved assumptions.
9. Produce `PROFILE_V2_5_BACKGROUND_MAINTENANCE.md`.
10. Independently verify the plan against the final implementation.
11. Remove all temporary `PLAN*.md` files.
12. Commit accepted production changes and the performance report without retaining planning files.

If a fundamental architectural contradiction emerges, resolve it centrally rather than silently relaxing ownership or memory-safety requirements.
