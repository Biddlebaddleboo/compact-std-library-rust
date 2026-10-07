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

`crates/compact_core/src/offset.rs`: `Offset32`, `OffsetSlice32`

`crates/compact_core/src/bytes.rs`: `ByteRange32`

`crates/compact_core/src/packed.rs`: `read_bits`, `write_bits`, `BitField`, `PackedWord`

`crates/compact_core/src/error.rs`: `Error`

## Collections

Inspect:
- `crates/compact_collections/src/boxed.rs`: `CompactBox`, `CompactOption`
- `crates/compact_collections/src/vec.rs`: `CompactVec`, `reserve_in`, `push_in`, `pop_in`, `truncate`, `clear`
- `crates/compact_collections/src/small.rs`: `CompactSmallVec` and promotion logic
- `crates/compact_collections/src/slab.rs`: `CompactSlab`, `insert`, `remove`, generation logic
- `crates/compact_collections/src/string.rs`: `CompactString`, `push_str_in`, `clear`
- `crates/compact_collections/src/intern.rs`: read for compatibility; avoid unnecessary changes.

---

# Proposed additions

Possible core modules:

```text
crates/compact_core/src/
  allocation.rs
  free_list.rs
```

Possible private/internal descriptors:

```rust
Allocation32<'arena>
Layout32
```

Do not expose raw free-list internals as public API.

---

# Reusable allocator design

Preserve bump allocation as the fast tail path. Released ranges must become reusable without adding a native allocation record per compact allocation.

Free ranges must track offset, length, and alignment compatibility. Metadata must be compact/bounded. Released arena bytes may be used for linkage when safe and large enough, but tiny ranges need a defined strategy. Do not create unsound typed references into free memory.

On release, coalesce directly adjacent free ranges where practical. If the free/coalesced range reaches the current tail, contract `cursor`; repeat through adjacent free ranges. This is especially important for geometric vector growth.

Provide a core primitive sufficient for containers to attempt in-place growth when:
- the allocation is at the bump tail; or
- an immediately following reusable range can satisfy the extension.

Shrinking may release the trailing part. Collections must not manipulate allocator internals directly.

Benchmark whether reusable-range search or bump-first ordering is best rather than assuming.

---

# Allocation identity

Safe release/reallocation must not accept arbitrary user-provided `(offset, len)`.

Use allocator-issued ownership metadata or an equivalent private proof of:
- exact allocation start;
- exact allocation extent;
- currently live state;
- not previously released.

Safe APIs must prevent double free.

`Offset32` remains a reference and must not become the owning/freeable token.

---

# Ownership trait design

Do not fix generic containers by merely deleting `T: Copy`.

Define an explicit sealed or carefully unsafe compact-value contract distinguishing:

### Trivially relocatable values
Integers, offsets, generated compact handles, and other values with no destructor/address-sensitive self-reference. These may use bytewise relocation where justified.

### Move/drop arena values
Values with meaningful `Drop` semantics that can safely be moved between arena slots. Containers must move rather than duplicate them, drop live values exactly once, drop removed values, and clean up partially initialized destinations after failure.

### Rejected values
Values requiring stable native address, pinning, self-reference, unsupported borrowed native references, or other invariants the runtime cannot preserve. Reject them through trait bounds/explicit APIs.

Potential conceptual split:

```rust
trait CompactValue { ... }
unsafe trait CompactRelocatable: CompactValue { ... }
```

Names are not frozen. Prefer safe blanket impls only where mechanically justified.

---

# Non-Copy relocation

For non-byte-copy relocation:

1. allocate destination uninitialized;
2. complete all fallible validation before moving the first element when possible;
3. move elements one by one with correct ownership transfer;
4. track initialized destination length;
5. ensure failure either occurs before movement or uses a guard capable of safe rollback/cleanup;
6. commit container metadata only after successful relocation;
7. release old raw storage without dropping elements already moved.

No leaks, duplicate ownership, or double drops.

---

# Drop semantics decision gate

Current wrappers do not retain arena access at ordinary Rust drop time. Do not implement non-`Copy` containers until this is resolved.

Compare at least:
1. native arena pointer stored in wrapper;
2. arena scope-managed destructor registry;
3. explicit arena owner/context token;
4. restricting generic support to destructor-free relocatable types.

Reject any design that sacrifices the compact handle target or adds per-access registry lookup without strong justification.

If an arena-local destructor registry is chosen:
- it is owned by `Arena`;
- metadata is compact and amortized;
- remaining destructors run before backing disappears;
- explicit free/move unregisters or marks entries inactive idempotently;
- no double-drop after clear/release;
- panic behavior is documented;
- no native heap allocation per element;
- register per owner/allocation where possible, not per scalar element.

Record the chosen model and representation cost in the implementation report.

---

# CompactVec requirements

Preserve the 12-byte general handle if possible.

Required behavior remains `new`, `with_capacity`, `reserve`, `push`, `pop`, `truncate`, `clear`, indexed access, mutable access, slices, and iteration.

After V2.1:
- repeated growth releases/reuses superseded storage;
- in-place growth is used where possible;
- non-`Copy` relocation is ownership-correct;
- `pop` moves ownership out exactly once;
- `truncate` and `clear` destroy removed live values exactly once;
- dropping/releasing a vector destroys live elements and releases backing according to the chosen owner model;
- failed reserve/growth leaves the old vector authoritative.

Keep geometric growth unless measurements justify changing it.

---

# CompactString requirements

Keep current inline representation unless benchmarks justify a change.

For heap growth:
1. grow in place when possible, otherwise allocate replacement;
2. copy old initialized bytes;
3. append;
4. commit metadata;
5. release old allocation.

`clear` should preferably retain capacity like std `String::clear`; add `shrink_to_fit_in` or equivalent for explicit release if needed.

Scope/drop handling must release heap backing. Inline strings remain allocation-free.

---

# CompactBox

Support accepted non-`Copy` compact values.

Required:
- unique ownership;
- destructor exactly once when required;
- release allocation after destruction;
- moving the handle transfers ownership without duplicating destructor registration.

---

# CompactSmallVec

Remove assumptions that inline `MaybeUninit<T>` storage requires `T: Copy`.

Promotion must move initialized values exactly once. Failure before movement preserves inline state. Destruction must distinguish inline and heap representation.

---

# CompactSlab

Preserve stale-handle rejection, generation checks, and generation-wrap retirement.

Support accepted non-`Copy` values:
- revise unions/derives that rely on `T: Copy`;
- remove moves the occupied value out;
- reusing a slot cannot observe old value state;
- slab destruction drops each occupied value once.

---

# Packed fast paths

Rewrite `read_bits` and `write_bits` around byte spans instead of per-bit loops.

For a field:

```text
first_byte = bit_offset / 8
intra = bit_offset % 8
bytes_needed = ceil((intra + width) / 8)
```

For spans <= 8 bytes:
- load into `u64` or narrower scalar;
- normalize according to the existing LSB/native-byte-order contract;
- shift/mask once;
- preserve neighboring bits on write;
- use alignment-safe/unaligned-safe access.

A 64-bit-wide field with a nonzero intra-byte offset can require 9 bytes. Use a two-word path or correctness fallback; never truncate.

Test width 64 at offset 0 and nonzero offsets.

Generated macros should call these primitives rather than duplicate bit logic.

Use ordinary `#[inline]` only where small hot functions and measurements justify it; do not broadly use `#[inline(always)]`.

---

# Error/failure safety

Document for fallible operations whether:
- state is unchanged;
- capacity may change but logical contents do not;
- another guarantee applies.

Prefer the strong guarantee for reserve/growth.

---

# Tests

## Reclamation
- allocate A/B/C; free B; compatible D reuses B;
- free adjacent B/C; larger D uses coalesced space;
- release tail and verify tail contraction contract;
- vector 4→8→16→32 does not permanently charge all historical buffers;
- repeated grow/drop/recreate reuses bounded arena capacity;
- in-place tail extension;
- fragmentation fallback.

## Ownership
Use deterministic local drop counters:
- box exactly once;
- vector push/pop;
- truncate;
- clear;
- relocation;
- failed relocation setup;
- small-vector promotion;
- slab remove;
- slab destruction.

## Packed
Compare optimized functions with a test-only reference bit loop across:
- widths 1..64;
- offsets around byte/word boundaries;
- cross-byte fields;
- neighbor preservation;
- current zero-width contract.

Run focused Miri tests for unsafe relocation/ownership paths when available.

---

# Benchmarking

Add a lightweight repeatable release-mode benchmark/example for:
- bump allocation baseline;
- allocate/free/reuse;
- vector growth;
- vector traversal;
- packed boolean read/write;
- 3-bit read/write;
- aligned 16/32-bit field;
- cross-byte field.

Compare optimized packed access against the old reference bit loop kept only in benchmark/test code.

Do not add a heavy benchmark framework unless needed.

---

# Non-goals

Do not add:
- thread-safe allocator;
- per-object native heap allocation;
- pinned/self-referential support;
- GC;
- default reference counting;
- multi-arena references;
- 16-bit local offsets.

---

# Required handoff

Report:
- allocator structure;
- allocator metadata bytes;
- fragmentation/coalescing policy;
- ownership/drop model and rationale;
- exact supported non-`Copy` class;
- representation-size changes;
- unsafe relocation invariants;
- vector/string reuse measurements;
- packed benchmark before/after;
- changed files;
- commit SHA;
- validation commands/results;
- deviations and unresolved assumptions.
