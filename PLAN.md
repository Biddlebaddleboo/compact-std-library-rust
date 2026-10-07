# PLAN.md — V2.4 aggressive 32-bit cage architecture

## Baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`  
Branch: `main`  
Verified baseline commit: `4fa719265cea1cfd654991753b2a4b826d94adcf`  
Baseline version: `2.3.0`

V2.4 is intentionally allowed to be source- and representation-breaking.

There is no V2.3 application compatibility requirement and no migration requirement. V2.3 is an architectural checkpoint only.

Do not preserve complexity solely to retain V2.3 APIs or layouts.

## Objective

V2.4 must turn the V2.3 process-wide cage into an aggressively compact, internally simple 32-bit-addressed object model.

The primary invariant is:

> Every retained address, owner, link, descriptor, or reference that is part of the cage-aware data model must be representable using 32-bit cage-relative addressing. Native 64-bit pointers may exist only in the process runtime and as temporary machine addresses/borrows during execution or native/FFI interaction.

V2.4 should deliberately use concentrated `unsafe` Rust where that removes lifetime plumbing, identity metadata, allocation registries, native pointer storage, or other structural complexity.

Architecture-specific intrinsics and assembly are permitted where they materially simplify a primitive or provide a clearly superior implementation, but must not replace the Rust-level ownership/safety model.

The desired architecture is:

```text
safe application API
        |
compact collections / generated types
        |
small aggressively-unsafe cage kernel
        |
u32 owners / offsets / ranges
        |
one process-wide cage
```

The process itself remains a normal 64-bit Rust process.

## Hard invariants

V2.4 must satisfy all of the following.

### Process and ABI

- `target_pointer_width` remains 64.
- `usize`/`isize` remain native-width scalar types.
- ordinary Rust references remain native pointers.
- libc, syscalls, FFI and third-party dependencies remain normal platform ABI.
- there is exactly one process-wide compact cage.
- the cage backing address is stable for process lifetime.
- no public cage teardown/reinitialization exists.
- cage exhaustion is explicit; there is no silent native-heap fallback for cage storage.

### Retained cage addressing

The following must never retain a native pointer to cage storage:

- `CageAllocation<T>`
- `CompactBox<T>`
- `CompactVec<T>`
- `CompactString`
- `CompactBytes`
- `CompactVecDeque<T>`
- hash collections
- slab storage and ordinary object-graph links
- scratch descriptors
- generated `#[compact]` values
- frozen retained representations
- internal allocator free-list links

All retained cage addressing must be `u32`-based.

Temporary native addresses are allowed only after resolution:

```text
stored:
    offset32

during access:
    cage_base + offset32 -> native *const T / *mut T / &T / &mut T

after access:
    no native pointer retained in cage-aware state
```

### Unsafe policy

Unsafe is not a last-resort restriction for V2.4.

Use unsafe when it materially eliminates representation or architectural complexity.

Unsafe must nevertheless be:

- concentrated in a small number of low-level modules;
- documented with explicit invariants;
- hidden behind safe APIs where a sound safe API exists;
- exercised by Miri, property tests, fuzzing, panic/drop tests and concurrency tests.

Do not scatter independent pointer arithmetic implementations throughout collections.

### Assembly/intrinsics policy

`core::arch` intrinsics, `asm!`, or architecture-specific modules may be introduced when they:

1. make a low-level primitive substantially simpler; or
2. express an operation the compiler cannot express cleanly; or
3. materially improve a hot compact-memory primitive without complicating correctness.

Prefer, in order:

1. ordinary Rust;
2. unsafe Rust;
3. architecture intrinsics;
4. inline/global assembly.

Assembly must not be used for:

- ownership state machines;
- allocator bookkeeping when Rust expresses it clearly;
- drop/unwind handling;
- bounds validation;
- concurrency synchronization merely to avoid Rust atomics/mutexes.

Every architecture-specific implementation must have a portable Rust implementation unless the crate explicitly limits the supported targets.

Initial supported optimized targets may be:

- `x86_64`
- `aarch64`

Do not make V2.4 dependent on assembly simply because it is allowed.

---

# Verified repository facts

At baseline `4fa719265cea1cfd654991753b2a4b826d94adcf`:

## Cage runtime

`crates/compact_backend_std/src/cage.rs` contains:

- `CompactRuntime`
- `CageConfig`
- global `CAGE: OnceLock<CageState>`
- `CageState`
- `Allocator`
- `AllocationHeader`
- `CageAllocation<T>`

The cage base is retained once in `CageState`.

`CageAllocation<T>` is already a four-byte owner consisting of a nonzero `u32` offset plus zero-sized marker state.

Temporary native pointers are formed by resolving that offset against `CageState::base()`.

## Current allocation header

The current V2.3 header is:

```rust
#[repr(C)]
struct AllocationHeader {
    magic: u64,
    block_start: u32,
    prefix: u32,
    block_len: u64,
    capacity: u32,
    initialized: u32,
}
```

On ordinary 64-bit targets this is approximately 32 bytes.

V2.2 used a 16-byte four-`u32` header.

The V2.4 implementation must not retain the V2.3 32-byte header merely for convenience.

## Current native allocator metadata

`Allocator` currently contains:

```rust
cursor: usize,
live_bytes: usize,
free: BTreeMap<usize, usize>,
live_offsets: HashSet<u32>,
```

The `BTreeMap` and `HashSet` allocate native pointer-based structures outside the cage.

They were useful for getting V2.3 running, but conflict with the intended V2.4 architecture.

## Compact core

`crates/compact_core/src/allocation.rs` defines unsafe trait `CompactValue`.

Its current contract already forbids native pointers/references in compact values and requires relocatability.

Primitive scalars, arrays, `MaybeUninit`, `Option`, `Result`, and tuples have implementations.

`crates/compact_core/src/offset.rs` provides the four/eight-byte offset descriptors.

Current core layout tests verify:

- `Offset32<T>` = 4 bytes
- `OffsetSlice32<T>` = 8 bytes
- `ByteRange32` = 8 bytes

## Frozen storage

`crates/compact_frozen/src/storage.rs` currently stores per-descriptor graph identity:

```text
FrozenVec<T>:
    offset   u32
    len      u32
    graph_id u32

FrozenString:
    offset   u32
    len      u32
    graph_id u32
```

These retained frozen descriptors are therefore 12 bytes.

`FrozenBuilder` currently constructs into a native `Vec<u64>` before copying the graph into one cage allocation.

Graph identity prevents descriptors from one graph being interpreted through another graph.

V2.4 must preserve the safety property, but not necessarily the 12-byte representation.

## Current validation defect

`.github/workflows/miri.yml` is stale.

The V2.3 commit deleted test targets including:

- `compact_collections --test containers`
- `compact_collections --test arena_traits`
- `compact_collections --test hash_map`
- `compact_std --test serde_direct`

but the workflow still invokes them.

The GitHub Actions run for baseline commit `4fa719...` failed with:

```text
error: no test target named `containers` in `compact_collections` package
```

The core Miri step passed, but later unsafe-code checks did not run.

This must be repaired as part of V2.4.

## Test coverage regression

The V2.3 rewrite correctly removed obsolete arena compatibility tests, but also removed substantial behavioral coverage covering:

- container panic/drop behavior;
- hash collisions/removal;
- property testing;
- OS/path behavior;
- several unsafe edge cases.

`crates/compact_collections/tests/cage_collections.rs`,
`crates/compact_backend_std/tests/integration.rs`, and
`crates/compact_std/tests/v2_3.rs`
replace part, but not all, of that behavioral coverage.

V2.4 must restore the useful behavioral tests without restoring arena compatibility.

---

# Implementation scope

Inspect these files/symbols first and avoid broad repository searching unless a moved symbol, compilation failure, test failure, or correctness dependency requires it.

## Core representation

### `crates/compact_core/src/allocation.rs`

Inspect first:

- `CompactValue`
- primitive/aggregate implementations

Strengthen its documented invariant.

Do not require safe Rust to prove facts that are intentionally owned by the unsafe cage kernel.

Manual `unsafe impl CompactValue` remains an explicit safety boundary.

Do not permit retained native pointers/references in valid cage values.

### `crates/compact_core/src/offset.rs`

Inspect:

- `Offset32<T>`
- `OffsetSlice32<T>`
- raw constructors
- null handling

Retain:

```text
Offset32<T>       4 bytes
OffsetSlice32<T>  8 bytes
```

Add or refine low-level constructors only when necessary for the cage kernel.

Do not add native base pointers, arena IDs, runtime pointers or allocation IDs.

### `crates/compact_core/src/bytes.rs`

Inspect `ByteRange32`.

Target remains 8 bytes.

### `crates/compact_core/src/abi.rs`

Update ABI/version constants for V2.4.

Keep the compact address domain explicitly 32-bit.

### `crates/compact_core/src/error.rs`

Add errors only where V2.4 introduces a real failure mode.

Do not encode internal allocator state using public error variants unless externally observable.

---

# Phase 1 — define the V2.4 unsafe kernel boundary

Primary file:

`crates/compact_backend_std/src/cage.rs`

Prefer splitting only if it materially improves auditability, for example proposed internal modules:

```text
src/cage.rs
src/cage/allocator.rs
src/cage/resolve.rs
```

Do not split merely to reduce line count.

Create one canonical implementation for:

- cage state access;
- offset-to-native-pointer resolution;
- allocation;
- release;
- resize;
- initialized-length updates;
- raw element movement;
- range validation.

Collections must consume these primitives instead of reproducing pointer arithmetic.

Conceptual internal primitives may resemble:

```rust
unsafe fn ptr_from_offset<T>(offset: NonZeroU32) -> *mut T;
unsafe fn slice_from_offset<T>(offset: NonZeroU32, len: usize) -> *mut T;

fn allocate<T: CompactValue>(capacity: u32) -> Result<CageAllocation<T>>;
fn try_resize<T: CompactValue>(owner: &mut CageAllocation<T>, capacity: u32)
    -> Result<bool>;
fn release<T: CompactValue>(owner: &mut CageAllocation<T>);
```

Names may differ if the existing API permits a smaller surface.

Do not expose arbitrary safe offset resolution capable of producing unbounded lifetimes.

Where an unsafe resolver returns a reference with caller-chosen lifetime, document that this is an internal unsafe primitive and require the caller to bind it to an owning borrow.

---

# Phase 2 — replace native allocator registries with intrusive cage metadata

Primary file:

`crates/compact_backend_std/src/cage.rs`

Remove:

```rust
BTreeMap<usize, usize>
HashSet<u32>
```

from `Allocator`.

Do not replace them with another native heap collection.

Target allocator process state should contain only small scalar/native synchronization state, conceptually:

```text
cursor/live-tail: u32
free_head:        u32
live_bytes:       u32 or equivalent
mutex
```

The mutex itself is allowed to be native runtime state because it is one process-level synchronization object, not retained per cage object.

## Intrusive free ranges

Released cage blocks must contain their own free-list metadata.

Use cage-relative `u32` links.

A free block may conceptually contain:

```text
block_start
+----------------+
| next: u32      |
| len:  u32      |
| optional prev  |
| optional flags |
+----------------+
```

Select singly-linked or doubly-linked organization based on the smallest correct implementation.

Prefer a sorted-by-offset intrusive list if it keeps coalescing simple.

Required behavior:

- first-fit or similarly deterministic allocation;
- split reusable blocks when the remainder can form a valid free node;
- do not create unusably small fragments;
- coalesce adjacent free blocks;
- contract the tail when the highest free block reaches the allocation cursor;
- maintain correct live-byte accounting;
- no native allocation during normal free-list insertion/removal.

Do not introduce a complex segregated allocator unless deterministic tests demonstrate the simple intrusive allocator is inadequate.

V2.4 favors representation simplicity over allocator sophistication.

---

# Phase 3 — shrink the live allocation header

Primary file:

`crates/compact_backend_std/src/cage.rs`

The current 32-byte V2.3 header is not acceptable as the final V2.4 common header without a demonstrated correctness requirement.

Target common header:

**16 bytes or less.**

Preferred starting point is the proven V2.2-style representation:

```rust
#[repr(C)]
struct AllocationHeader {
    block_len: u32,
    prefix: u32,
    capacity: u32,
    initialized: u32,
}
```

Exact field meanings may change.

Because the entire cage address domain is 32-bit:

- `block_len` does not need to be `u64`;
- block offsets do not need `usize`;
- ordinary allocation capacity metadata should use `u32`;
- an 8-byte magic value must not be retained in every allocation merely for defensive convenience.

If state bits can be packed safely into alignment-guaranteed low bits without complicating the implementation, that is allowed.

Do not make bit packing mandatory if four plain `u32`s are clearer.

If a field can be derived safely from the owner offset and other header fields, eliminate it.

The header must still support:

- finding the physical block extent;
- locating the data payload;
- capacity;
- initialized element count;
- in-place resize;
- exact release;
- panic-safe drop.

Required size assertion:

```rust
size_of::<AllocationHeader>() <= 16
```

If implementation correctness proves 16 bytes insufficient, stop and document the exact reason before accepting a larger layout.

---

# Phase 4 — preserve four-byte ownership and remove redundant identity

Primary file:

`crates/compact_backend_std/src/cage.rs`

`CageAllocation<T>` must remain:

```text
4 bytes
```

and:

```text
Option<CageAllocation<T>> = 4 bytes
```

Do not add:

- allocator pointer;
- cage pointer;
- allocation ID;
- generation;
- native handle;
- graph identity.

Ownership validity comes from:

- private construction;
- non-`Copy` ownership;
- Rust move semantics;
- authoritative cage header/free-list state;
- unsafe kernel invariants.

Do not keep the V2.3 `live_offsets: HashSet<u32>` merely to validate legitimate owners.

Arbitrary raw offset validation is a different problem from owner validity.

Stale-handle generations remain allowed only for abstractions that genuinely support independently copied handles, especially `CompactSlab`.

---

# Phase 5 — aggressively centralize pointer resolution

Primary files:

- `crates/compact_backend_std/src/cage.rs`
- `crates/compact_core/src/offset.rs`

All ordinary compact access should resolve:

```text
native = global_cage_base + zero_extend(offset32)
```

in one audited implementation.

Collections must not retain the resulting pointer.

Preferred pattern:

```rust
pub fn as_slice(&self) -> &[T] {
    // small audited unsafe bridge
}
```

rather than safe API designs that force callers to carry:

- runtime handles;
- resolver tokens;
- arena objects;
- identity wrappers.

Native references are temporary execution artifacts and do not violate the V2.4 retained-layout invariant.

Ensure reference lifetimes are always tied to the owner/view borrow.

Do not expose APIs that allow:

```rust
Offset32<T> -> &'static T
```

through safe code.

---

# Phase 6 — collection representation audit

Inspect first:

- `crates/compact_collections/src/boxed.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/string.rs`
- `crates/compact_collections/src/compact_bytes.rs`
- `crates/compact_collections/src/deque.rs`
- `crates/compact_collections/src/hash_map.rs`
- `crates/compact_collections/src/slab.rs`
- `crates/compact_collections/src/small.rs`
- `crates/compact_collections/src/bitvec.rs`
- `crates/compact_collections/src/intern.rs`
- `crates/compact_collections/src/os_path.rs`

For each retained type, audit every field.

Classify every field as:

```text
scalar
inline payload
u32 cage address
u32 cage length/index
legitimate stale-handle generation
```

Any retained native pointer/reference is a V2.4 correctness failure.

## `CompactBox<T>`

Hard target:

```text
4 bytes
```

Its only retained ownership state should be the cage owner offset.

## `CompactVec<T>`

Preferred hard target:

```text
4 bytes
```

Keep capacity/initialized length in allocation metadata if this does not create excessive access complexity.

Normal API should remain:

- `new`
- `with_capacity`
- `len`
- `capacity`
- `reserve`
- `push`
- `pop`
- `get`
- `get_mut`
- `as_slice`
- `as_mut_slice`
- iterators
- `truncate`
- `clear`
- `shrink_to_fit`

Allocation-bearing methods may remain fallible.

Do not add an allocator/runtime parameter.

## `CompactVecDeque<T>`

Target approximately:

```text
owner u32
head  u32
len   u32
= 12 bytes
```

Capacity should remain allocation metadata where practical.

## `CompactString`

Preserve or improve inline representation.

Heap-backed state must not contain a native pointer.

Re-evaluate the current inline threshold after the smaller allocation header is implemented.

Do not optimize the inline threshold before header/layout correctness is complete.

## `CompactBytes`

Same policy as `CompactString`.

Re-evaluate SSO threshold after V2.4 header size is fixed.

## `CompactHashMap` / `CompactHashSet`

Keep table/control storage cage-backed.

Native `RandomState` hashing state is acceptable only if it is scalar/native algorithm state and not a retained pointer to cage storage.

Do not replace randomized hashing with a weaker design merely to make every scalar 32-bit.

Audit control-table scans as potential intrinsic/assembly candidates only after the normal implementation is correct.

## `CompactSlab<T>`

Keep generation counters.

A slab handle legitimately needs stale-handle protection.

Do not generalize slab generation overhead into every cage allocation.

Evaluate whether `slab_id` is still required once there is one process-wide cage.

If cross-slab confusion remains possible and memory-safety relevant, keep the smallest correct identity mechanism.

If it is merely defensive and safe APIs can bind handles more cheaply, simplify it.

Document the decision.

## SmallVec / ring / interner / bitvec / OS/path

Convert every retained storage link to cage-relative state.

Do not reintroduce native heap ownership except for explicit native boundary objects.

---

# Phase 7 — strengthen `CompactValue` as the cage-safe representation contract

Primary file:

`crates/compact_core/src/allocation.rs`

Retain the unsafe trait unless a rename clearly improves the implementation enough to justify churn.

The trait contract must explicitly require:

- no retained native pointer/reference fields;
- no self-reference;
- no address-sensitive state;
- no pinning requirement;
- relocation through raw read/write is valid;
- destruction remains valid while cage runtime exists;
- any nested ownership is itself cage-safe;
- scalar integers are values, not disguised native addresses relied upon for correctness.

Do not attempt impossible complete compile-time enforcement.

The macro/derive layers should enforce the rule structurally where they control field generation.

Manual unsafe implementations remain the caller's responsibility.

Add compile-time/layout tests that cage-native library types satisfy this contract.

---

# Phase 8 — frozen representation redesign

Primary file:

`crates/compact_frozen/src/storage.rs`

Current 12-byte descriptors containing `graph_id` are transitional V2.3 design.

V2.4 target for retained frozen descriptors:

```text
typed slice:
    offset32 + len32 = 8 bytes

string:
    offset32 + len32 = 8 bytes

bytes:
    offset32 + len32 = 8 bytes
```

However, do not simply delete `graph_id` and make cross-graph misuse possible.

Preserve the safety guarantee by separating:

1. compact retained representation; and
2. temporary graph-bound access views.

Preferred design:

```text
retained frozen bytes:
    FrozenSliceRepr<T> { offset: u32, len: u32 }
    FrozenStrRepr      { offset: u32, len: u32 }

temporary safe access:
    graph-bound view carrying the borrow/context required to resolve it
```

A temporary graph view may contain a native Rust reference because it is not retained cage representation.

The safe public API must make it impossible to interpret arbitrary representation bytes against the wrong graph.

Possible implementation shape:

```rust
struct FrozenSlice32<T> {
    offset: u32,
    len: u32,
    marker: PhantomData<T>,
}

struct FrozenView<'g, T> {
    graph: &'g FrozenGraph<...>,
    repr: T,
}
```

Exact public naming may differ.

Avoid reintroducing arena-style lifetime plumbing into ordinary application code.

The graph-bound lifetime may exist in temporary access types while stored graph data remains lifetime-free and 32-bit.

## Frozen builder

Current native construction buffer is allowed initially if:

- it is temporary;
- final retained graph is fully cage-backed;
- the design does not leak native addresses into frozen representation.

Investigate whether direct cage construction materially simplifies V2.4 after the allocator redesign.

Do not force direct construction if it creates relocation/patching complexity.

One native temporary build buffer is preferable to a complex relocation system unless measurements or correctness show otherwise.

---

# Phase 9 — scratch region

Primary file:

`crates/compact_backend_std/src/scratch.rs`

Scratch remains one block in the same cage.

Retained scratch descriptors use offsets/counts only.

The stack `ScratchRegion` controller may hold ordinary native temporaries if they are not stored into cage-aware values, but prefer u32 cursor/capacity state where practical.

Continue restricting typed scratch values where destructor handling is not supported.

Do not create a second cage or independent address domain.

---

# Phase 10 — macros and generated layouts

Inspect:

- `crates/compact_macros/src/compact.rs`
- `crates/compact_macros/src/compact_deserialize.rs`
- `crates/compact_macros/src/lib.rs`

`#[compact]` generated retained state must satisfy the V2.4 address invariant recursively.

Generated code must not store native references or pointers.

String/collection fields must use compact/cage-native representations.

Generated accessors should remain normal-looking:

```rust
record.name()
record.set_name("worker")?
```

Do not restore arena parameters or lexical arena rewriting.

For generated structures that expose graph/frozen data, use temporary graph-bound access types rather than adding graph identity to every retained descriptor.

Preserve:

- bounded integer packing;
- booleans;
- enums;
- SoA where already supported.

Do not broaden macro feature scope unrelated to V2.4.

---

# Phase 11 — Serde

Inspect:

- `crates/compact_serde/src/deserialize.rs`
- `crates/compact_serde/src/json.rs`
- `crates/compact_serde/src/toml.rs`
- derive integration in `compact_macros`

Maintain direct cage construction:

```rust
from_str(...)
from_slice(...)
```

No allocator argument.

Partial construction must release every completed cage allocation on:

- parse error;
- allocation failure;
- user visitor failure;
- panic where Rust unwinding permits cleanup.

Restore deterministic tests for partial construction.

Do not add V2.3 migration adapters.

---

# Phase 12 — FFI/native boundary

Inspect existing FFI surfaces in `compact_std` and collections.

Required rule:

> A compact offset is never a C/Swift/JNI pointer.

FFI borrowing:

```text
cage offset
   ↓ resolve
temporary native pointer
   ↓ callback/native operation
borrow ends
```

Retained foreign ownership must use an explicit native exported buffer or a copied/exported representation.

Do not allow a foreign caller to keep a pointer into cage memory without an explicit owner/lifetime protocol.

---

# Phase 13 — selective architecture-specific primitives

Do this only after portable correctness is established.

Candidate files may be proposed under:

```text
crates/compact_core/src/arch/
crates/compact_collections/src/arch/
```

only if useful.

Potential candidates:

- packed bit extraction/insertion;
- control-byte scans in hash tables;
- wide equality/search over compact bytes;
- other tiny hot primitives where intrinsics clearly reduce code or branches.

Preferred pattern:

```rust
#[cfg(target_arch = "x86_64")]
fn primitive(...) { ... }

#[cfg(target_arch = "aarch64")]
fn primitive(...) { ... }

#[cfg(not(...))]
fn primitive(...) { portable implementation }
```

Use intrinsics before `asm!` where both are equivalent.

For inline assembly:

- document input/output registers and clobbers;
- do not depend on undocumented ABI behavior;
- test against the portable implementation;
- keep unsafe assembly blocks very small;
- no hidden persistent pointer state;
- no architecture-specific behavior visible in serialized/cage layout unless explicitly documented.

Assembly is optional.

Do not add assembly merely to satisfy this plan.

---

# Phase 14 — concurrency model

The one cage is an address domain, not a single-thread restriction.

Allocator metadata remains synchronized.

Reads of live allocations should not require the allocator mutex.

`CageAllocation<T>` should receive `Send`/`Sync` behavior only when sound for `T`.

Required tests include:

- allocate on thread A, move owner to B, drop on B;
- concurrent allocations/frees;
- allocator free-list coalescing under contention;
- immutable sharing of sound values;
- mutation requires ordinary Rust exclusivity;
- frozen graph parallel reads;
- no stale native pointer cached across relocation.

Do not introduce lock-free allocator complexity in V2.4 unless the mutex is proven to be an actual problem.

---

# Phase 15 — panic, drop and relocation correctness

This is mandatory because V2.4 relies more heavily on unsafe.

Required invariants:

## Drop

For every owner/container:

- every initialized element is dropped exactly once;
- uninitialized elements are never dropped;
- allocation is released exactly once;
- nested cage owners are dropped before their containing storage is released.

## Panic during element destruction

Before invoking `Drop` on an element:

- remove it from the authoritative initialized/live state first.

If the destructor panics:

- no later operation may double-drop it;
- remaining initialized elements must remain accounted for;
- allocator metadata must not claim a still-owned block is free.

Use guards where required.

## Relocation

For grow/move:

1. acquire destination capacity first;
2. preserve old state until failure is no longer possible where practical;
3. transfer ownership exactly once;
4. clear source initialization state before any path that could double-drop;
5. publish destination initialization consistently;
6. release old allocation only after successful transfer.

Do not retain native pointers across any operation that can relocate cage storage within the cage.

The cage base itself never moves.

---

# Phase 16 — restore and expand validation coverage

## Fix CI first

Update:

`.github/workflows/miri.yml`

Remove every stale V2.2 target.

Point Miri at actual V2.4 tests.

Expected eventual commands should include real targets such as:

```sh
cargo miri test -p compact_core
cargo miri test -p compact_backend_std --test integration
cargo miri test -p compact_collections --test cage_collections
cargo miri test -p compact_std --all-features --test v2_4
```

Add separate focused test binaries if Miri runtime becomes excessive.

Do not reference deleted arena tests.

## Restore behavioral coverage

Create or expand deterministic cage-era tests for behavior previously covered by removed V2.2 tests.

Required areas:

### allocator

- first allocation;
- exact-fit reuse;
- split free range;
- adjacent coalescing;
- both-side coalescing;
- tail contraction;
- alignment;
- ZST;
- near-u32 limits without actually allocating enormous memory;
- capacity/length overflow;
- failed resize leaves original allocation intact;
- in-place grow;
- in-place shrink;
- moved grow;
- allocation exhaustion;
- one-time runtime initialization.

### ownership

- four-byte owner;
- four-byte optional owner;
- move does not double-drop;
- drop count exactness;
- nested owners;
- panic during destructor;
- cross-thread drop.

### collections

- Vec push/pop/reserve/truncate/clear/shrink;
- Box;
- String UTF-8 boundaries;
- Bytes inline/heap transitions;
- VecDeque wrap/grow/make-contiguous;
- ring overwrite/drop;
- SmallVec promotion;
- bitvec boundaries;
- interner canonicalization;
- OS/path exact round-trip;
- hash collision probing;
- tombstones;
- hash panic where relevant;
- slab stale generation behavior.

### frozen

- exact descriptor sizes;
- graph-bound access;
- cannot safely mix representations between graphs;
- UTF-8 validation;
- typed alignment;
- parallel reads;
- root and nested descriptors.

### Serde

- empty values;
- large values;
- Unicode;
- nested compact values;
- malformed input;
- allocation failure;
- partial cleanup;
- rename/default/skip/deny-unknown-fields behavior.

---

# Phase 17 — property testing and fuzzing

Restore property coverage removed by V2.3 where behavior still exists.

Inspect/update:

- `fuzz/fuzz_targets/allocator_ops.rs`
- `fuzz/fuzz_targets/chunk_bytes.rs`
- `fuzz/fuzz_targets/frozen_graph_access.rs`
- `fuzz/fuzz_targets/hash_collisions.rs`
- `fuzz/fuzz_targets/packed_bits.rs`
- `fuzz/fuzz_targets/serde_json_input.rs`
- `fuzz/fuzz_targets/serde_toml_input.rs`
- `fuzz/fuzz_targets/common/mod.rs`

Allocator fuzzing must model:

- allocate;
- free;
- resize;
- split;
- reuse;
- coalescing;
- tail release;
- randomized alignments;
- initialized lengths.

Maintain a simple reference model independent of the intrusive implementation.

Assert after each operation:

- live blocks do not overlap;
- free blocks do not overlap;
- free + live ranges remain in cage bounds;
- free-list links are valid u32 offsets;
- live byte accounting is correct;
- no block is simultaneously live and free.

For packed/architecture-specific primitives, differential-test optimized implementation against portable Rust.

---

# Phase 18 — representation assertions

Add hard layout tests.

Required:

```text
Offset32<T>                    4 bytes
OffsetSlice32<T>               8 bytes
ByteRange32                    8 bytes
CageAllocation<T>              4 bytes
Option<CageAllocation<T>>      4 bytes
CompactBox<T>                  4 bytes
CompactVec<T>                  4 bytes
AllocationHeader              <=16 bytes
```

Desired:

```text
CompactVecDeque<T>            ~12 bytes
retained frozen slice          8 bytes
retained frozen string         8 bytes
retained frozen byte range     8 bytes
```

Audit every cage-aware public type for native pointer-sized retained fields.

Add a documented final representation table to `ARCHITECTURE.md`.

Do not make `usize` scalars illegal merely because they are 64-bit; the prohibition concerns retained addressing/pointers, not all 64-bit scalar data.

---

# Phase 19 — unsafe audit

Before release, review every `unsafe` occurrence in:

- `compact_core`
- `compact_backend_std`
- `compact_collections`
- `compact_frozen`
- generated macro output

For each unsafe block verify:

1. what pointer/range is being constructed;
2. who owns the memory;
3. why it is live;
4. alignment;
5. initialization;
6. aliasing;
7. whether relocation can occur;
8. what lifetime binds the resulting reference;
9. what happens during panic/unwind.

Keep:

```rust
#![forbid(unsafe_op_in_unsafe_fn)]
```

where currently used.

An `unsafe fn` must still contain explicit unsafe blocks.

Update `SAFETY.md` to describe actual V2.4 invariants rather than aspirational ones.

---

# Phase 20 — final V2.4 benchmarking

Do not spend implementation time producing a V2.3 benchmark suite first.

Benchmark only after V2.4 representation and validation are stable.

Use existing examples where suitable:

```sh
cargo run --release -p compact_std --example runtime_bench
cargo run --release -p compact_std --example workloads_bench --features json,toml
```

Compare final V2.4 primarily against the recorded V2.2 baseline and native equivalents.

V2.3 may be included only if useful; lack of V2.3 benchmark data is not a blocker.

Measure at minimum:

- object wrapper sizes;
- per-allocation overhead;
- allocation/reuse throughput;
- fragmentation/high-water use;
- Vec growth;
- String/Bytes small and heap cases;
- VecDeque;
- maps/sets;
- path-heavy workload;
- Serde;
- frozen graphs;
- concurrent allocator workload;
- packed primitive throughput where architecture-specific code was added.

The main V2.4 success metric is memory representation and structural simplicity, not beating native Rust in every microbenchmark.

Do not revert compactness solely because one short-lived allocation microbenchmark favors the native allocator.

---

# Phase 21 — documentation and release

Update:

- `README.md`
- `ARCHITECTURE.md`
- `SAFETY.md`
- `BENCHMARKS.md`
- crate documentation
- workspace version

Set version to:

```text
2.4.0
```

Update compact ABI version where applicable.

Documentation must describe V2.4 as the current architecture only.

No V2.2/V2.3 migration guide is required.

Git history preserves prior designs.

Clearly document:

```text
normal Rust / dependencies:
    native 64-bit pointers

retained cage-aware state:
    u32 cage offsets

temporary cage access:
    native pointer/reference formed from cage_base + offset
```

Document that native references returned from cage values are temporary views and must not be retained beyond their Rust borrow.

---

# Explicit non-goals

V2.4 does **not**:

- change `target_pointer_width`;
- make normal Rust `&T` four bytes;
- make `usize` 32-bit;
- change the Linux/macOS/Windows ABI;
- implement x32;
- create a custom rustc target;
- replace ordinary `std::Vec`, `std::String`, or `std::Box` inside third-party dependencies;
- modify libc ABI;
- implement a moving GC;
- introduce multiple cages;
- support cage teardown/reinitialization;
- preserve V2.3 source compatibility;
- provide a V2.3 migration layer;
- make all third-party dependencies cage-aware.

Those belong to a possible future V3/toolchain effort.

---

# Security and safety concerns

The more aggressive unsafe implementation makes the following release blockers:

- forged offset interpreted as initialized value;
- out-of-bounds offset arithmetic;
- alignment error;
- arithmetic overflow;
- free-list cycle/corruption;
- overlapping live allocations;
- double free;
- double drop;
- use after release;
- native pointer retained across relocation;
- cross-graph frozen descriptor confusion;
- unsound `Send`/`Sync`;
- panic leaving untracked initialized values.

Do not trade these invariants for smaller representation.

Representation metadata whose sole purpose is compensating for an avoidable safe-Rust abstraction may be removed.

Metadata preventing a genuine independently reproducible stale-handle or ownership error must remain in the smallest correct form.

---

# Acceptance criteria

V2.4 is complete only when all of the following are true.

## Architecture

- one process-wide cage;
- one retained native cage base in process runtime;
- no native pointer in cage-aware retained ownership or graph representation;
- intrusive cage-relative free metadata;
- no native `BTreeMap`/`HashSet` allocator registry;
- common allocation header <=16 bytes;
- four-byte owner remains intact;
- frozen retained descriptors reduced toward 8-byte offset/length representation without sacrificing safe graph identity.

## API

Typical code remains close to ordinary Rust:

```rust
let mut values = compact_std::Vec::new();
values.push(10)?;

let mut name = compact_std::String::from_str("abc")?;
name.push_str("def")?;

consume(name.as_str());
consume_slice(values.as_slice());
```

No:

```text
arena argument
runtime handle argument
resolver token
arena! wrapper
*_in methods
```

for ordinary usage.

## Validation

- GitHub Miri workflow is green and references existing tests;
- full workspace tests pass;
- all-features tests pass;
- restored behavioral/property coverage passes;
- fuzz targets compile and relevant bounded fuzz runs pass;
- concurrency stress passes;
- layout assertions pass;
- no stale V2.2/V2.3 arena compatibility tests remain.

## Representation

Hard:

```text
CageAllocation<T>          4 B
Option<CageAllocation<T>>  4 B
CompactBox<T>              4 B
CompactVec<T>              4 B
Offset32<T>                4 B
OffsetSlice32<T>           8 B
AllocationHeader          <=16 B
```

Frozen stored slice/string/byte descriptors should be 8 B unless a documented safety proof demonstrates that additional retained state is unavoidable.

## Final audit

Search the final tree for:

```text
BTreeMap
HashSet
*const
*mut
NonNull
usize
graph_id
allocation_id
arena
Arena
_in(
```

This search is an audit aid, not an automatic ban.

For each occurrence in cage-aware implementation code, determine whether it is:

- a legitimate temporary/native/runtime concern;
- scalar application data;
- necessary stale-handle metadata;
- or a retained native-address/legacy artifact that must be removed.

---

# Validation commands

At minimum:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Miri after workflow repair:

```sh
cargo +nightly miri setup
cargo +nightly miri test -p compact_core
cargo +nightly miri test -p compact_backend_std --test integration
cargo +nightly miri test -p compact_collections --test cage_collections
cargo +nightly miri test -p compact_std --all-features --test v2_4
```

Use the actual final test target names if implementation splits them differently.

Build fuzz targets:

```sh
cargo check --manifest-path fuzz/Cargo.toml
```

Run bounded allocator/representation fuzzing as appropriate before release.

Final benchmark pass:

```sh
cargo run --release -p compact_std --example runtime_bench
cargo run --release -p compact_std --example workloads_bench --features json,toml
```

---

# Final-diff checklist

Before calling V2.4 complete:

- [ ] current branch still descends from the intended V2.3 baseline or relevant changes were reconciled;
- [ ] version is `2.4.0`;
- [ ] no V2.3 compatibility shim was added;
- [ ] no arena architecture returned;
- [ ] allocator no longer depends on native heap maps/sets;
- [ ] free-list links are `u32`;
- [ ] allocation header <=16 bytes;
- [ ] cage owner remains 4 bytes;
- [ ] compact Vec remains 4 bytes;
- [ ] all retained cage addresses are 32-bit;
- [ ] temporary native pointer formation is centralized;
- [ ] no native cage pointer is stored in compact values;
- [ ] frozen cross-graph safety remains sound after descriptor compaction;
- [ ] slab stale-handle protection remains correct;
- [ ] panic/drop behavior is tested;
- [ ] Miri workflow is green;
- [ ] removed V2.2 behavioral tests have cage-era replacements where behavior still matters;
- [ ] property tests/fuzz targets cover intrusive allocator invariants;
- [ ] architecture-specific code has a portable reference implementation;
- [ ] every assembly block, if any, is separately documented/tested;
- [ ] `SAFETY.md` matches implementation;
- [ ] `ARCHITECTURE.md` includes exact final layout table;
- [ ] final benchmarks describe V2.4 rather than claiming unmeasured V2.3 results;
- [ ] final diff contains no `PLAN*.md`.

---

# Execution handoff

Implement this plan from the latest `main`.

1. Verify `main` has not advanced in a way that changes the cage architecture.
2. Read this entire `PLAN.md` before editing.
3. Fix the validation plumbing early so unsafe changes can be tested continuously.
4. Establish the V2.4 allocator/header/kernel representation before rewriting dependent collections.
5. Keep unsafe concentrated in the cage kernel and narrowly scoped container internals.
6. Port collections after the allocator interface is stable.
7. Redesign frozen retained descriptors only after the common offset/resolution model is stable.
8. Add architecture-specific intrinsics/assembly only after a correct portable implementation exists.
9. Run targeted tests continuously and the complete validation set before release.
10. Report changed files, tests, deviations from this plan, and unresolved assumptions.
11. Independently compare the final diff against every hard invariant and acceptance criterion above.
12. Resolve architectural contradictions centrally rather than silently adding metadata or compatibility layers.
13. Review the complete final diff.
14. Delete all `PLAN*.md` files.
15. Commit the implementation without planning files remaining.

Scope may expand only when compilation, moved symbols, failing tests, call-site compatibility, safety, or correctness requires it.

Do not reintroduce V2.2/V2.3 architecture to make implementation easier.
