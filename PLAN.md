# PLAN.md — V2.3 process-wide compact cage redesign

## Objective

Replace the V2.2 multi-arena memory model with one process-wide compact cage.

V2.3 is intentionally a clean replacement of the V2.2 architecture. There are no applications depending on the V2.2 API, so implementation must not preserve, wrap, deprecate, migrate, or emulate the previous public model.

The redesign prioritizes:

- no native pointers stored in compact values or compact owners;
- one unambiguous 32-bit address domain per process;
- substantially smaller retained collection/owner metadata;
- less lifetime and allocator plumbing;
- fewer public concepts;
- simpler application integration;
- retention of packing, direct Serde, frozen immutable graphs, scratch allocation, and explicit FFI boundaries where useful.

The process remains an ordinary native process. On x86-64:

- Rust/native pointers remain 64-bit;
- libc/syscall/FFI boundaries remain normal native pointers;
- retained compact addresses use 32-bit offsets;
- one native cage base exists only in process runtime state.

The defining invariant is:

```text
compact native address = process_cage_base + offset32
```

Offset zero remains reserved as null.

Do not preserve V2.2 machinery merely because it already exists. Delete abstractions made unnecessary by the cage.

---

# Intended V2.3 model

```text
process
│
├── native Rust / OS world
│   ├── stack references
│   ├── libc / syscalls
│   ├── FFI pointers
│   └── ordinary third-party allocations
│
└── process-wide compact cage
    ├── runtime/allocator metadata
    ├── general compact allocations
    ├── scratch blocks
    └── frozen graph blocks
```

All compact retained addresses use the same `u32` offset space.

There are no independently branded compact arenas.

Different allocation policies may exist inside the cage, but they do not create distinct address spaces.

---

# Compatibility policy

There is no V2.2 compatibility requirement.

Delete rather than preserve:

- `Arena<'arena, '_>`;
- `ArenaAllocation<'arena, T>`;
- `StdArena`;
- user-facing `StdBacking`;
- `CompactStore`;
- `RootHandle`;
- `StoreRoot`;
- persistent-arena attachment/rebranding;
- collection `*_in(..., arena)` methods;
- `arena!(arena, { ... })`;
- V2.1/V2.2 compatibility fixtures;
- compatibility aliases for removed types;
- migration helpers;
- migration feature flags;
- deprecated wrappers.

Do not produce a V2.2-to-V2.3 migration guide.

Documentation should describe V2.3 as the current architecture without teaching users how to convert code that was never deployed.

V2.2 may be referenced only internally during implementation as a behavioral, safety, and benchmark comparison point.

---

# Verified repository baseline

Plan against:

`ca9ef013de99f003cbf046ee241c5e48f75fa85a`

`feat(release): complete v2.2.0`

Relevant V2.2 facts:

- `ArenaAllocation<'arena, T>` contains a native `NonNull<ArenaState>`, a `u32` offset, a `u32` allocation ID, and marker state.
- `CompactVec` owns `Option<ArenaAllocation<T>>`.
- `CompactBox` owns one `ArenaAllocation<T>`.
- heap-backed `CompactString` owns `ArenaAllocation<u8>`.
- `ArenaState` carries allocator state and a native backing pointer.
- `CompactStore` exists only to preserve/rebrand a separately owned arena.
- `arena!` contains substantial machinery to inject arena arguments.
- frozen graphs currently use a separate backing.
- V2.2 already has useful behavioral implementations and tests for:
  - bytes;
  - VecDeque/ring;
  - maps/sets;
  - OS strings/paths;
  - Serde;
  - frozen graphs;
  - FFI;
  - packing;
  - Miri/property/fuzz tests;
  - workload benchmarks.

Use these only as implementation references. Their arena architecture is disposable.

---

# Architectural invariants

## One address domain

There is exactly one compact cage per process.

Every retained compact address is a `u32` offset.

A compact value or owner must not retain:

```text
NonNull<CageState>
*const CageState
*mut CageState
native cage base pointers
native allocator-state pointers
```

Native pointers may exist only transiently inside runtime implementation and native borrows.

## Process lifetime

The cage is initialized once and remains alive until process termination.

There is no public cage teardown or reinitialization.

## Allocator

Allocation, resizing, and release are synchronized.

Reading an already allocated value must not require the allocator lock.

Start with one correct allocator lock.

Do not implement sharding until benchmarks demonstrate a need.

## Failure

Cage exhaustion remains explicit.

No compact collection silently falls back to the native heap.

Fallible mutation must preserve the previous logical value where that behavior is promised.

## Native boundary

A cage offset is never an FFI/native pointer.

Resolve it to a native pointer only for a bounded native borrow or explicit exported allocation.

---

# Phase 0 — capture the V2.2 implementation reference

Before replacing code:

1. verify the latest `main`;
2. run the current V2.2 tests;
3. record the existing V2.2 workload benchmarks;
4. retain the V2.2 commit SHA as an engineering reference.

This is not compatibility work.

Do not carry its fixtures or public API forward merely to keep them compiling.

---

# Phase 1 — process-wide cage runtime

Implement the cage runtime in `compact_backend_std`.

Proposed surface:

```rust
pub struct CompactRuntime;

pub struct CageConfig {
    pub capacity: usize,
}

impl CompactRuntime {
    pub fn init(config: CageConfig) -> Result<()>;
    pub fn is_initialized() -> bool;
    pub fn capacity() -> Result<usize>;
    pub fn used_bytes() -> Result<usize>;
    pub fn remaining_bytes() -> Result<usize>;
}
```

The cage backing is allocated once and never moved.

Do not automatically allocate 4 GiB.

The caller chooses capacity up to the offset limit.

Use race-safe one-time process initialization.

Double initialization must fail deterministically and must never replace the cage.

Runtime state may hold one native base pointer.

That pointer exists once per process, not once per owner.

Conceptually:

```text
CageRuntime
    base
    capacity
    allocator mutex
```

Do not duplicate the cage base into compact objects.

---

# Phase 2 — four-byte cage owners

Delete `ArenaAllocation`.

Introduce a thin non-`Copy` owner:

```rust
#[repr(transparent)]
pub struct CageAllocation<T: CompactValue> {
    offset: NonZeroU32,
    marker: PhantomData<T>,
}
```

Target:

```text
CageAllocation<T>         4 bytes
Option<CageAllocation<T>> 4 bytes
```

Do not carry a universal allocation ID.

Rust ownership already prevents safe duplication of the owner.

Generation/instance IDs belong only in abstractions that expose independently copyable stale handles.

Initially preserve V2.2's proven allocation-header information:

```text
block length
alignment prefix
capacity
initialized length
```

Do not mix header compression into the first cage conversion.

`CageAllocation<T>` must support:

```text
len
capacity
get
get_mut
as_slice
as_mut_slice
push
pop
truncate
move_into
resize
drop
```

All access resolves through the global cage.

Drop must destroy every initialized element exactly once and return the block to the cage allocator.

No arena argument is involved.

---

# Phase 3 — lifetime-free compact offsets

Replace:

```rust
Offset32<'arena, T>
OffsetSlice32<'arena, T>
ByteRange32<'arena>
CompactOption<'arena, T>
```

with:

```rust
Offset32<T>
OffsetSlice32<T>
ByteRange32
CompactOption<T>
```

Target:

```text
Offset32<T>      4 bytes
CompactOption<T> 4 bytes
```

Offsets are non-owning.

Do not expose safe arbitrary-integer construction.

Stale-handle protection remains the responsibility of APIs that expose long-lived independently copyable handles.

---

# Phase 4 — delete the arena architecture

After cage ownership works, remove:

```text
Arena
ArenaState
ArenaInner
ArenaAllocation
with_arena
StableBacking where no longer necessary
StdArena
user-facing StdBacking arena API
persistent arena headers
persistent attach
persistent raw-offset rebranding
CompactStore
RootHandle
StoreRoot
```

Do not leave compatibility wrappers.

Do not mark them deprecated.

Delete them.

---

# Phase 5 — compact collections

Convert every owning collection to `CageAllocation`.

No normal collection operation accepts an arena.

## CompactBox

Target:

```text
4 bytes
```

API:

```rust
CompactBox::new(value)?
box.get()
box.get_mut()
```

Preserve `Deref`/`DerefMut`.

## CompactVec

Preferred representation:

```rust
struct CompactVec<T> {
    storage: Option<CageAllocation<T>>,
}
```

with length/capacity remaining in allocation metadata.

Target:

```text
4 bytes
```

API:

```text
new
with_capacity
len
capacity
reserve
push
pop
get
get_mut
as_slice
as_mut_slice
iter
truncate
clear
shrink_to_fit
```

Allocation-bearing operations remain fallible.

Benchmark header lookup cost before considering duplicated length/capacity fields.

## CompactString

Keep small-string optimization.

Use a four-byte cage owner for heap representation.

Target no more than 16 bytes if that can coexist with the current inline payload.

Rebenchmark the inline threshold after the owner shrinks.

## CompactBytes

Use the cage owner.

Re-evaluate the current inline size against the new representation.

## CompactVecDeque

Likely representation:

```text
storage owner  4
head           4
length         4
```

Capacity comes from allocation metadata.

Target approximately 12 bytes.

## HashMap / HashSet

Use cage allocations for control and entry tables.

Preserve randomized hashing.

No arena parameters.

Do not redesign the hash algorithm unless required by correctness or measured memory/performance.

## Slab

Keep stale-handle generations at slab/slot level.

Do not reintroduce a universal allocation generation field.

## Remaining collections

Convert:

- ring;
- SmallVec;
- interner;
- bit vector;
- OS string;
- path;
- generated SoA storage.

No retained native runtime pointer is permitted.

---

# Phase 6 — remove arena helper traits

Delete:

```text
CloneIn
FromIteratorIn
ExtendIn
ToCompactStringIn
```

Replace only where useful with explicitly fallible cage operations such as:

```text
TryClone
TryFromIterator
TryExtend
TryToCompactString
```

Do not implement infallible `Clone` when an allocation can fail.

---

# Phase 7 — remove `arena!`

Delete the public `arena!` model.

Its primary purpose—injecting arena arguments—no longer exists.

Normal compact code should look like:

```rust
let mut values = Vec::new();
values.push(10)?;
values.push(20)?;
```

Do not maintain the large lexical rewrite engine for historical syntax.

If useful, add narrowly scoped explicit fallible macros:

```text
compact_vec!
compact_format!
```

These must not require a lexical type-analysis engine.

Keep `#[compact]` because packing remains useful.

Delete the obsolete macro implementation and dependencies after its remaining responsibilities have been separated.

---

# Phase 8 — packing without arenas

Preserve all useful `#[compact]` functionality:

- packed booleans;
- bounded integers;
- compact enum discriminants;
- hot/cold sections;
- SoA.

Generated handles use cage offsets and have no arena lifetime.

Desired usage:

```rust
let compact = value.compact()?;
let retries = compact.retries()?;
compact.set_retries(4)?;
```

No arena argument.

No generated native pointer.

---

# Phase 9 — cage scratch regions

Scratch is a temporary allocation policy inside the same cage.

A scratch region should own one cage block:

```text
ScratchRegion
    block_offset
    cursor
    capacity
```

The stack wrapper must not retain a native cage pointer.

All temporary addresses remain ordinary global cage offsets.

On scratch destruction:

1. run required destructor records if supported;
2. release the complete scratch block.

Retain compile-time/lifetime escape prevention where needed.

Do not introduce a second compact address domain.

---

# Phase 10 — cage-backed frozen graphs

Replace the separate frozen backing with cage storage.

Preferred top-level model:

```rust
FrozenGraph<T>
```

owning one contiguous cage allocation.

Its internal references are cage offsets.

Its root does not require a separate `FrozenArena` parameter for every access.

Frozen data remains:

- immutable;
- `Send + Sync` where its value contract permits;
- lock-free for reads;
- bulk-released when the graph owner drops.

A temporary native construction buffer is acceptable initially if retained data is entirely cage-backed and the benchmark reports the cost.

---

# Phase 11 — direct Serde through the cage

Keep direct compact Serde.

Replace:

```rust
from_slice_in(input, arena)
from_str_in(input, arena)
```

with APIs such as:

```rust
from_slice(input)
from_str(input)
```

Visitors construct cage-backed compact collections directly.

Derived compact types should not carry arena lifetimes merely for allocation ownership.

Preserve partial-construction cleanup and all current malformed/edge-input tests.

---

# Phase 12 — FFI

Keep V2.2's native-boundary philosophy.

Cage-backed values may provide:

```text
as_ptr
as_slice
as_str
with_ffi_bytes
```

without an arena argument.

The native pointer is a temporary resolved address.

It becomes invalid after owner destruction or relocating mutation.

Use `FfiByteBuffer` or an equivalent explicit native allocation when native code must retain memory.

Never expose a `u32` offset as a C/Swift/JNI address.

---

# Phase 13 — concurrency

A single cage is one address domain, not one-thread-only storage.

Use a synchronized allocator.

Reads of live allocations must not take the global allocator mutex.

Allow `Send`/`Sync` according to element semantics rather than artificial arena restrictions.

Test:

```text
allocate on one thread / drop on another
parallel independent allocations
parallel releases
shared immutable compact values
non-Send values rejected
parallel frozen reads
```

Start with one allocator lock.

Only add sharding after real measurements justify it.

---

# Phase 14 — hard representation requirements

Add size tests for:

```text
Offset32<T>                    = 4
CompactOption<T>               = 4
CageAllocation<T>              = 4
Option<CageAllocation<T>>      = 4
CompactBox<T>                  = 4
```

Desired, subject to correctness:

```text
CompactVec<T>                  = 4
CompactVecDeque<T>             ≈ 12
persistent ptr+len view        = 8
CompactString                  <= 16
```

Every larger layout must be documented and justified.

The final audit must explicitly confirm that no compact owner contains a native runtime pointer.

---

# Phase 15 — V2.3 fixtures only

Delete the old compatibility fixture.

Do not create migration fixtures.

Create:

```text
fixtures/v2_3_cage_contract
```

covering only the intended V2.3 architecture:

```text
runtime initialization
Vec / Box / String
HashMap / HashSet
PathBuf / OsString
direct Serde
packing
scratch
frozen graph
FFI borrowing
```

Update the real-world fixture directly to V2.3 APIs.

No fixture should retain `Arena`, `StdArena`, `CompactStore`, `RootHandle`, or ordinary `*_in` usage.

---

# Phase 16 — retain the V2.2 safety program

Port rather than weaken:

- Miri;
- property tests;
- allocator reference-model tests;
- fuzzing;
- panic/drop tests.

Update fuzz targets so none require an arena.

Continue testing:

```text
allocator operations
packed bits
HashMap collision paths
Serde JSON/TOML
chunk bytes
frozen graph access
```

Add concurrent allocator stress tests.

---

# Phase 17 — benchmark against V2.2

The old V2.2 numbers are useful only as engineering baselines.

Compare:

- wrapper sizes;
- arena/cage high-water use;
- allocator throughput;
- traversal;
- log ring;
- maps/sets;
- PathBuf workload;
- chunk assembly;
- String/Bytes;
- direct config Serde;
- frozen catalog;
- concurrent allocation.

Acceptance requires:

1. no native pointer in compact owners;
2. materially smaller Box/Vec representations;
3. no arena parameters in normal collection APIs;
4. no meaningful steady-state read/traversal regression;
5. acceptable allocation throughput.

Do not claim compatibility or migration based on these comparisons.

---

# Phase 18 — documentation

Rewrite documentation as V2.3 documentation.

Do not include a migration guide.

Do not include a deprecated-API section.

Do not describe V2.1 or V2.2 as supported contracts.

Historical releases can remain visible through git history/tags.

The current docs should simply describe:

```text
process-wide cage
u32 compact offsets
one runtime base
thin owners
global allocator
scratch/frozen policies
native borrow boundary
```

Update `SAFETY.md` for the cage invariants.

Update `BENCHMARKS.md` with V2.3 results and optional V2.2 comparison tables strictly as benchmark history.

---

# Phase 19 — release cleanup

After implementation is green:

- set version to 2.3.0;
- update `ABI_VERSION`;
- remove dead arena files/modules;
- remove obsolete macro implementation;
- remove obsolete tests and fixtures;
- update Cargo.lock;
- update docs/benchmarks.

Repository-wide search for:

```text
Arena<
ArenaAllocation
StdArena
CompactStore
RootHandle
StoreRoot
with_arena
with_persistent_arena
arena!
V2.1 compatibility baseline
V2.2 compatibility
migration
deprecated
```

Any remaining hit must have a concrete reason unrelated to compatibility support.

---

# Non-goals

V2.3 does not:

- change Rust's native pointer width;
- make ordinary `&T` 32-bit;
- change libc or syscall ABI;
- create a rustc target;
- require the cage below virtual address 4 GiB;
- add garbage collection;
- add moving GC;
- support multiple cages;
- support cage teardown/reinitialization;
- maintain V2.2 source compatibility;
- provide V2.2 migration tooling.

---

# Integration order

Implement sequentially:

```text
0  capture V2.2 reference tests/benchmarks
1  cage runtime
2  thin owner
3  lifetime-free offsets
4  delete arena/store model
5  collections
6  helper traits
7  remove arena!
8  packing
9  scratch
10 frozen graphs
11 Serde
12 FFI
13 concurrency
14 representation assertions
15 V2.3 fixtures
16 safety/fuzz/property validation
17 benchmarking
18 docs
19 release cleanup
```

Do not preserve old APIs as temporary architecture beyond the minimum needed to keep intermediate commits buildable.

The final tree must contain one coherent cage model.

---

# Final validation

Run:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Run all V2.3 fixtures.

Run Miri.

Run property tests.

Run fuzz regression/smoke testing.

Run the cage workload benchmarks.

Review all unsafe code.

Manually verify public compact owner layouts contain no native runtime pointer.

---

# Final checklist

- one process-wide cage;
- one-time initialization;
- no public teardown;
- four-byte offsets;
- four-byte `CageAllocation`;
- no native pointer in a compact owner;
- four-byte `CompactBox` unless correctness proves impossible;
- materially smaller `CompactVec`;
- no arena parameters on normal collection methods;
- no arena/store reattachment architecture;
- no universal allocation identity tax;
- correct stale slab generations;
- exact-once destruction;
- cage-backed frozen graphs;
- packing preserved;
- direct Serde preserved;
- native FFI boundary preserved;
- synchronized allocator;
- normal Rust Send/Sync semantics where sound;
- Miri/property/fuzz green;
- V2.3 benchmarks recorded;
- no compatibility layer;
- no migration layer;
- no deprecated V2.2 API;
- old architecture deleted.

---

# Execution handoff

Begin with:

> Implement PLAN.md exactly against latest main. V2.3 is a clean replacement, not a migration. Use V2.2 only as a benchmark and correctness reference while deleting its arena architecture. The defining invariant is one process-wide cage with no native pointers in compact values or owners. Keep the native OS/FFI world 64-bit. Do not carry compatibility wrappers, deprecated aliases, or migration tooling.

Each implementation phase reports:

```text
changed files
commit SHA
tests run
test results
representation sizes
deviations
unresolved safety assumptions
```

At completion:

1. compare final diff with this plan;
2. review every unsafe block;
3. run all validation;
4. compare benchmarks with the recorded V2.2 baseline;
5. delete `PLAN.md`;
6. commit the final implementation without planning files remaining.
