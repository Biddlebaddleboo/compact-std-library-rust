# PLAN_COLLECTION_HOT_PATHS.md — Deque, hash and copyable-vector optimization

## Objective

Target the three largest remaining non-allocator benchmark regressions without changing retained layout or adding software prefetching.

Targets:

```text
A4 deque        ~7.5x native
A5 hash         ~4.8x native
B6 order book  ~15.5x native
```

These are independent from the pending-release allocator redesign.

# Part 1 — A4 steady-state deque

## Implementation scope

Primary file:

```text
crates/compact_collections/src/deque.rs
```

Inspect first:

```text
CompactVecDeque::push_back
CompactVecDeque::push_front
CompactVecDeque::pop_front
CompactVecDeque::pop_back
CompactVecDeque::reserve
CompactVecDeque::physical_index
CageAllocation::uninit_capacity
CageAllocation::uninit_capacity_mut
```

Benchmark:

```text
crates/compact_std/examples/benchmark_compare/scenarios.rs
deque_churn
```

## Verified bottleneck shape

A4:

```text
capacity = 4096
80,000 iterations:
    push_back
    pop_front
```

Capacity remains stable.

Current `push_back` performs a `reserve(1)` capacity check followed by another storage resolution for the actual write.

## Required change

Add a no-growth steady-state fast path.

Conceptually:

```text
if storage exists:
    resolve mutable uninitialized-capacity slice once
    if len < capacity:
        compute back index
        write
        increment len
        return
slow path:
    reserve/grow
    write
```

Do the equivalent for `push_front`.

Do not call `reserve(1)` first when existing storage visibly has room.

The slow growth path remains authoritative when full.

## Pop paths

Profile `pop_front` and `pop_back` after the push optimization.

Only change them if measurements show meaningful remaining overhead.

Potential improvements may include:

- fewer repeated scalar conversions;
- combining wrap/head update with resolved storage lifetime;
- branch simplification.

Do not retain capacity in `CompactVecDeque`; that would make the 12-byte representation larger.

## Tests

Verify:

- empty→first push;
- fill exactly to capacity;
- growth;
- wrap;
- alternating push/pop;
- push_front/pop_back;
- ZST behavior if supported;
- exact drop counts;
- panic-safe nested values;
- iterator equivalence after heavy churn.

# Part 2 — A5 hash probing

## Implementation scope

Primary:

```text
crates/compact_collections/src/hash_map.rs
crates/compact_collections/src/hash_control.rs
```

Relevant symbols:

```text
control_group
first_empty_slot
CompactHashMap::find_slot
CompactHashMap::find_slot_in
CompactHashMap::ensure_insert_capacity
CompactHashMap::rehash
hash_control::classify
```

## Verified current behavior

`control_group` always creates `[u8; 16]` and fills it lane-by-lane using:

```text
control[(start + lane) & mask]
```

even when the 16-byte group does not wrap around the table.

That adds copies and wrapped-index arithmetic before every SIMD classification.

## Required change — contiguous group fast path

For physically contiguous full 16-byte groups:

- classify directly from the control slice or one direct 16-byte load;
- do not construct/copy a temporary group lane-by-lane.

Only use a scratch `[u8; 16]` for the actual wraparound case or partial final group.

Preserve scalar/Miri semantics.

Possible internal API:

```rust
classify_slice_16(&control[cursor..cursor + 16])
```

or an equivalent pointer-based classifier whose safety contract is explicit.

Do not allow an SIMD load to cross slice/table boundaries.

## Hash probing

Preserve:

```text
EMPTY = 0
FULL = 1
TOMBSTONE = 2
```

Preserve first-tombstone behavior exactly.

Optimize only the mechanics of group access/classification unless profiling proves another hotspot.

## Rehash scratch allocation

`rehash` currently allocates a native `Vec<usize>` named `destinations` sized to the old control table.

Investigate eliminating or reducing this temporary native allocation.

Potential options:

- two-pass transfer with no destination vector;
- reuse the new control table to determine target during a non-fallible transfer phase;
- compact temporary index representation if correctness requires scratch.

Do not introduce extra retained cage metadata.

No arbitrary user code may run after movement begins unless unwind correctness is proven.

Keep the existing "all fallible work before destructive transfer" property.

If removing `destinations` materially complicates panic safety or does not benchmark better, leave it unchanged.

## Tests

Run/extend:

- scalar vs SIMD classification differential tests;
- every wrap position;
- all control-state combinations used by property tests;
- collisions;
- tombstone insertion preference;
- remove/reinsert;
- near-full table;
- rehash;
- deterministic custom hashers;
- random operation model vs `std::collections::HashMap`.

# Part 3 — B6 copyable vectors

## Implementation scope

Primary:

```text
crates/compact_collections/src/vec.rs
```

Benchmark:

```text
crates/compact_std/examples/benchmark_compare/scenarios.rs
order_book
compact_retain_levels
```

Relevant current APIs:

```text
CompactVec::as_mut_slice
CompactVec::truncate
CompactVec::try_clone
CompactVec::try_clone_copy
```

## Step 1 — fix benchmark/API mismatch

`PriceLevel: Copy`.

B6 must use:

```rust
try_clone_copy()
```

instead of generic:

```rust
try_clone()
```

for copyable snapshots.

Do this before drawing conclusions about the library's copy performance.

Report both old and new B6 phase timings.

## Step 2 — add real `CompactVec::retain`

Implement a production `retain` API rather than keeping B6's benchmark-only manual retain.

Required semantics should match `Vec::retain`:

```rust
pub fn retain<F>(&mut self, keep: F)
where
    F: FnMut(&T) -> bool
```

or a fallible variant only if the crate's API conventions require it.

For `needs_drop::<T>() == false`:

- resolve the allocation once;
- scan once;
- compact kept values in place;
- publish final initialized length once;
- avoid repeated `Index`/`IndexMut` resolutions.

For destructor-bearing `T`:

- preserve exact-once drop;
- lower logical initialization before any destructor that may panic;
- use a guard so unprocessed/moved values remain valid during unwind.

Do not assume general `CompactValue` is `Copy`.

## Copy specialization

For the non-drop path, use raw moves/copies where semantically valid.

For B6's `PriceLevel: Copy`, the hot path should look much closer to native slice compaction rather than repeatedly resolving `CompactVec` indexing operations.

## Snapshot clone

Keep `try_clone_copy` as the explicit optimized API unless Rust specialization can safely select it automatically without unstable features.

Do not add fragile specialization machinery.

## Tests

Add:

- retain all;
- retain none;
- alternating;
- first/last;
- `Copy` scalar values;
- destructor-bearing values;
- predicate panic;
- destructor panic;
- exact drop counts;
- vector remains valid after unwind;
- compare behavior with native `Vec::retain`.

# Targeted benchmark protocol

Run A4, A5, B6 separately before full suite.

For each record phase-level measurements.

### A4

Separate:

```text
build
mutation
traverse
drop
```

The key metric is steady-state mutation.

### A5

Separate:

```text
build
mutation
lookup_scan
drop
```

If possible add diagnostic counters for:

```text
control groups examined
contiguous groups
wrapped groups
candidate FULL lanes
```

Counters must be benchmark-only.

### B6

Separate:

```text
build
quote_updates_and_snapshot_rebuilds
best_price_and_depth
snapshot_copy
drop
```

The benchmark should identify how much of the previous 15.5x ratio came from:

- benchmark-level generic clone;
- repeated index resolution in manual retain;
- genuine cage overhead.

# Architecture constraints

Do not:

- enlarge deque;
- enlarge vector;
- change hash-map retained control format;
- add fingerprints in this pass;
- add software prefetching;
- introduce a new hasher solely to win A5;
- change observable iteration/order semantics;
- make benchmark-only special APIs.

# Assembly/intrinsics

Retain existing AArch64 NEON and x86-64 SSE2 classifier implementations.

The contiguous-group change should feed those existing implementations more directly.

Do not add prefetch instructions.

Do not add new inline assembly in this pass unless required to preserve existing behavior; record potential asm opportunities separately.

# Validation

Run collection unit/property tests plus:

```text
cargo test -p compact_collections --all-features
cargo test -p compact_std --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
```

Run Miri over the affected collection tests.

Inspect release disassembly for:

```text
A4 push_back/pop_front loop
A5 contiguous control classification
B6 retain/copy loop
```

The goal is to verify removal of repeated resolution/copy overhead, not to force particular instructions.

# Handoff

Report:

```text
changed files
commit SHA
A4 before/after
A5 before/after
B6 before/after
retained-size assertions
tests
Miri
assembly observations
deviations
unresolved assumptions
```
