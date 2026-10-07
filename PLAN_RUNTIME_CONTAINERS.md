# Workstream Plan: Runtime and Compact Containers

## Objective

Build the runtime/container layer that makes arena-resident ownership practical without exposing low-level offsets to ordinary application code.

Primary outputs:

- hardened byte access;
- compact allocation descriptors;
- `CompactBox`;
- `CompactVec`;
- `CompactString`;
- inline/small storage;
- slab allocation;
- interning.

## Starting files

Inspect first:

```text
crates/compact_core/src/arena.rs
crates/compact_core/src/offset.rs
crates/compact_core/src/native.rs
crates/compact_core/src/packed.rs
crates/compact_core/src/layout.rs
crates/compact_core/src/error.rs
crates/compact_core/src/lib.rs
crates/compact_backend_std/src/memory.rs
```

Proposed additions:

```text
crates/compact_core/src/bytes.rs
crates/compact_core/src/range.rs

crates/compact_collections/Cargo.toml
crates/compact_collections/src/lib.rs
crates/compact_collections/src/boxed.rs
crates/compact_collections/src/vec.rs
crates/compact_collections/src/string.rs
crates/compact_collections/src/small.rs
crates/compact_collections/src/slab.rs
crates/compact_collections/src/intern.rs
```

## Verified facts

- `Arena` currently uses a monotonic bump cursor.
- Safe typed allocations currently require `T: Copy`.
- `Offset32<T>` is four bytes.
- `OffsetSlice32<T>` stores a four-byte offset and four-byte element count.
- Safe resolution uses arena branding.
- Entire arena memory cannot safely become `&[u8]` because padding and uninitialized storage may exist.

## 1. Byte/range API

Add allocation-scoped initialized byte views.

Conceptual APIs:

```rust
arena.bytes_of(offset)
arena.bytes_of_slice(slice)
```

Requirements:

- zero-copy;
- exact allocation range;
- safe only for initialized bytes;
- no surrounding padding;
- checked byte lengths.

Raw used-memory inspection may return:

```rust
&[MaybeUninit<u8>]
```

not `&[u8]`.

Provide safe mutable bytes only for representations where every bit pattern is valid.

Provide an explicitly unsafe exact-allocation mutable-byte escape hatch for other types.

## 2. Byte-range descriptor

Introduce only if it simplifies multiple container APIs:

```rust
ByteRange32<'arena>
```

Target representation:

```text
offset u32
len    u32
```

Requirements:

- eight bytes;
- branded to arena;
- safe constructors only from validated arena allocations;
- unsafe raw constructor documented;
- exact byte range.

Likely uses:

- string payload;
- byte vectors;
- interned blobs;
- small/large representation transitions.

## 3. Allocation primitives

Expose enough internal/public allocation machinery for compact containers without making raw arena allocation unsound.

Needed operations may include:

- allocate raw aligned byte span;
- initialize exact byte span;
- copy between non-overlapping arena ranges;
- copy/move element sequences;
- validate range;
- obtain native temporary pointer/view.

Do not expose more unsafe surface than containers require.

## 4. CompactBox

Implement the simplest arena-owning abstraction first.

Target logical behavior:

```rust
let value = CompactBox::new_in(42, arena)?;
assert_eq!(*value.get(arena)?, 42);
```

Representation should be essentially one compact offset when type/layout permits.

Target metadata:

```text
offset u32
```

Do not store a native arena pointer in every `CompactBox`.

Support:

- immutable access;
- mutable access;
- conversion to compact offset where useful;
- zero-copy native borrow.

Define ownership semantics clearly: dropping the logical wrapper does not individually reclaim memory in a pure bump arena unless a recyclable allocator owns that allocation.

## 5. CompactVec

Target general metadata:

```text
offset   u32
len      u32
capacity u32
```

Target size:

12 bytes unless alignment/compiler representation forces a documented difference.

Required operations:

- `new_in`;
- `with_capacity_in`;
- `len`;
- `capacity`;
- `is_empty`;
- `push`;
- `pop`;
- `get`;
- `get_mut`;
- indexing where sound/idiomatic;
- slices;
- iteration;
- `reserve`;
- `truncate`;
- `clear`.

Growth:

1. compute new capacity safely;
2. allocate replacement arena span;
3. copy/move existing initialized values;
4. initialize new element;
5. commit metadata last.

Failure must preserve the old vector.

Do not silently depend on native `Vec<T>` internally for arena payload storage.

## 6. Growth policy

Default growth should balance memory and reallocation cost.

Do not blindly clone standard `Vec` growth if arena memory cannot reclaim old buffers.

A pure bump arena makes repeated geometric relocation leak previous capacity until arena reset.

Therefore choose one or more of:

- exact/small-step growth for monotonic backing;
- slab/recyclable backing for dynamic vectors;
- segmented/chunked representation;
- container-owned free-list reuse.

Measure before choosing.

This is a critical design point.

Do not ship a `CompactVec` growth strategy that causes pathological arena waste.

## 7. CompactString

Build on byte-oriented compact vector/storage primitives.

Required logical behavior:

- valid UTF-8 invariant;
- `new_in`;
- `from_str_in`;
- `len`;
- `is_empty`;
- `as_str`;
- `as_bytes`;
- push char/string where supported;
- clear;
- comparisons with `str`;
- zero-copy `&str` borrow.

General long-string representation should avoid 64-bit native pointers.

## 8. Small string/vector optimization

Add an inline representation for short payloads when it materially reduces allocations and cache misses.

Requirements:

- representation discriminant compactly encoded;
- no native pointer;
- no separate allocation for inline payload;
- transition inline -> arena-backed is failure-safe;
- exact inline capacity selected from layout measurements.

Possible string example:

```text
inline:
[tag/len + inline bytes]

large:
[tag + offset + len + capacity]
```

Do not choose a representation that makes common long-string metadata dramatically larger than the non-SSO representation without evidence.

## 9. Compact option/reference

Provide compact nullable reference behavior using offset zero where possible.

Goal:

```text
Option-like compact reference
= 4 bytes
```

Do not rely on Rust enum niche optimization unless the physical representation is explicitly verified and stable enough for the intended API.

A dedicated compact option wrapper is acceptable.

## 10. Slab

Implement a dense arena-local slab for repeated objects.

Possible representation:

```text
storage range
free-list head
len
capacity
generation metadata if needed
```

Free slots may store compact next-free indices.

Requirements:

- slot reuse;
- deterministic exhaustion;
- safe occupied/vacant distinction;
- no reads from vacant slots;
- stale handles rejected if handle reuse would otherwise be unsafe.

Use generation counters only if logically required; avoid per-entry overhead when ownership/lifetimes make stale handles impossible.

## 11. Interning

Implement optional immutable interning for:

- `str`;
- `[u8]`.

API should return compact handles.

Canonical payload must be arena-owned.

Do not force interning into ordinary `CompactString`.

Measure metadata overhead.

## 12. Drop/non-Copy strategy

Do not simply remove `T: Copy` everywhere.

Classify values:

### Plain arena values

Can be copied/moved bitwise safely and need no destructor.

### Compact-owning values

Own only arena-relative resources and can use explicit compact lifecycle semantics.

### Native Drop values

May own OS handles/native heap/native pointers and require ordinary Rust destruction.

Containers must not silently leak native-drop semantics.

If broad generic non-Copy support cannot be made sound within this milestone, constrain generic containers and document it rather than inventing an incomplete destructor registry.

## 13. Cache locality

Prefer contiguous payloads and compact headers.

Avoid pointer-heavy linked metadata for containers unless necessary.

For slabs, keep occupancy metadata compact.

For strings, keep length/tag close to inline bytes.

For iteration, expose contiguous native slices where representation permits.

## Tests

Required:

- exact representation sizes;
- allocation failure rollback;
- push/pop;
- growth;
- zero length;
- maximum metadata boundaries;
- UTF-8 correctness;
- inline/large string transition;
- zero-copy `&str`;
- zero-copy slices;
- compact optional null behavior;
- slab reuse;
- stale handle behavior;
- interning deduplication;
- byte access initialization safety;
- Miri coverage for moves/copies/raw byte handling.

## Non-goals

Do not implement:

- compiler integration;
- proc macros;
- prelude;
- automatic layout rewriting;
- automatic hot/cold detection;
- thread-safe mutation;
- general-purpose native allocator replacement.

## Handoff

Report:

- final representations and `size_of` values;
- growth strategy;
- reclaim/reuse model;
- changed files;
- commit SHA;
- tests;
- unsafe invariants;
- unresolved limitations.

Macro/facade work must consume these public interfaces rather than reaching into private allocation internals.
