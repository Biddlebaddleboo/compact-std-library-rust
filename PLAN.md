# PLAN.md — V2.4 recycling and collection hot-path optimization

## Baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`
Branch: `main`
Verified baseline: `8698490347a5e798df4a6a3ab5f516ce3338711a`

Baseline Miri workflow for this exact SHA completed successfully.

This pass follows the allocator/teardown optimization that reduced B8 drop from approximately 50 ms to approximately 3.14 ms.

## Objective

Continue optimizing V2.4 without worsening retained memory density.

This pass has four measured targets:

1. fix the ineffective small-block reuse path exposed by B8;
2. accelerate A4 steady-state deque churn;
3. accelerate A5 hash-map/set probing and mutation;
4. accelerate B6 copyable-vector retain/clone/mutation behavior.

Software prefetching is explicitly deferred.

## Frozen representation contract

Do not change:

```text
CageAllocation<T>          4 B
Option<CageAllocation<T>>  4 B
CompactBox<T>              4 B
CompactVec<T>              4 B
CompactVecDeque<T>        12 B
AllocationHeader          16 B
FrozenVec<T>               8 B
FrozenString               8 B
FrozenBytes                8 B
```

Continue preserving:

- one process-wide cage;
- 32-bit cage-relative ownership;
- no retained native pointers;
- no per-object allocator metadata increase;
- existing frozen-graph safety contract;
- normal Rust borrowing semantics.

Global bounded runtime metadata is allowed where measured.

## Verified repository facts

### Allocator/recycling

`crates/compact_backend_std/src/cage.rs` currently has:

- `ReleaseExtent`
- `ReleaseCollector`
- `ACTIVE_RELEASE_COLLECTOR`
- `CompactRuntime::with_batched_releases`
- `release`
- `release_extent`
- `release_many`
- `release_many_locked`
- `AllocatorTransaction`
- exact global size classes `[32, 40, 112, 528]`
- cache capacity 32 blocks per class.

B8 telemetry at the current baseline recorded:

```text
size-class hits:       8
size-class misses:     486,599
```

B8 payload sizes are:

```text
8, 24, 96, 512, 1024 bytes
```

`CompactBytes` stores up to 20 bytes inline, making the important cage allocations approximately:

```text
24 B payload    ->   40 B block
96 B payload    ->  112 B block
512 B payload   ->  528 B block
1024 B payload  -> 1040 B block
```

Three of the four heap-backed payload sizes therefore already match exact configured size classes.

The poor hit ratio is primarily structural:

- B8 removal defers the freed block into `ReleaseCollector`;
- the following replacement allocation occurs before that extent reaches the allocator;
- when the collector flushes, adjacent ranges are aggressively coalesced;
- exact-sized blocks therefore frequently cease to exist as individual class candidates.

### A4 deque

Current optimized benchmark:

```text
native:  ~0.354 ms
compact: ~2.674 ms
ratio:   ~7.5x
```

`CompactVecDeque::push_back` currently:

1. calls `reserve(1)`;
2. `reserve` reads storage capacity/header;
3. returns when capacity is sufficient;
4. `push_back` resolves storage again through `uninit_capacity_mut`;
5. writes the slot.

A4 holds a fixed capacity of 4096 and performs 80,000 push-back/pop-front pairs, so almost every `reserve(1)` is a predictable no-growth call.

`physical_index` already uses branch-based wrap handling rather than modulo.

### A5 hash map/set

Current optimized benchmark:

```text
native:  ~1.16 ms
compact: ~5.63 ms
ratio:   ~4.8x
```

`find_slot_in` processes 16-byte control groups.

`control_group` currently constructs a temporary `[u8; 16]` and copies each active lane using wrapped indexing even when the requested 16 bytes are physically contiguous.

Existing architecture-specific SIMD classification should remain intact.

### B6 order book

Current optimized benchmark:

```text
native:  ~0.038 ms
compact: ~0.593 ms
ratio:   ~15.5x
```

B6 uses `PriceLevel`, which is `Clone + Copy` and contains only scalar fields.

The benchmark currently implements compact retain manually as:

```rust
fn compact_retain_levels(levels: &mut CompactVec<PriceLevel>)
```

and uses generic `try_clone()` even though `PriceLevel: Copy`.

`CompactVec` already exposes `try_clone_copy`, but B6 does not currently use it.

This pass should distinguish benchmark/API inefficiency from genuine library overhead.

## Planning set

- `PLAN_ALLOCATOR_REUSE.md`
  - pending-release direct recycling;
  - class-specific and miss-reason telemetry;
  - evaluate whether global size-class caches are still justified.

- `PLAN_COLLECTION_HOT_PATHS.md`
  - A4 steady-state deque push/pop optimization;
  - A5 control-group/probe optimization;
  - B6 copyable retain/clone fast paths.

## Workstream independence

### Allocator workstream

Primary write ownership:

```text
crates/compact_backend_std/src/cage.rs
crates/compact_backend_std/Cargo.toml
crates/compact_std/examples/benchmark_compare/main.rs
crates/compact_std/examples/benchmark_compare/measure.rs
```

It may add allocator-specific tests.

It must not redesign collection layouts.

### Collection workstream

Primary write ownership:

```text
crates/compact_collections/src/deque.rs
crates/compact_collections/src/hash_map.rs
crates/compact_collections/src/hash_control.rs
crates/compact_collections/src/vec.rs
crates/compact_collections/tests/cage_collections.rs
crates/compact_std/examples/benchmark_compare/scenarios.rs
```

It treats the allocator API at baseline as read-only.

### Shared/documentation ownership

After both workstreams are integrated, update centrally:

```text
BENCHMARKS.md
ARCHITECTURE.md
SAFETY.md
```

Neither workstream should independently redesign shared architecture documentation.

## Parallel safety

The two workstreams are safe to implement in separate branches/worktrees because their primary production write surfaces do not overlap.

Potential conflict:

- benchmark output structure from allocator telemetry;
- benchmark scenario definitions from collection work.

Resolve by assigning:
- `main.rs` / `measure.rs` to allocator workstream;
- `scenarios.rs` to collection workstream.

## Integration order

1. verify latest `main`;
2. integrate allocator recycling workstream;
3. run allocator tests and B8/A2/A3/B10 targeted benchmarks;
4. integrate collection hot-path workstream;
5. run A4/A5/B6 targeted benchmarks;
6. run complete benchmark suite twice;
7. compare against both:
   - native Rust;
   - `8698490347a5e798df4a6a3ab5f516ce3338711a`;
8. update documentation;
9. run complete validation;
10. delete all `PLAN*.md`;
11. commit implementation.

## Cross-workstream invariants

All implementations must preserve:

- exact retained layout sizes;
- allocator live-byte accounting;
- allocator free/live exclusivity;
- panic-safe destruction;
- no user destructor while allocator mutex is held;
- no persistent native pointer in cage-aware state;
- exact hash-map semantic equivalence;
- exact deque ordering;
- exact vector drop semantics.

## Explicitly deferred

Do not implement in this pass:

- software prefetching;
- `PRFM`;
- x86 `PREFETCH*`;
- speculative cache-line hints;
- thread-local general-purpose arenas;
- lock-free allocation;
- pointer-model redesign;
- per-object cache hints;
- larger retained descriptors.

Existing NEON/SSE2 hash classification stays enabled.

No new assembly is required. If ordinary Rust/intrinsics generate obviously pathological code, record it for a later pass rather than expanding this optimization wave.

## Required benchmark comparison

Targeted benchmark before full-suite execution:

```text
A2 box/object allocation
A3 string/bytes
A4 deque churn
A5 hash map/set churn
B6 order book
B8 cache churn
B10 concurrent worker state
```

Then run all 16 benchmark scenarios twice.

For every changed scenario report:

```text
native median/p95
8698490 baseline compact median/p95
new compact median/p95
old/new compact ratio
compact/native ratio
retained-memory ratio
```

## Success criteria

No fixed speedup is mandatory, but each optimization must justify its complexity.

Required:

- B8 pending reuse is actually measured;
- allocator miss telemetry becomes explainable rather than one aggregate number;
- no regression to B8's ~3.14 ms teardown improvement;
- no material B10 concurrency regression;
- A4 materially improves or the speculative change is removed;
- A5 materially improves or the speculative change is removed;
- B6 uses the best library path for `Copy` data before judging the library architecture;
- memory ratios do not worsen;
- exact object sizes remain frozen.

## Validation

Run:

```text
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
```

Run the repository's complete Miri suite.

Run deterministic allocator/property tests with the new pending-reuse state included.

Inspect optimized AArch64 and x86-64 assembly for changed hot kernels, but do not introduce prefetch instructions.

## Final-diff checklist

- [ ] owners remain 4 B
- [ ] deque remains 12 B
- [ ] allocation header remains 16 B
- [ ] frozen descriptors remain 8 B
- [ ] no software prefetching
- [ ] no new retained pointer
- [ ] pending-release reuse is exact-once
- [ ] cancelled releases cannot later be flushed
- [ ] allocator `live_bytes` remains exact
- [ ] no other thread can observe a pending extent as free
- [ ] panic/unwind paths restore TLS collector correctly
- [ ] size-class policy backed by per-class measurements
- [ ] deque behavior unchanged
- [ ] hash collision/tombstone behavior unchanged
- [ ] vector retain/drop behavior unchanged
- [ ] B8 teardown remains fast
- [ ] B10 checked for contention regression
- [ ] all benchmark checksums match
- [ ] Miri passes
- [ ] all `PLAN*.md` deleted before implementation commit

## Execution handoff

> Verify latest main and reconcile relevant changes. Read PLAN.md first. Assign `PLAN_ALLOCATOR_REUSE.md` and `PLAN_COLLECTION_HOT_PATHS.md` to bounded executors in isolated worktrees. Integrate allocator work first, then collection hot paths. Preserve the frozen V2.4 retained layouts. Do not add software prefetching. Require measured benefit for every optimization. Run targeted benchmarks, two complete benchmark suites, correctness/property/Miri validation, independently review the final diff against this plan, delete all PLAN*.md files, and commit without planning files remaining.
