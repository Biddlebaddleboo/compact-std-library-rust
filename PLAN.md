# PLAN.md — V2.4 allocator and teardown optimization

## Baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`  
Branch: `main`  
Verified baseline: `1743dfead3e4db1ba82fb374d224644072f7e061`

The retained V2.4 contract remains frozen.

Current optimized V2.4 already preserves:

```text
CageAllocation<T>          4 B
Option<CageAllocation<T>>  4 B
CompactBox<T>              4 B
CompactVec<T>              4 B
AllocationHeader          16 B
FrozenVec<T>               8 B
FrozenString               8 B
FrozenBytes                8 B
```

Do not make any of these larger.

This pass targets allocator and destruction overhead around the existing representation.

Primary objectives:

1. batch release and free-range coalescing;
2. scoped allocator transactions for high-level operations;
3. small-block size-class fast paths in global allocator state;
4. bulk destruction/move specialization based on `needs_drop`;
5. rebenchmark native Rust vs optimized V2.4 and compare against `1743dfe...`.

## Verified current bottleneck

At `1743dfe...`, B8 fixed-population cache churn remains the clearest allocator-side pathology.

Current approximate drop phase:

```text
native:      0.57–0.60 ms
V2.4:       49.5–49.8 ms
```

The benchmark report identifies per-entry cage release as the dominant residual cost.

Current `CageAllocation<T>::drop` eventually calls `release(offset)`, and `release` currently obtains cage state, acquires the allocator mutex, reads the header, derives the block range, calls `insert_free`, scans the ordered free list, coalesces adjacent ranges, scans toward the tail for cursor contraction, and unlocks. Nested containers repeat that once per child allocation.

The memory remains bounded, so this is primarily a throughput problem rather than a correctness or fragmentation-growth problem.

## Hard invariants

Do not change:

- 32-bit cage-relative addressing;
- one process-wide cage;
- allocation header representation;
- retained owner representation;
- retained collection layouts;
- frozen descriptors;
- public safe ownership rules;
- no-retained-native-pointer rule.

Global allocator metadata may grow modestly because it is one process-wide structure rather than per-object retained overhead.

Any new allocator metadata must have a clear upper bound independent of the number of live objects.

# Workstream 1 — batch release and coalescing

Primary file:

`crates/compact_backend_std/src/cage.rs`

Relevant current symbols:

- `release`
- `insert_free`
- `read_header`
- `read_free_node`
- `write_free_node`
- `Allocator`
- `CageAllocation<T>::drop`

## Goal

Permit many cage blocks to be released under one allocator lock and one coalescing/tail-contraction pass.

## Design

Introduce an internal release descriptor containing only temporary stack/native state, for example:

```rust
struct ReleaseExtent {
    start: u32,
    len: u32,
}
```

Do not retain this in cage objects.

Separate release into two conceptual phases:

```text
phase A:
    run destructors
    derive/collect release extents

phase B:
    acquire allocator lock once
    merge all release extents
    update free structure
    contract cursor once
```

No user destructor may execute while the allocator mutex is held.

## Batch API

Add an internal mechanism such as:

```rust
fn release_many(extents: &mut [ReleaseExtent])
```

or a temporary release collector.

Exact API is implementation-dependent.

Requirements:

- accept unsorted release order;
- sort or otherwise merge extents efficiently;
- detect overlap/corruption;
- coalesce released extents with each other;
- coalesce with existing free blocks;
- update `live_bytes` exactly once;
- perform tail contraction once after merge;
- preserve allocator invariants on failure/panic.

Single-owner `release(offset)` remains available and may delegate to the batch primitive for one extent.

## Avoid temporary heap allocation where practical

For common collection teardown sizes, prefer stack-backed small buffers, caller-provided scratch, or chunked batching.

Do not introduce a large native allocation merely to save cage allocator work.

For very large releases, bounded temporary native allocation is acceptable if benchmarked and documented.

# Workstream 2 — scoped allocator transactions

Primary file:

`crates/compact_backend_std/src/cage.rs`

Consumer files:

- `crates/compact_collections/src/hash_map.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/deque.rs`
- any nested collection drop/clear path proven hot

## Goal

Avoid repeatedly reacquiring the same global allocator lock during one high-level operation.

Conceptual structure:

```rust
struct AllocatorTransaction<'a> {
    state: &'a CageState,
    allocator: MutexGuard<'a, Allocator>,
}
```

Exact naming may differ.

The transaction may expose internal operations such as allocation, release, batch release, coalescing, and tail contraction without reacquiring the mutex.

## Safety rule

No arbitrary user code or destructor may execute while a transaction holds the allocator lock.

Therefore high-level teardown should generally follow:

```text
1. logically detach/move values
2. run destructors outside lock
3. collect cage extents
4. open transaction
5. release/coalesce extents
6. close transaction
```

## Initial consumers

Prioritize:

- `CompactHashMap::clear`
- `CompactHashMap::drop`
- hash-map rehash/replacement cleanup
- bulk `CompactVec` replacement paths
- nested cache-like structures represented by B8

Do not force every collection through transaction APIs unless it reduces actual hot-path work.

# Workstream 3 — small-block size-class fast paths

Primary file:

`crates/compact_backend_std/src/cage.rs`

## Goal

Avoid linear first-fit free-list traversal for common small allocations.

Preserve the existing ordered intrusive free list as the authoritative general allocator/fallback.

## Size classes

Measure the current realistic workloads first and choose a small fixed set of classes from actual allocation-size telemetry.

Likely candidates might include 32 B, 64 B, 128 B, 256 B, and 512 B, but do not hard-code these solely from guesswork.

Add benchmark-only telemetry first if needed to determine dominant `block_len` classes.

## Global allocator metadata only

Size-class heads live in `Allocator`, not allocation headers or owners.

Possible shape:

```rust
struct Allocator {
    cursor: u32,
    live_bytes: u32,
    free_head: u32,
    small_free: [u32; N],
}
```

Exact structure may differ.

This is allowed because `Allocator` is one global runtime object.

No per-object memory-density regression is permitted.

## Correctness

The allocator must never have one free block simultaneously reachable from the ordered general free list and a size-class list.

Choose one of:

1. size-class list owns exact-size free blocks and general list owns everything else; or
2. size-class heads are only a cache/index over authoritative free ranges with carefully maintained coherence.

Prefer the simpler invariant.

## Fast allocation

For matching small sizes:

```text
check exact/compatible size-class head
    ↓ hit
unlink O(1)
return block
```

Fallback:

```text
ordered first-fit free list
    ↓
cursor allocation
```

## Free behavior

Batch release should place suitable final extents into size classes where profitable.

Adjacent blocks must still be coalescible.

Do not let size classes permanently prevent neighboring free blocks from merging into larger reusable regions.

A periodic or transaction-end merge back into the general list is acceptable if simpler.

# Workstream 4 — bulk destruction and move specialization

Primary files:

- `crates/compact_backend_std/src/cage.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/hash_map.rs`
- `crates/compact_collections/src/deque.rs`

## needs_drop == false

For trivially destructible values:

```rust
if !core::mem::needs_drop::<T>()
```

avoid per-element destructor loops, repeated initialized-count updates, and per-element move/drop bookkeeping.

Operate on the contiguous range as one logical unit.

Examples:

- truncate
- clear
- move
- rehash transfer
- deque teardown
- vector replacement

## needs_drop == true

For destructor-bearing values:

1. update logical ownership so each value is dropped exactly once;
2. run each destructor;
3. collect nested cage releases where possible;
4. perform allocator release work in bulk afterward.

Panic safety remains mandatory.

## Bulk move

Where source and destination are contiguous and relocation semantics permit, prefer `ptr::copy_nonoverlapping` or equivalent for `Copy` or trivially movable state. Keep element-wise raw `read`/`write` for general `CompactValue` where required.

Do not assume arbitrary `CompactValue` is `Copy`.

## Hash map teardown

`CompactHashMap::clear` and `Drop` are priority consumers.

Current loop moves and drops each FULL pair independently.

Refactor so control scan logically detaches entries, drops values safely, releases nested cage allocations in batches where observable, then releases table storage.

Do not hold allocator mutex across key/value destructors.

# Optional hot-assembly layer

This pass may continue investigating architecture-specific hot kernels, but assembly is secondary to allocator amortization.

ARM64 remains primary.

Potential candidates after profiling:

- batch free-range merge/scanning if LLVM produces poor code;
- fixed-size block copy/move kernels;
- size-class list manipulation only if instruction-level profiling identifies it;
- existing hash mask extraction.

Rules:

1. write ordinary Rust first;
2. use `core::arch` intrinsics where vectorization helps;
3. inspect ARM64 and x86-64 release assembly;
4. add `asm!` only for a clearly measured improvement;
5. no allocator ownership logic inside opaque assembly blocks.

Do not use asm merely because a function is hot.

# Concurrency

The single allocator mutex remains authoritative unless measurements prove it must change.

This plan does not introduce lock-free allocation.

Scoped transactions will increase lock hold duration, so measure single-thread latency, concurrent B10 throughput, and allocator contention.

Do not optimize B8 by making B10 substantially worse.

Batch operations should minimize total lock acquisitions while keeping lock hold periods bounded.

# Failure and panic handling

Batch teardown must remain correct if a destructor panics.

Required invariant:

```text
every initialized value:
    dropped at most once

every cage allocation:
    either remains owned
    or is eventually released exactly once
```

Use guards that can continue cleanup during unwind without double releasing already-processed blocks.

If collecting a complete release batch before destructors cannot preserve panic safety, process values in bounded chunks.

Allocator corruption is never an acceptable performance tradeoff.

# Tests

Add deterministic tests for:

- batch release of adjacent extents;
- batch release of non-adjacent extents;
- unsorted input releases;
- release containing current cursor tail;
- multiple adjacent ranges collapsing to one;
- batch plus existing free-list neighbors;
- size-class hit/miss;
- class block returned and reused;
- class block coalesced into larger range;
- allocator state after repeated class churn;
- no block visible in two free structures;
- panic during nested destructor;
- exact-once release after panic;
- `needs_drop == false` fast path;
- destructor-bearing bulk clear;
- multithreaded allocation/release after batching.

Continue asserting exact V2.4 sizes.

# Property/fuzz tests

Extend allocator fuzzing with operations:

```text
allocate
single release
batch release
size-class allocation
resize
drop all
```

Maintain an independent model.

Assert after every operation:

```text
no live/live overlap
no live/free overlap
free ranges non-overlapping
free + live == high-water prefix
size-class and general-list ownership disjoint
all returned alignments valid
contents of live blocks preserved
```

Add long deterministic churn runs modeled after B8.

# Benchmark plan

Preserve the existing authoritative methodology.

Primary comparison:

```text
native Rust
vs
new optimized V2.4
```

Secondary:

```text
1743dfe optimized V2.4
vs
new allocator-optimized V2.4
```

## Priority scenarios

### B8

Primary target.

Report phases separately:

```text
build
lookup/scan
churn
drop
```

The important metric is whether ~50 ms teardown collapses materially.

Also record:

- allocator lock acquisition count;
- free-list nodes visited;
- batch size;
- batch release count;
- size-class hits/misses;
- high-water cursor;
- free-block count;
- largest free block.

These counters must be benchmark/debug instrumentation only and must not enlarge retained objects.

### B10

Ensure allocator batching does not damage concurrency.

### A2

Useful small-object allocation/free benchmark.

### A3

Checks short strings/bytes and likely size-class benefits.

### A5

Hash collections create/destroy table storage and may benefit from transaction/bulk teardown.

### B1 / B3 / B4 / B5 / B7

Confirm no regression in the realistic workloads that already became competitive.

# Success criteria

No arbitrary percentage is required before measurement.

A successful pass should:

- preserve exact retained representation sizes;
- materially reduce B8 drop time;
- reduce allocator lock acquisition count during large teardown;
- reduce free-list walking for common small blocks;
- preserve bounded fragmentation;
- preserve B5's ~0.71x retained memory ratio;
- not materially regress B3/B5 CPU wins;
- not materially regress B10 concurrent behavior;
- leave all correctness/Miri/property tests green.

If size classes make fragmentation worse enough to erase memory-density benefits, remove or narrow them.

# Documentation

Update:

- `BENCHMARKS.md`
- `ARCHITECTURE.md`
- `SAFETY.md`

Document:

- batch release semantics;
- transaction lock rules;
- size-class invariant;
- panic/destructor rules;
- unchanged retained layouts;
- benchmark before/after results;
- any asm/intrinsics ultimately used.

Do not call this a new representation version.

# Non-goals

This pass does not:

- change owner size;
- change header size;
- change offset width;
- add retained native pointers;
- replace the process-wide cage;
- introduce lock-free allocation;
- redesign hash control bytes;
- alter frozen descriptors;
- implement a new ABI/version;
- optimize solely for synthetic microbenchmarks.

# Implementation order

1. add benchmark/debug allocator counters;
2. implement reusable release-extent derivation;
3. implement batch release under one lock;
4. add safe/panic-correct collection consumers;
5. introduce allocator transaction abstraction;
6. benchmark B8/B10 before size classes;
7. profile actual block-size distribution;
8. add minimal measured size classes;
9. add `needs_drop` bulk destruction/move specialization;
10. inspect generated ARM64/x86-64 code;
11. add intrinsics/asm only if justified;
12. run full validation;
13. run full benchmark suite twice;
14. update documentation;
15. delete `PLAN.md`;
16. commit implementation.

# Final-diff checklist

- [ ] `CageAllocation<T>` still 4 B
- [ ] `CompactVec<T>` still 4 B
- [ ] header still 16 B
- [ ] frozen descriptors still 8 B
- [ ] no retained native pointers
- [ ] no destructor runs under allocator mutex
- [ ] batch release merges adjacent extents
- [ ] one batch does not reacquire lock per extent
- [ ] tail contraction occurs once per batch/transaction
- [ ] size classes are global-only metadata
- [ ] size-class/general free ownership cannot overlap
- [ ] trivial-drop paths avoid element-by-element destruction
- [ ] general destructor paths remain panic-safe
- [ ] allocator fuzz model passes
- [ ] Miri passes
- [ ] B8 teardown explicitly rebenchmarked
- [ ] B10 concurrency explicitly rebenchmarked
- [ ] memory-density ratios checked for regression
- [ ] previous optimized V2.4 baseline preserved in docs
- [ ] `PLAN.md` removed before implementation commit

# Execution handoff

> Implement PLAN.md exactly. Verify latest main first. Preserve the frozen V2.4 retained layouts. Optimize allocator/teardown behavior through batch release, scoped allocator transactions, measured small-block size classes, and `needs_drop`-aware bulk destruction/move paths. Never run user destructors while holding the allocator lock. Benchmark B8 and B10 throughout, rerun the full native-vs-V2.4 suite, preserve memory density, run Miri/property/fuzz validation, delete PLAN.md, and commit.
