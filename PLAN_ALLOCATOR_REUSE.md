# PLAN_ALLOCATOR_REUSE.md — Pending-release recycling and cache policy

## Objective

Fix the structural conflict between deferred batch release and small-block reuse.

The current allocator successfully amortizes teardown but prevents many remove→allocate workloads from immediately reusing the block that was just logically released.

## Implementation scope

Start with:

```text
crates/compact_backend_std/src/cage.rs
```

Inspect these symbols first:

```text
ReleaseExtent
ReleaseCollector
ReleaseBatchScope
ACTIVE_RELEASE_COLLECTOR
CompactRuntime::with_batched_releases
CageAllocation<T>::allocate
release
release_extent
release_many
release_many_locked
allocate_block
size_class_index
Allocator
AllocatorStats
validate_allocator
```

Benchmark telemetry integration:

```text
crates/compact_std/examples/benchmark_compare/main.rs
crates/compact_std/examples/benchmark_compare/measure.rs
```

## Verified behavior

A pending `ReleaseExtent` remains accounted as live until the collector flushes.

That means the global allocator cannot reuse it, but it is also not visible to another thread as free.

This creates an opportunity for safe same-thread cancellation/recycling.

## Required change 1 — pending-release direct reuse

Add a method on `ReleaseCollector` equivalent to:

```rust
fn take_compatible(
    &mut self,
    bytes: usize,
    alignment: usize,
) -> Option<RecycledExtent>
```

Exact naming is flexible.

Start with **exact block reuse only**.

Do not split pending extents in V1 of this optimization.

A candidate extent is reusable only when `block_layout(...)` at that extent's original block start produces exactly the same `block_len`.

Alignment must also be satisfied.

Prefer most-recent matching extent first unless benchmarking proves another policy better.

The collector has at most 64 entries, so bounded linear scan is acceptable initially.

## Allocation path

Before taking the global allocator mutex, `CageAllocation<T>::allocate` should check whether the current thread has an active release collector.

Conceptually:

```text
calculate requested bytes/alignment
        ↓
active collector?
        ↓ yes
find exact compatible pending extent
        ↓ hit
remove extent from pending batch
write fresh AllocationHeader
return new 4-byte owner
        ↓ miss
normal allocator lock / allocate_block
```

Do not expose this mechanism publicly.

## Accounting invariant

A pending release has not yet reduced `allocator.live_bytes`.

Therefore an exact pending-release→new-allocation recycle should not decrement and then increment global live-byte accounting.

The allocation remains continuously accounted as live.

This is a required invariant:

```text
logical old owner dies
pending release recorded
new owner claims exact pending extent
pending release removed

allocator.live_bytes: unchanged
```

## Synchronization invariant

The pending extent:

- is no longer owned by the dropped old object;
- is not yet in any global free structure;
- remains reachable only through the current thread's stack-local `ReleaseCollector`.

Another thread must never be able to allocate it concurrently.

Do not insert a recycled pending extent into the global size-class/general lists first.

## Header reinitialization

The old allocation header may contain the previous:

```text
capacity
initialized
prefix
block_len
```

When recycling:

- recompute `data_offset`;
- recompute `prefix`;
- require compatible exact `block_len`;
- write a completely fresh `AllocationHeader`;
- publish the new owner only after header initialization.

Do not trust old capacity/initialized values.

## Release cancellation correctness

Removing an extent from the collector must guarantee it cannot later be flushed.

Recommended implementation:

```text
swap_remove from collector array
decrement collector.len
```

or equivalent compact removal.

Tests must demonstrate that a recycled extent is not subsequently double-freed when the scope exits.

## Panic behavior

If construction fails before a new owner is returned, the extent must remain either:

1. in the pending release batch; or
2. restored to it.

Never lose an extent between removal and owner construction.

Prefer performing all fallible computations before removing the extent.

Once the extent is removed, subsequent operations should be infallible header writes and owner construction.

## Required change 2 — telemetry that explains misses

Replace aggregate-only interpretation with per-class/miss-reason data.

Add telemetry for at least:

```text
pending_reuse_hits
pending_reuse_misses

per-class global hits:
32
40
112
528

per-class global misses:
32
40
112
528

miss reason:
no active pending match
global class empty
alignment incompatible
requested size has no class
general-list fallback
cursor fallback

released exact-size extents
exact-size extents cached
exact-size extents coalesced before cache
```

Telemetry remains feature-gated and global/benchmark-only.

Do not enlarge retained objects.

## Required change 3 — reassess global size classes

After pending reuse works, benchmark three allocator configurations:

```text
A: current global size classes
B: global size classes disabled
C: pending reuse + current size classes
```

If useful, also evaluate a reduced subset of classes.

Do not retain the current four-class mechanism merely because it already exists.

Keep a class only when it shows measurable benefit on at least one realistic workload without hurting fragmentation or other target workloads.

If global classes remain nearly unused after pending reuse, simplify/remove them.

If they remain:

- preserve exclusivity between global class lists and general free list;
- preserve bounded capacity;
- preserve coalescing correctness.

## B8-specific expected behavior

B8 alternates removals and allocations from a stable population with repeated payload sizes.

Pending direct reuse should therefore be capable of converting patterns like:

```text
release 112
allocate 112
```

into same-thread exact recycling without:

```text
allocator mutex
general free list
global size class
coalescing
```

The plan does not require a specific hit rate, but the new telemetry must explain why any remaining compatible allocation misses.

## Tests

Add deterministic tests for:

- pending exact-size reuse;
- pending reuse with same size but incompatible alignment;
- pending reuse miss on different block length;
- multiple pending extents;
- most-recent matching behavior;
- removal from middle of collector;
- full 64-element collector;
- collector flush after some extents were recycled;
- no double release after reuse;
- nested `with_batched_releases`;
- panic during nested destructor followed by collector flush;
- recycled block receives fresh capacity/initialized header;
- recycled owner drops normally;
- `live_bytes` unchanged during release→reuse cancellation;
- `live_bytes` correct after new owner later drops;
- cross-thread isolation;
- allocator validation while another thread has no access to TLS pending state.

## Property tests

Extend allocator model state to include:

```text
live
pending-release
global-free
size-class-free
```

An extent must belong to exactly one category.

Allowed transitions:

```text
live -> pending
pending -> live       (recycle)
pending -> global-free
global-free -> live
size-class-free -> live
```

Never allow:

```text
pending + global-free simultaneously
pending + live simultaneously
```

except during an internal transition before publication, which must not be externally observable.

## Benchmarks

Primary:

```text
B8
```

Also:

```text
A2
A3
B3
B5
B10
```

Collect:

```text
pending hit rate
per-class hit rate
global allocator locks
free-list visits
release batches
cursor growth
free block count
largest free block
retained memory
```

B10 is mandatory because the new optimization is thread-local and must not introduce global contention.

## Non-goals

Do not add:

- software prefetching;
- new architecture-specific instructions;
- thread-local persistent heap/slab storage;
- extent splitting inside the pending collector;
- cross-thread pending reuse;
- retained object metadata;
- new pointer representations.

## Handoff

Report:

```text
changed files
commit SHA
targeted tests
full tests
Miri results
B8 before/after
pending reuse hit/miss breakdown
global size-class decision
deviations
unresolved assumptions
```
