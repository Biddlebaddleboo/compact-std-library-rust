# PLAN.md — V2.4 hot-path optimization and rebenchmark

## Baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`  
Branch: `main`  
Verified baseline: `585f73e1abb02b7818719c4c622720c2a6af512a`  
Version: `2.4.0`

The V2.4 retained representation remains frozen.

This task is an explicitly approved **implementation-performance optimization pass** within the frozen V2.4 representation.

Do not change:

- 32-bit cage addressing;
- 4-byte `CageAllocation<T>`;
- 4-byte `CompactVec<T>`;
- 16-byte allocation header;
- 8-byte frozen descriptors;
- intrusive allocator architecture;
- public cage ownership model;
- serialized/frozen representation;
- public safety invariants.

The optimization target is to reduce repeated work around the existing representation.

Primary goals, in order:

1. hoist repeated cage/header resolution out of hot loops;
2. add bulk collection construction/mutation operations so one logical operation does not devolve into N full `push` paths;
3. make iterators and traversals operate on resolved native views/pointers after one safety proof;
4. add ARM64 NEON and x86-64 SIMD fast paths for hash-table control-byte probing, with a portable scalar reference implementation;
5. rerun the complete native-Rust vs V2.4 benchmark suite and publish before/after results.

The primary benchmark remains:

```text
native Rust
vs
optimized V2.4
```

The previous published V2.4 numbers may additionally be shown as an optimization delta, but they are not the primary baseline.

---

# Verified repository facts

At baseline `585f73e...`:

## Cage access

`crates/compact_backend_std/src/cage.rs`

Relevant symbols:

- `CageAllocation<T>::len`
- `CageAllocation<T>::capacity`
- `CageAllocation<T>::as_slice`
- `CageAllocation<T>::as_mut_slice`
- `CageAllocation<T>::get`
- `CageAllocation<T>::get_mut`
- `CageAllocation<T>::push`
- `CageAllocation<T>::extend_copy`
- `CageAllocation<T>::uninit_capacity`
- `CageAllocation<T>::uninit_capacity_mut`
- `CageAllocation<T>::header`
- `CageAllocation<T>::data_ptr`
- `ptr_from_offset`
- `read_header`

`len()` and `capacity()` obtain allocation-header state.

`as_slice()` resolves the owner to a native pointer and obtains the initialized length.

Repeated high-level calls can therefore repeatedly:

- resolve process cage state;
- resolve owner offset;
- read allocation header;
- construct a temporary slice.

The representation must not be changed to cache a native pointer.

## CompactVec

`crates/compact_collections/src/vec.rs`

Relevant symbols:

- `CompactVec::reserve`
- `CompactVec::push`
- `CompactVec::as_slice`
- `CompactVec::as_mut_slice`
- `CompactVec::iter`
- `CompactVec::try_extend`
- `CompactVec::try_clone`

Current `try_extend`:

1. reads iterator size hint;
2. reserves the lower bound;
3. loops over the iterator;
4. calls `self.push(value)` for every element.

Each `push` calls `reserve(1)` and then `CageAllocation::push`.

This creates avoidable repeated header/capacity work for bulk construction.

`try_clone` similarly loops through values and calls `push`.

## CompactVecDeque

`crates/compact_collections/src/deque.rs`

Relevant symbols:

- `CompactVecDeque::physical`
- `value_at`
- `value_at_mut`
- `get`
- `get_mut`
- `push_back`
- `push_front`
- `pop_front`
- `pop_back`
- `iter`
- `reserve`
- `make_contiguous`
- `CompactVecDequeIter::next`
- `CompactVecDequeIter::next_back`

`physical()` performs modulo using `self.capacity()`.

`capacity()` reaches the cage allocation header.

`CompactVecDequeIter::next` calls `deque.get(index)`, which:

- checks logical bounds;
- calls `physical()`;
- resolves storage again.

The iterator therefore pays general random-access costs on every element.

## CompactHashMap

`crates/compact_collections/src/hash_map.rs`

Relevant symbols:

- `CompactHashMap::find_slot`
- `CompactHashMap::rehash`
- `CompactHashMap::iter`
- `CompactHashMap::iter_mut`
- `CompactHashMap::retain`
- `empty_table`

Current control values:

```rust
const EMPTY: u8 = 0;
const FULL: u8 = 1;
const TOMBSTONE: u8 = 2;
```

`find_slot` performs scalar linear probing:

```text
for each slot:
    read one control byte
    branch EMPTY/TOMBSTONE/FULL
    potentially inspect key
```

There is no fingerprint byte; SIMD can therefore accelerate classification of control states, especially locating EMPTY/FULL/TOMBSTONE groups, but must not invent a fingerprint representation because that would change the table format.

Current `rehash` also scans controls and searches destination controls scalar slot by slot.

## Frozen graphs

`crates/compact_frozen/src/storage.rs`

Relevant symbols:

- `FrozenGraph::root`
- `FrozenGraph::slice`
- `FrozenGraph::str`
- `FrozenGraph::bytes`
- `FrozenGraph::reference`
- `FrozenGraph::check_descriptor`
- `FrozenGraph::check_range`
- `FrozenGraph::byte_ptr`

Safe access validates that the descriptor itself lies inside its owning graph and validates target bounds/alignment.

This safety model must remain unchanged.

Hot repeated traversal should avoid redoing graph-level work unnecessarily after a descriptor has been safely validated for the duration of one borrow.

## Benchmarks

Authoritative harness:

```text
crates/compact_std/examples/benchmark_compare/
    main.rs
    measure.rs
    models.rs
    datasets.rs
    scenarios.rs
```

Published baseline is in `BENCHMARKS.md`.

Key slow cases include:

```text
A1 CompactVec                       ~58.8x native
A4 VecDeque                        ~12.5x
A5 HashMap/HashSet                  ~7.8–9.3x
B6 order-book update/copy          ~27x
B8 cache churn                      ~7.5x
B9 frozen sequential traversal     ~13.3x
```

Memory wins in B1/B2/B3/B4/B5/B9 must not be sacrificed merely to improve speed.

---

# Hard invariants

## Representation

This optimization pass must preserve these exact hard targets:

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

Do not add cached:

- native pointers;
- lengths;
- capacities;
- allocator pointers;
- architecture tags;
- iterator state to retained collection objects.

Temporary stack-only views may contain native pointers.

## Safety

The safe public API remains safe.

All pointer-hoisting must be lifetime-bound to an existing owner/collection borrow.

No native pointer may survive:

- owner relocation;
- replacement allocation;
- mutation that may reallocate;
- owner drop.

Optimized code must remain valid under Miri using portable paths.

Architecture-specific SIMD code must have a scalar reference implementation.

## Architecture support

Primary optimized architecture:

```text
aarch64
```

Required parity target:

```text
x86_64
```

Portable scalar fallback remains available.

For AArch64, basic NEON may be used without runtime detection where guaranteed by the target architecture.

For x86-64:

- provide a baseline SIMD path using architecture-guaranteed or appropriately detected features;
- optionally add AVX2 only if measured worthwhile;
- do not require AVX2 for correctness.

Prefer `core::arch` intrinsics.

Use `asm!` only where:

- generated code from intrinsics is materially worse; or
- inline assembly is clearly simpler and equally maintainable.

Any asm must be tiny, independently differential-tested, and isolated from ownership/lifetime logic.

---

# Phase 1 — hoist cage/header resolution

Primary file:

`crates/compact_backend_std/src/cage.rs`

## Objective

Create internal borrowed-view primitives that resolve an allocation once and expose the data/header state for the duration of an ordinary Rust borrow.

Do not change `CageAllocation<T>` representation.

A possible internal design is:

```rust
pub(crate) struct CageSliceRef<'a, T> {
    ptr: *const T,
    len: usize,
    capacity: usize,
    marker: PhantomData<&'a [T]>,
}

pub(crate) struct CageSliceMut<'a, T> {
    ptr: *mut T,
    len: usize,
    capacity: usize,
    marker: PhantomData<&'a mut [T]>,
}
```

Exact names/design may differ.

Prefer returning ordinary slices plus scalar metadata where that is simpler.

Required behavior:

```text
owner borrow
    ↓
read header once
resolve offset once
    ↓
temporary native pointer/slice + len/capacity
    ↓
perform complete hot operation
```

Do not store these views in retained cage values.

## Exact hot paths to review

In `CageAllocation<T>`:

- `len`
- `capacity`
- `as_slice`
- `as_mut_slice`
- `get`
- `get_mut`
- `push`
- `extend_copy`
- `move_into`
- `try_resize`
- `uninit_capacity`
- `uninit_capacity_mut`

Avoid chains such as:

```text
as_slice()
    -> len()
       -> header()
    -> data_ptr()
```

when a single operation can load the header and pointer once.

Where appropriate implement private helpers such as:

```rust
fn resolved(&self) -> Result<ResolvedAllocation<'_, T>>;
fn resolved_mut(&mut self) -> Result<ResolvedAllocationMut<'_, T>>;
```

or equivalent.

Do not make ordinary infallible collection reads start returning `Result`.

Internal helpers for known-live owners may use the owner invariant and debug assertions rather than repeatedly performing defensive validation intended for arbitrary offsets.

## Release behavior

Do not weaken validation of public unsafe arbitrary-offset APIs.

Optimization applies to valid private owner paths.

---

# Phase 2 — bulk collection operations

Primary files:

- `crates/compact_backend_std/src/cage.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/string.rs`
- `crates/compact_collections/src/compact_bytes.rs`
- parser/build paths in `compact_serde` where applicable

## CageAllocation bulk append

Add an internal/public-safe bulk primitive for initialized append.

For `T: Copy`, preserve/use `extend_copy` but optimize it to:

1. resolve header once;
2. verify available capacity once;
3. resolve destination pointer once;
4. copy entire range;
5. update initialized length once.

For general `T`, add a panic-safe iterator/slice construction primitive if useful.

Potential form:

```rust
fn try_extend<I: IntoIterator<Item = T>>(...)
```

or a private destination writer.

Requirements:

- reserve/grow before moving elements;
- no repeated `set_initialized` if the whole operation cannot panic after ownership transfer, or use an initialization guard when it can;
- panic must never leak or double-drop initialized values.

## CompactVec::try_extend

Rewrite `CompactVec::try_extend` so it does not call full `push()` for every item.

For exact-size iterators:

```text
determine count
reserve once
resolve destination once
write contiguous elements
publish initialized count safely
```

For non-exact iterators:

- use size hint;
- fill available capacity through a resolved batch;
- grow in amortized chunks;
- do not call `reserve(1)` per item.

Implement a robust initialization guard so panic or iterator failure leaves the vector with the exact initialized prefix.

## CompactVec::try_clone

For `T: Copy`, use bulk copy where trait constraints permit.

For `T: Clone`, reserve exactly once and clone into contiguous uninitialized destination while tracking initialized count with a guard.

Do not repeatedly call `push`.

## String and bytes

Audit:

- `CompactString::push_str`
- `CompactBytes::extend_from_slice`
- constructors from slices/strings

Ensure byte bulk operations resolve once and update length once.

Do not change SSO representation.

## Serde

Where direct deserializers currently append one field/element at a time through the slow generic path, consume the improved bulk/vector writer.

Do not redesign Serde architecture.

---

# Phase 3 — direct resolved iterators and traversal

## CompactVec

`CompactVec::iter` already returns `slice::Iter` over `as_slice()`.

After Phase 1, ensure `as_slice()` is one header read + one pointer resolution and does not redundantly re-read state.

Do not replace `slice::Iter` with a custom iterator unless measurements prove necessary.

## CompactVecDeque

Primary file:

`crates/compact_collections/src/deque.rs`

Replace per-element `get()` iteration.

Current:

```text
iterator.next
    -> deque.get(logical)
    -> physical(logical)
    -> capacity()
    -> modulo
    -> storage resolution
```

Target:

At iterator construction, resolve storage once and capture:

```text
base pointer
capacity
head
len
front logical position
back logical position
```

Then `next()`/`next_back()` should:

- calculate physical index using cached capacity;
- preferably increment/decrement and branch-wrap;
- dereference the already-resolved temporary pointer;
- perform no cage header lookup.

Avoid integer division/modulo in the per-element loop.

Preferred increment:

```text
index += 1
if index == capacity:
    index = 0
```

Implement double-ended traversal correctly across wrapping.

Alternative design:

represent the deque as two temporary contiguous slices and chain them.

Use whichever implementation gives:

- simpler safety reasoning;
- fewer branches;
- good optimizer output.

Update `front`, `back`, `get`, `get_mut`, `reserve`, and movement loops to cache capacity/pointer locally where hot.

Do not add capacity to the retained deque representation.

## Hash-map iteration

Current `CompactHashMapIter` already captures native `control` and `entries` slices, which is good.

Preserve this model.

Optimize `retain`, `clear`, `rehash`, and other loops to acquire control/entry views once instead of repeatedly calling:

```text
control.as_slice()
control.as_mut_slice()
entries.as_mut_slice()
```

inside each iteration.

## Frozen graph validated views

Primary file:

`crates/compact_frozen/src/storage.rs`

Do not weaken graph-identity safety.

Add temporary validated view helpers for repeated traversal, for example:

```rust
pub struct FrozenSliceView<'g, T> {
    slice: &'g [T],
}
```

or simply ensure `graph.slice(&descriptor)` resolves once and callers can hold the returned `&[T]`.

Audit B9 benchmark/application paths to ensure they do not repeatedly call:

```text
graph.slice(...)
graph.str(...)
```

inside element loops when the descriptor can safely be resolved once outside the loop.

For nested frozen records where each record contains descriptors, consider an internal traversal helper that validates the outer range once while still validating each nested descriptor exactly when required by safety.

Do not permit copied external descriptors.

---

# Phase 4 — SIMD hash control scanning

Primary file:

`crates/compact_collections/src/hash_map.rs`

Proposed supporting module:

```text
crates/compact_collections/src/hash_control.rs
```

or:

```text
crates/compact_collections/src/arch/hash_control.rs
```

Use whichever keeps `hash_map.rs` readable.

## Important representation constraint

Do **not** change:

```rust
EMPTY = 0
FULL = 1
TOMBSTONE = 2
```

Do not introduce SwissTable-style hash fingerprints in V2.4.

The SIMD optimization works over the existing control bytes.

## Portable scalar reference

Implement one small primitive with a clear semantic contract.

Example conceptual result:

```rust
struct ControlGroupMask {
    empty: u32,
    full: u32,
    tombstone: u32,
}
```

for a fixed-width control group.

Possible primitive:

```rust
fn classify_control_group(ptr: *const u8) -> ControlGroupMask
```

or safe equivalent.

The scalar implementation is authoritative.

It must support partial/end groups safely without OOB reads.

## AArch64

Implement NEON using `core::arch::aarch64`.

Initial width:

```text
16 control bytes
```

Conceptual operations:

```text
vld1q_u8
vceqq_u8(control, EMPTY)
vceqq_u8(control, FULL)
vceqq_u8(control, TOMBSTONE)
convert vector comparison results to compact bit masks
```

Select the simplest reliable mask-extraction implementation.

If mask extraction through intrinsics produces poor compiler output, inspect disassembly and consider a tiny `asm!` helper.

Do not use assembly for the probe loop or key equality itself unless separately justified.

## x86-64

Provide equivalent fixed-group classification.

Baseline should work on ordinary x86-64.

Use appropriate SSE2/SSE-family intrinsics.

Potential AVX2 specialization may classify 32 bytes at once if:

- runtime/static target detection is correct;
- it demonstrates meaningful additional improvement;
- it does not complicate the core algorithm.

AVX2 is optional.

## Probing algorithm

Rewrite `find_slot` to process control groups rather than one control byte at a time.

Preserve linear-probing semantics and first-tombstone behavior.

Conceptually:

```text
start at hashed slot

for each group:
    classify controls
    inspect FULL candidates in probe order
    remember first TOMBSTONE in probe order
    if EMPTY appears:
        stop at earliest EMPTY after checking earlier FULL candidates
```

Because probing may start in the middle of a group and wrap around, mask ordering must exactly preserve current scalar probe order.

Do not simply inspect aligned groups out of logical sequence.

A simple safe approach is acceptable:

- classify 16-byte windows beginning at current probe position using a wrapped temporary group;
- or process first partial segment, aligned full groups, then wrap.

Prefer avoiding temporary allocation.

## Rehash

Use the same group-scanning primitive when searching for empty destination slots in `rehash`.

## Iteration

SIMD scanning of FULL control bytes may also improve:

- `iter`
- `iter_mut`
- `clear`
- `retain`

only if the implementation remains simpler than scalar iteration.

Do not expand scope unless profiling shows these scans materially contribute.

## Differential testing

For every possible control byte value pattern generated by tests, compare:

```text
scalar
aarch64 optimized
x86_64 optimized
```

where architecture permits.

Random/property cases must include:

- all EMPTY;
- all FULL;
- all TOMBSTONE;
- alternating states;
- first match at every lane;
- wraparound start positions;
- tables smaller than group width;
- exactly group width;
- non-multiple group sizes where reachable;
- high-load maps;
- tombstone-heavy maps;
- adversarial constant hasher collisions.

Optimized results must match scalar slot choice exactly.

---

# ARM64-first optimization policy

The user's primary machine architecture is ARM64.

Treat AArch64 as the first performance target.

Implementation order:

1. scalar reference;
2. AArch64 NEON;
3. benchmark/disassemble;
4. x86-64 equivalent;
5. differential tests;
6. optional tiny `asm!` only if justified.

Do not delay the x86-64 implementation indefinitely; the final merged change must support both.

Use compile-time target selection where possible.

Do not add third-party SIMD crates unless they significantly simplify correctness and have negligible dependency cost; std/core intrinsics are preferred.

---

# Assembly rule

Inline assembly is permitted only for a narrowly proven hot primitive.

Before using `asm!`:

1. implement the intrinsic version;
2. compile release code for ARM64/x86-64;
3. inspect emitted instructions;
4. benchmark the primitive;
5. use asm only if it materially improves speed or materially reduces complexity.

If asm is used:

- keep each block small;
- provide complete SAFETY comments;
- state register/clobber assumptions;
- no calls into user code;
- no persistent pointers;
- no allocation;
- no ownership manipulation;
- no architecture-dependent retained representation;
- differential-test against scalar and intrinsic implementations.

---

# Benchmark instrumentation

Use the existing authoritative:

```text
benchmark_compare
```

Do not replace its methodology.

## Before optimization

Record the current published baseline from:

```text
585f73e1abb02b7818719c4c622720c2a6af512a
```

and preserve the currently published numbers.

## After each phase

During implementation, run targeted scenarios:

### Phase 1 / resolution

```text
A1
A4
A6
B2 read phase
B7 query phase
B9 traversal
```

### Phase 2 / bulk operations

```text
A1
A3
B1 build
B3 build
B5 build
B6 snapshot copy
B7 build/update
```

### Phase 3 / direct iterators

```text
A1 traversal
A4
B4 retained scan
B7 scan
B9 sequential/random/parallel traversal
```

### Phase 4 / hash SIMD

```text
A5
B8 lookup/churn
any hash-heavy B3/B5 phases
```

B6 is vector-heavy rather than primarily hash-control-heavy; use it mainly to assess bulk clone/retain improvements.

## Final suite

Run the exact full native-vs-V2.4 benchmark suite at least twice on the same stable ARM64 machine/toolchain.

Primary table:

```text
native
vs
optimized V2.4
```

Secondary optimization table:

```text
published V2.4 baseline
vs
optimized V2.4
```

The secondary table must clearly state that it measures implementation optimization, not the native comparison.

---

# Benchmark success interpretation

Do not require every scenario to improve.

The key question is whether the optimized implementation reduces large avoidable CPU gaps while preserving memory behavior.

Particularly important targets:

## A1 vector

Current ~59x gap is dominated by tiny absolute native time, but should improve substantially from bulk construction.

Do not sacrifice representation to chase parity.

## A4 deque

Current ~12.5x gap is a strong target for cached-capacity/direct-pointer iteration and branch wrapping.

## A5 hash

Current ~8–9x gap is the primary SIMD target.

## B5 mobility

Current:

```text
~1.07x CPU
0.71x memory
```

Do not regress this excellent real-world tradeoff.

## B8 churn

The four requested optimizations may improve lookup/mutation, but the ~50 ms nested teardown problem is not the primary scope of this pass.

Do not introduce batch allocator release in this task.

Document residual drop cost.

## B9 frozen

Current construction is excellent but repeated traversal is slow.

Improve only through view/resolution hoisting.

Do not weaken descriptor identity validation.

---

# Testing

## Existing validation

Must remain green:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Miri:

```sh
cargo +nightly miri test -p compact_core
cargo +nightly miri test -p compact_backend_std --test integration
cargo +nightly miri test -p compact_backend_std --lib
cargo +nightly miri test -p compact_collections --test cage_collections
cargo +nightly miri test -p compact_std --all-features --test v2_4
```

Miri should exercise portable scalar paths, not architecture intrinsics unsupported by Miri.

## New deterministic tests

Add tests for:

### resolved views

- returned slice lifetime tied to owner;
- mutable view exclusive;
- empty/ZST cases;
- high alignment;
- no stale view retained across resize in safe API.

### bulk vector construction

- exact-size iterator;
- inaccurate lower bound;
- zero lower bound;
- panic in iterator;
- panic in `Clone`;
- allocation failure;
- initialized-prefix cleanup;
- non-`Copy` drop exactness;
- ZST;
- `Copy` bulk path.

### deque iterator

- no wrap;
- wrapped head;
- wrap exactly at capacity;
- one element;
- full buffer;
- alternating front/back iteration;
- `rev`;
- `ExactSizeIterator`;
- mutation remains prohibited while iterator exists.

### hash SIMD

- scalar/optimized identical masks;
- scalar/optimized identical `find_slot`;
- tombstone precedence;
- collision chains;
- wraparound;
- table growth/rehash;
- remove/reinsert;
- constant hasher property tests.

### frozen views

- same graph accepted;
- copied descriptor still rejected where current API requires;
- descriptor from different graph rejected;
- nested descriptors;
- repeated validated traversal;
- parallel immutable reads.

---

# Property and fuzz tests

Extend existing hash collision fuzzing to compare optimized probing against a scalar reference.

Do not delete scalar reference code after optimized implementation is complete.

For hash probing fuzz operations, assert:

```text
same found/not-found
same selected insertion slot
same first tombstone
same key/value result
same final logical map
```

Continue existing allocator/property fuzzing unchanged unless compilation requires adaptation.

---

# Performance inspection

For the four optimized areas, inspect optimized compiler output.

On ARM64, inspect at least:

```text
hash control classifier
CompactVec bulk append
deque iterator next
frozen slice traversal helper
```

Confirm hot loops do not repeatedly call:

```text
OnceLock::get
header()
capacity()
ptr_from_offset
integer division/modulo
```

where the relevant state can be legally hoisted.

On x86-64 inspect hash SIMD code to verify vector compares are actually emitted.

Do not make benchmark claims based solely on expected compiler behavior.

---

# Documentation

Update `BENCHMARKS.md` after final measurements.

Preserve the original published benchmark section or clearly identify it as:

```text
V2.4 pre-optimization baseline
```

Add a new:

```text
V2.4 optimized hot-path results
```

section.

Document:

- exact optimization commit;
- exact machine/toolchain;
- native vs optimized V2.4;
- old V2.4 vs optimized V2.4;
- ARM64 SIMD implementation used;
- x86-64 support and whether benchmarked;
- whether inline asm was ultimately necessary;
- unchanged retained layouts;
- remaining major bottlenecks such as nested deallocation if still present.

Update `ARCHITECTURE.md` only enough to state:

- retained architecture remains frozen;
- implementation has architecture-specific optional hot paths;
- scalar implementation remains semantic reference.

Update `SAFETY.md` for any new raw-pointer iterator/view or SIMD unsafe block.

Do not call this V2.5 unless separately requested.

---

# Non-goals

This pass does not:

- change retained layouts;
- change allocation header;
- change allocator strategy;
- add thread-local allocator caches;
- add segregated free lists;
- implement batch release;
- change hash-table control-byte representation;
- add hash fingerprints;
- replace SipHash solely for speed;
- redesign frozen graph identity;
- change cage size/addressing;
- implement V3;
- optimize every benchmark loss;
- use assembly outside demonstrated hot primitives.

---

# Expected write scope

Primary:

```text
crates/compact_backend_std/src/cage.rs
crates/compact_collections/src/vec.rs
crates/compact_collections/src/deque.rs
crates/compact_collections/src/hash_map.rs
crates/compact_frozen/src/storage.rs
crates/compact_collections/tests/cage_collections.rs
crates/compact_std/examples/benchmark_compare/*
BENCHMARKS.md
SAFETY.md
ARCHITECTURE.md
```

Proposed addition if useful:

```text
crates/compact_collections/src/hash_control.rs
```

or small architecture submodules.

Secondary only if required by actual call sites:

```text
crates/compact_collections/src/string.rs
crates/compact_collections/src/compact_bytes.rs
crates/compact_serde/src/*
```

Do not broaden further unless compilation/tests or verified hot call sites require it.

---

# Integration order

Implement in this order:

```text
1. resolved cage access primitives
        ↓
2. CompactVec/bulk construction
        ↓
3. direct deque/hash/frozen traversal
        ↓
4. scalar hash control-group abstraction
        ↓
5. ARM64 NEON path
        ↓
6. x86-64 SIMD path
        ↓
7. complete tests/Miri
        ↓
8. complete benchmark rerun
```

Do not begin architecture-specific hash code before the scalar group API and differential tests exist.

---

# Acceptance criteria

The pass is complete when:

- V2.4 retained representation sizes are unchanged;
- ordinary API compatibility is preserved;
- `CompactVec::try_extend` no longer performs a full `push/reserve` cycle per known-bulk element;
- direct collection iteration does not repeatedly resolve cage/header state per element where avoidable;
- deque iteration does not perform integer modulo/header lookup per element;
- hash probing has portable scalar + AArch64 SIMD + x86-64 SIMD implementations;
- optimized hash probing matches scalar slot semantics exactly;
- ARM64 is the primary tuned implementation;
- x86-64 compiles and passes equivalent tests;
- inline asm, if present, has measured justification;
- all existing validation remains green;
- full native-vs-V2.4 benchmarks are rerun twice;
- memory ratios do not regress materially due to the optimization;
- benchmark report contains both native comparison and pre/post V2.4 optimization deltas;
- residual bottlenecks are documented instead of hidden.

---

# Final-diff checklist

Before completion:

- [ ] current `main` reconciled;
- [ ] no retained native pointer added;
- [ ] owner/header/frozen sizes unchanged;
- [ ] no allocator architecture change;
- [ ] owner hot operations resolve header/base once where possible;
- [ ] vector bulk paths publish initialized count safely;
- [ ] panic/drop tests cover partially initialized bulk writes;
- [ ] deque iterator resolves once;
- [ ] deque per-element modulo eliminated or proven optimized away;
- [ ] hash scalar group classifier exists;
- [ ] AArch64 NEON implementation exists;
- [ ] x86-64 SIMD implementation exists;
- [ ] scalar/SIMD probing differential tests pass;
- [ ] no OOB SIMD loads;
- [ ] Miri uses portable path and is green;
- [ ] ARM64 release disassembly inspected;
- [ ] x86-64 release compilation/tests pass;
- [ ] full benchmark suite run at least twice;
- [ ] native vs optimized V2.4 results published;
- [ ] previous V2.4 vs optimized delta published separately;
- [ ] B5 memory/CPU tradeoff did not materially regress;
- [ ] B8 residual teardown bottleneck documented;
- [ ] no unsupported performance claims;
- [ ] `PLAN.md` removed before implementation commit.

---

# Execution handoff

1. Verify latest `main`.
2. Read this complete plan.
3. Preserve the frozen V2.4 retained representation.
4. Implement resolution hoisting first.
5. Implement bulk vector/byte/string operations using the new resolved primitives.
6. Replace per-element deque traversal with resolved pointer/slice traversal.
7. Add scalar hash control-group scanning and differential tests.
8. Add ARM64 NEON implementation.
9. Add x86-64 SIMD implementation.
10. Use inline asm only after inspecting intrinsic-generated release code.
11. Run targeted benchmarks after each phase.
12. Run complete tests, Miri, and architecture-specific compilation.
13. Run the authoritative benchmark suite twice on the same ARM64 host.
14. Compare native vs optimized V2.4 as the primary result.
15. Compare frozen published V2.4 vs optimized V2.4 as a secondary optimization result.
16. Update benchmark/safety/architecture documentation.
17. Review the final diff against every invariant.
18. Delete `PLAN.md`.
19. Commit the implementation without planning files remaining.

Executor prompt:

> Implement PLAN.md exactly. Verify latest main first. Preserve all frozen V2.4 retained layouts and APIs. Optimize by hoisting cage/header resolution, adding panic-safe bulk collection operations, using direct resolved traversal, and adding scalar + ARM64 NEON + x86-64 SIMD hash-control probing. Benchmark native Rust against optimized V2.4, report the pre/post V2.4 delta separately, run all tests/Miri, delete PLAN.md, and commit.
