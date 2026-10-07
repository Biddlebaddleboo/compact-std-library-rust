# Workstream Plan: Runtime, Ownership, Reclamation, and Packed Performance

## Objective

Remove the remaining fundamental runtime limitations in V2:

- monotonic dynamic-buffer waste;
- `T: Copy` as the blanket generic-container restriction;
- slow bit-by-bit packed access.

This workstream owns all allocator and generic ownership safety decisions.

---

# Implementation scope

Inspect first:

## Core

`crates/compact_core/src/arena.rs`

Exact symbols:

- `Arena`
- `Arena::allocate`
- `Arena::checked_range`
- `Arena::alloc_uninit`
- `Arena::alloc_value`
- `Arena::alloc_slice`
- `Arena::alloc_uninit_slice`
- `Arena::write_uninit`
- `Arena::write_uninit_at`
- `Arena::copy_slice_assume_init`
- `Arena::get_slice_assume_init`
- `Arena::get_slice_mut_assume_init`
- `Arena::alloc_bytes`
- `Arena::alloc_zeroed_bytes`
- `Arena::copy_bytes`

`crates/compact_core/src/offset.rs`

- `Offset32`
- `OffsetSlice32`

`crates/compact_core/src/bytes.rs`

- `ByteRange32`

`crates/compact_core/src/packed.rs`

- `read_bits`
- `write_bits`
- `BitField`
- `PackedWord`

`crates/compact_core/src/error.rs`

- `Error`

## Collections

`crates/compact_collections/src/boxed.rs`

- `CompactBox`
- `CompactOption`

`crates/compact_collections/src/vec.rs`

- `CompactVec`
- `CompactVec::reserve_in`
- `push_in`
- `pop_in`
- `truncate`
- `clear`

`crates/compact_collections/src/small.rs`

- `CompactSmallVec`
- promotion logic

`crates/compact_collections/src/slab.rs`

- `CompactSlab`
- `insert`
- `remove`
- slot-generation logic

`crates/compact_collections/src/string.rs`

- `CompactString`
- `push_str_in`
- `clear`
- heap representation

`crates/compact_collections/src/intern.rs`

Read for compatibility; avoid unnecessary changes.

---

# Proposed additions

Exact names may change if repository style demands it, but keep responsibilities equivalent.

Possible core modules:

```text
crates/compact_core/src/
  allocation.rs
  free_list.rs
```

Possible types:

```rust
Allocation32<'arena>
Layout32
```

or equivalent private/internal descriptors.

Do not expose raw free-list internals as public API.

---

# Reusable allocator design

## Fast path

Preserve bump allocation for new tail allocations.

The ordinary allocation path should first use:

- suitable reusable range when profitable;
- otherwise bump tail.

Benchmark ordering rather than assuming free-list search is always cheaper than bumping.

## Free ranges

Track free ranges inside arena-managed storage or in compact bounded arena metadata.

Requirements:

- offset;
- length;
- alignment compatibility;
- no native allocation per free range;
- no dangling free-list records pointing into reused user payload.

A free-range representation may use released arena bytes themselves for linkage when the range is large enough, but tiny ranges need a defined strategy.

Do not create unsound typed references into free memory.

## Coalescing

On release:

- merge directly adjacent free ranges where practical;
- reclaim the high-water tail by moving `cursor` backward when the released/coalesced range reaches the current tail;
- repeat tail contraction through adjacent free ranges if applicable.

This tail contraction is especially important for vector growth patterns.

## In-place resize

Provide a primitive sufficient for collections to ask:

```text
can this allocation grow to N bytes without moving?
```

Supported cases should include:

- allocation is at current bump tail;
- immediately following range is free and large enough.

Shrinking may release the trailing portion.

Do not let collections manipulate allocator internals directly.

---

# Allocation identity

Releasing arbitrary `(offset, len)` supplied by safe user code would be unsafe.

Release/reallocation APIs must require an allocator-issued descriptor or otherwise preserve enough private information to prove:

- exact allocation start;
- exact allocation extent;
- allocation is currently live;
- allocation has not already been released.

Do not allow double-free through safe APIs.

The ordinary `Offset32` must remain a reference, not become a freeable owning token.

---

# Ownership trait design

Introduce a sealed or carefully unsafe trait representing values the runtime can place/move/drop in compact storage.

Do not call arbitrary Rust values compact-safe by default.

The contract must state:

- whether raw byte relocation is valid;
- whether explicit move construction is required;
- how destruction occurs;
- whether address stability is required;
- whether native references may be embedded.

Prefer safe blanket implementations only for classes whose correctness is mechanically justified.

Potential conceptual split:

```rust
trait CompactValue { ... }

unsafe trait CompactRelocatable: CompactValue { ... }
```

Do not finalize these names without checking ergonomics.

Generated compact handles and offset-based container handles should implement the compact-safe contract where correct.

Primitive scalar types should be supported.

---

# Non-Copy relocation

For an element that cannot be byte-copied:

1. allocate destination as uninitialized storage;
2. move elements one by one using pointer read/write or equivalent ownership transfer;
3. track exactly how many destination elements have been initialized;
4. if a later operation fails:
   - either finish a non-fallible remaining move phase after all fallible work is complete;
   - or roll back safely using a guard;
5. only after successful relocation update vector metadata;
6. release the old raw storage without dropping already-moved values.

Prefer structuring relocation so all allocation/fallible checks happen before moving the first element.

That minimizes rollback complexity.

---

# Drop semantics

## CompactVec

Implement `Drop` only if the wrapper can safely retain access to the owning arena at drop time.

If the current representation cannot do that without adding a native pointer to every vector, do not compromise the 12-byte handle merely to force ordinary `Drop`.

Instead establish an explicit owner/context model that:

- guarantees container destruction while the arena exists;
- remains ergonomic under `arena!`;
- cannot silently omit `T::drop`.

The executor must compare at least these models:

1. wrapper stores native arena pointer;
2. scope-managed destructor registry;
3. explicit arena-owned owner token/context;
4. restricted generic support to destructor-free relocatable types.

Reject any model that introduces per-access registry lookup or large per-container overhead merely for convenience.

Because compactness is the project goal, correctness and representation cost must both be documented before choosing.

## Critical decision gate

Do not implement non-`Copy` containers until this drop-access question is resolved.

Record the chosen model in the implementation report.

---

# Preferred scope-managed option

If an arena-local destructor registry is chosen:

- registry must be owned by `Arena`;
- registration metadata should be compact and amortized;
- destruction must happen in reverse creation order where Rust ownership dependencies require it;
- explicitly freed/moved allocations must unregister or mark entries inactive idempotently;
- arena teardown must invoke remaining destructors before backing becomes unavailable;
- panic behavior during drop must be defined;
- no double-drop after explicit container clear/release;
- no native heap allocation per element.

Do not use trait-object/dynamic-dispatch registration per scalar element.

Registration should occur per owning container/allocation when possible.

---

# CompactVec requirements

Maintain compact general metadata target.

Add internal allocation ownership information only if essential; quantify any increase.

Required operations:

### reserve

- in-place grow if possible;
- otherwise allocate replacement;
- perform safe relocation;
- commit metadata;
- release old allocation.

### push

- reserve first;
- write one new value;
- increment `len` only after initialization.

### pop

For non-`Copy`:

- decrement logical length as part of a carefully ordered move-out;
- return ownership exactly once.

### truncate

For every removed initialized element:

- run destructor if required.

Then shrink/release trailing storage only according to capacity policy.

### clear

Drop all live elements.

Capacity may remain for reuse unless policy explicitly releases it.

### drop/release

Drop all live elements then release backing.

---

# Growth policy

Keep geometric growth unless benchmarks show another policy materially improves compact-arena use.

With reclamation, geometric growth is acceptable because replaced buffers cease to consume live capacity.

Add tests proving old buffers are reusable.

---

# CompactString requirements

Because bytes have trivial destruction:

- allocator reclamation is simpler than generic vector relocation.

On heap growth:

1. allocate/grow;
2. copy old initialized bytes;
3. append new bytes;
4. commit new metadata;
5. release old byte allocation.

On `clear`:

- decide whether to retain capacity like std `String::clear` or explicitly offer a releasing operation;
- ordinary `clear` should preferably preserve familiar Rust semantics and retain capacity;
- add `shrink_to_fit_in` or equivalent if release is needed.

On drop/scope destruction:

- release heap backing.

Preserve inline strings.

---

# CompactBox

Move from `T: Copy` to the accepted compact value contract.

Required:

- unique ownership;
- exactly-once destructor;
- release allocation after destructor;
- move of the handle transfers ownership without duplicating destructor registration.

---

# CompactSmallVec

Inline storage of non-`Copy` values requires explicit initialization tracking.

Required:

- no `[MaybeUninit<T>; N]` operation may assume `T: Copy`;
- promotion moves each initialized value;
- old inline slots become logically uninitialized after move;
- promotion failure before movement preserves inline state;
- destruction distinguishes inline vs heap representation.

---

# CompactSlab

Support accepted move/drop values.

Existing slot union must be revised carefully because its current `Copy` derivations rely on `T: Copy`.

Required:

- vacant slot metadata and occupied value representation remain disjoint;
- remove moves the value out;
- stale handles remain rejected;
- dropping slab drops every occupied value once;
- reusing a slot never observes old object state.

Preserve generation retirement behavior.

---

# Packed fast paths

Rewrite `read_bits` and `write_bits` around byte spans rather than individual bits.

For a field:

```text
first_byte = bit_offset / 8
intra      = bit_offset % 8
bytes_needed = ceil((intra + width) / 8)
```

For `bytes_needed <= 8`:

- load bytes into `u64`;
- normalize according to documented native/LSB contract;
- shift/mask;
- write back only the touched bytes.

Be careful with the case where `intra + width > 64`; a 64-bit-wide field beginning mid-byte can require 9 bytes.

For that case:

- use a two-word path or fallback;
- never truncate.

Provide explicit tests for width 64 at bit offsets 0 and nonzero.

---

# Error/failure safety

Add allocator errors where necessary, but avoid exposing implementation-specific fragmentation details unless actionable.

Every operation that returns `Err` must document whether:

- no state changed;
- capacity may have changed but value sequence did not;
- operation is otherwise strongly exception/failure safe.

Prefer the strong guarantee for reserve/growth.

---

# Tests

Create deterministic tests around exact symbols.

## Reclamation

- allocate A/B/C; free B; compatible D reuses B.
- free adjacent B/C; larger D uses coalesced region.
- release tail; `used_bytes()`/high-water behavior contracts tested.
- vector capacities 4→8→16→32 do not permanently charge all historical buffers.
- repeated grow/drop/recreate reuses bounded arena space.

## Ownership

Use a local `DropCounter` with deterministic counters.

Test:

- box exactly once;
- vector push/pop exactly once;
- truncate;
- clear;
- relocation;
- failed relocation setup;
- small-vector promotion;
- slab remove;
- slab destruction.

## Packed

Test all:

- widths 1..64;
- starting offsets around 7/8, 15/16, 31/32, 63/64 boundaries;
- neighbor preservation;
- zero-width behavior remains whatever current public contract specifies;
- output equals a simple test-only reference bit loop.

---

# Benchmarking

Add a lightweight benchmark target or examples suitable for repeatable release-mode measurement.

Measure:

- bump allocation baseline;
- allocate/free/reuse;
- vector growth;
- vector traversal;
- packed boolean read/write;
- 3-bit read/write;
- aligned 16/32-bit field;
- cross-byte field.

Compare optimized packed implementation against the old bit-loop reference kept only in benchmark/test code.

Do not add a large benchmarking framework if `std::time::Instant` release-mode examples are sufficient for initial data.

---

# Non-goals

Do not add:

- thread-safe allocator;
- per-object native heap allocation;
- general pinned/self-referential support;
- GC;
- reference counting by default;
- multi-arena references;
- 16-bit local offsets.

---

# Required handoff

Report:

- chosen reusable allocation structure;
- allocator metadata bytes;
- fragmentation/coalescing policy;
- chosen generic ownership/drop model and why;
- exact supported non-`Copy` class;
- representation-size changes, if any;
- unsafe relocation invariants;
- vector/string reuse measurements;
- packed benchmark before/after;
- changed files;
- commit SHA;
- validation commands/results;
- deviations and unresolved assumptions.
