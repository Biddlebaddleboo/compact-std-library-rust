# PLAN.md — V2.2 near-drop-in compact Rust environment

## Objective

Evolve `compact-std-library-rust` from the V2.1.0 compact collection/runtime foundation into a near-drop-in source-level alternative to common `std` allocation patterns while preserving the documented V2.1.0 contract exactly.

The target user experience is ordinary Rust code inside an explicit arena scope:

```rust
arena!(arena, {
    let mut names = vec!["alice".to_string(), "bob".to_string()];
    let mut counts = HashMap::new();

    for name in &names {
        counts.insert(name.clone(), name.len())?;
    }

    let line = format!("{} names", names.len())?;
});
```

with the macro and facade supplying arena-aware allocation underneath.

The implementation must also support application patterns verified in:

- `Biddlebaddleboo/local-service-orchestrator`
  - immutable configuration shared among tasks;
  - `HashMap`/`HashSet`;
  - paths and OS strings;
  - Serde/TOML/JSON;
  - short-lived scratch transformations;
  - `Arc`-shared read-only state.

- `Biddlebaddleboo/rideshare-bot`
  - retained native log rings;
  - `VecDeque`;
  - nested byte buffers;
  - chunk reassembly;
  - bounded caches;
  - packet construction;
  - C/JNI/Swift FFI boundaries.

The mutable V2.1.0 arena must remain single-owner and must not become implicitly `Send` or `Sync`.

V2.2 may add APIs, types, syntax rewrites, traits, and internal representations, but must not invalidate valid documented V2.1.0 source programs except where required to correct memory unsafety or an existing documented defect.

---

# Implementation scope

Start with these exact files and symbols.

Core runtime:

- `crates/compact_core/src/allocation.rs`
  - `ArenaState`
  - `ArenaInner`
  - `AllocationHeader`
  - `ArenaAllocation`
  - `CompactValue`
  - allocation/release/resize/free-list helpers
- `crates/compact_core/src/arena.rs`
  - `Arena`
  - `with_arena`
  - allocation/view APIs
- `crates/compact_core/src/backing.rs`
  - `StableBacking`
- `crates/compact_core/src/offset.rs`
  - `Offset32`
  - `OffsetSlice32`
- `crates/compact_core/src/error.rs`
- `crates/compact_core/src/tests.rs`

Std-backed ownership:

- `crates/compact_backend_std/src/memory.rs`
  - `StdBacking`
  - `StdArena`
- `crates/compact_backend_std/src/lib.rs`

Collections:

- `crates/compact_collections/src/vec.rs`
  - `CompactVec`
- `crates/compact_collections/src/string.rs`
  - `CompactString`
- `crates/compact_collections/src/boxed.rs`
  - `CompactBox`
- `crates/compact_collections/src/small.rs`
  - `CompactSmallVec`
- `crates/compact_collections/src/slab.rs`
  - `CompactSlab`
- `crates/compact_collections/src/intern.rs`
  - `CompactInterner`
- `crates/compact_collections/src/bitvec.rs`
- `crates/compact_collections/src/lib.rs`
- `crates/compact_collections/src/error.rs`
- `crates/compact_collections/tests/containers.rs`

Macros:

- `crates/compact_macros/src/arena.rs`
  - lexical binding-state model
  - constructor rewriting
  - receiver rewriting
  - move/assignment/branch handling
- `crates/compact_macros/src/compact.rs`
  - generated compact struct layouts
  - field classification
  - generated compact handles
- `crates/compact_macros/src/lib.rs`

Facade:

- `crates/compact_std/src/lib.rs`
- `crates/compact_std/src/prelude.rs`
- `crates/compact_std/tests/ui/`
- `fixtures/regular_rust_style/`
- `fixtures/arena_tracking/`

Documentation:

- `README.md`
- `ARCHITECTURE.md`
- `SAFETY.md`
- `BENCHMARKS.md`

Workspace/package metadata:

- root `Cargo.toml`
- affected crate `Cargo.toml` files
- `Cargo.lock`

Proposed new crates/modules/files are listed in the phases below.

---

# Verified repository facts

Current intended `main` inspected for this plan:

`e432ba6641049e17542e859175e360f52ebe86b9`

V2.1.0 is documented as the only supported contract.

The V2.1.0 compatibility surface includes:

- four-byte `Offset32<T>`;
- offset zero as null;
- 2^32-byte per-arena logical addressing;
- `arena!`;
- `#[compact]`;
- documented `Vec`, `String`, `Box`, collection, error, and explicit `*_in` behavior.

Internal allocation-header size, owner-token size, collection-handle size, free-list implementation, macro internal analysis, and packed-field implementation are explicitly not frozen.

`ArenaAllocation<T>` is currently a unique non-`Copy` owner containing:

- a `NonNull<ArenaState>`;
- a 32-bit offset;
- a 32-bit allocation identity;
- lifetime/threading marker state.

Its destructor destroys the initialized prefix and returns its block to the arena.

`ArenaState` uses an `UnsafeCell<ArenaInner>` and is deliberately mutable/single-owner.

The mutable arena therefore must not simply be marked `Send` or `Sync`.

`CompactVec`, `CompactString`, `CompactBox`, `CompactSmallVec`, `CompactSlab`, and `CompactInterner` use owning/reclaimable arena allocations.

`CompactString` currently has twelve inline bytes.

`CompactValue` is an unsafe movement/destruction contract.

The macro already tracks lexical scopes, moves, assignments, tuples, branches, loops, and selected closure use conservatively.

The std backend owns fixed stable memory in `StdBacking` using a non-resizing `Vec<MaybeUninit<u8>>`.

`StdBacking::with_arena` currently creates a fresh generative arena scope.

There is no persistent attach/reopen API for an already initialized arena.

There is no supported frozen arena, owned persistent store, direct arena-aware Serde layer, compact map/set/deque/path type, or scratch allocator.

The facade currently aliases only:

```text
Box
String
Vec
```

to compact equivalents.

Current real-world missing surfaces verified from the two target programs include:

```text
VecDeque
HashMap
HashSet
PathBuf
OsString
Serde JSON/TOML
collect
to_string
format!
vec!
clone requiring arena allocation
shared immutable data across threads/tasks
persistent arena ownership without exposed self-referential lifetimes
temporary scratch allocation
byte-buffer specialization
```

---

# Phase 0 — restore and lock the V2.1.0 baseline

This phase is mandatory before any V2.2 implementation.

Current `main` has a static API inconsistency:

`crates/compact_backend_std/src/lib.rs` still imports/re-exports `ABI_V1`, while `compact_core` now exports `ABI_VERSION`.

Fix that reference first.

Search the complete repository tree for any remaining obsolete public references to:

```text
ABI_V1
V1 ABI
old representation
original representation
0.1.0
```

Do not add compatibility aliases for older contracts.

The supported identifier is `ABI_VERSION`.

Before proceeding, require a completely green baseline:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features

cargo run --manifest-path fixtures/consumer/Cargo.toml
cargo run --manifest-path fixtures/regular_rust_style/Cargo.toml
cargo run --manifest-path fixtures/macro_layouts/Cargo.toml
cargo run --manifest-path fixtures/arena_tracking/Cargo.toml
```

No later phase may work around a baseline failure.

Record the exact green commit before V2.2 development begins.

---

# Compatibility invariants for every phase

Every phase must preserve the following.

A valid documented V2.1.0 program must continue to compile and preserve documented behavior.

Existing explicit APIs remain valid even when new shorthand becomes available.

`Offset32<T>` stays exactly four bytes.

Mutable arenas remain single-owner.

No global or thread-local ambient arena is introduced for normal runtime access.

No collection operation silently falls back to the native heap.

Allocation failure remains explicit.

Existing owner values are dropped exactly once.

Reusable storage cannot turn a stale owning handle into a valid new owner.

No public API may persist raw arena bytes as a durable/cross-process format.

New source conveniences must lower to explicit arena-aware operations.

Macro ambiguity must be rejected rather than guessed.

No `unsafe impl Send` or `unsafe impl Sync` may be added merely to satisfy an application integration.

---

# Phase 1 — owner-backed standard traits

Before adding more collection types, make existing owner-backed types act substantially more like their std equivalents.

## CompactVec

Implement where sound:

```text
Deref<Target = [T]>
DerefMut
AsRef<[T]>
AsMut<[T]>
Borrow<[T]>
BorrowMut<[T]>
Index
IndexMut
IntoIterator for &CompactVec
IntoIterator for &mut CompactVec
Debug
PartialEq / Eq
PartialOrd / Ord
Hash
```

The important architectural observation is that `ArenaAllocation<T>` already contains the stable arena-state pointer and owns the backing allocation.

Owner-backed reads therefore do not inherently require the caller to supply the `Arena` again.

Preserve the existing `get(..., arena)`, `as_slice(..., arena)`, etc. APIs for V2.1 compatibility.

Add internal owner-only accessors with safety reasoning that validates the owner token itself rather than requiring a second caller-supplied arena reference.

Do not weaken foreign-arena validation for APIs that still take an arena.

## CompactString

Implement where sound:

```text
Deref<Target = str>
AsRef<str>
Borrow<str>
Display
Debug
PartialEq<str>
PartialEq<&str>
Eq
Ord
Hash
fmt::Write
```

`fmt::Write` must use explicit arena-aware growth. Because the standard trait does not carry an arena argument, do not implement an allocation-capable `fmt::Write` directly unless the string object has a safe allocator context.

If allocation context cannot be supplied safely through the existing owner token, introduce a dedicated `CompactStringWriter<'_, 'arena>` created from:

```rust
text.writer(arena)
```

and implement `fmt::Write` on that writer.

Do not introduce hidden global allocation context.

## CompactBox

Implement `Deref`, `DerefMut`, `AsRef`, `AsMut`, `Debug`, comparisons and hashing where supported by `T`.

## Tests

For every trait, differential-test behavior against the corresponding std collection/string/box.

Add compile tests demonstrating that existing V2.1 explicit APIs still compile unchanged.

---

# Phase 2 — persistent owned arena/store

Introduce an ergonomic way to own arena backing for longer than a single callback without creating a self-referential Rust object.

Proposed std-backed type:

```rust
CompactStore<R>
```

Do not store `Arena<'arena, '_>` inside the object.

Do not store arbitrary lifetime-branded `R<'arena>` directly.

Instead split:

```text
owned stable backing
persistent allocator state
root descriptor encoded without a self-reference
ephemeral generative Arena on each access
```

## Persistent allocator attachment

Extend core arena initialization so there are separate operations for:

```text
initialize new arena state
attach to previously initialized arena state
```

Add an internal state header containing at least:

```text
magic
ABI_VERSION
capacity
allocator-state validity marker
```

The header must allow a `StdBacking` owner to safely detect:

- fresh/uninitialized backing;
- valid initialized arena;
- corrupt/incompatible state.

Do not treat arbitrary memory as an existing arena.

Existing `with_arena` behavior must remain compatible.

## Root identity

A store root must use an internal raw descriptor, not a lifetime-forged public `Offset32<'static, T>`.

Suggested internal form:

```rust
struct RawRoot {
    offset: u32,
    allocation_id or root-generation information where required,
}
```

On every `with`/`with_mut` access, convert the raw descriptor into a newly branded checked handle tied to that invocation's arena lifetime.

No public method should allow the branded value to escape the callback.

Proposed API shape:

```rust
CompactStore::build(capacity, |arena| -> Result<RootHandle<'_>> { ... })

store.with(|arena, root| {
    ...
})

store.with_mut(|arena, root| {
    ...
})
```

Exact root trait/type should be chosen after proving that it cannot retain an old arena brand.

## Drop

Dropping `CompactStore` must correctly destroy owned root state when the root type has destructors.

If arbitrary owner trees cannot be safely destroyed from a lifetime-erased root, restrict the first root contract to an explicit `StoreRoot` trait with generated destruction/access hooks.

Do not solve this with unchecked transmute.

## Safety tests

Test:

- moving `CompactStore`;
- repeated access with distinct generated lifetimes;
- inability to return arena references from `with`;
- root destruction exactly once;
- corrupted header rejection;
- root mismatch rejection;
- allocator persistence across accesses.

Add trybuild tests ensuring borrowed arena values cannot escape.

---

# Phase 3 — scratch allocation

Provide a separate scoped scratch facility for temporary work.

Do not add an unrestricted `checkpoint()/rewind()` to the general mutable arena if doing so could invalidate independently owned `ArenaAllocation` values.

Preferred API:

```rust
arena.scratch(|scratch| {
    ...
})
```

or a dedicated:

```rust
ScratchArena
```

with an HRTB lifetime preventing scratch references from escaping.

## Scratch semantics

Scratch allocations must support:

- fast bump allocation;
- deterministic bulk release at scope end;
- nested scratch scopes;
- alignment;
- bytes;
- primitive slices;
- compact temporary `Vec`, `String`, and byte buffers.

If values with `Drop` are permitted, scratch must maintain a compact LIFO destructor stack.

The destructor stack itself must be stored in scratch-managed memory and contain:

```text
drop function
object offset
element count/layout metadata
previous record
```

During scope cleanup:

1. prevent new allocation;
2. run registered destructors exactly once in reverse construction order;
3. survive a destructor panic without exposing already-dropped records as live;
4. release the entire scratch region.

If this cannot be proven sound for the first implementation, restrict scratch values to a sealed no-drop class and explicitly use the general arena for drop-bearing values.

Correctness takes priority over feature breadth.

## Tests

Test nested scratch scopes, alignment, zero-sized types, allocation failure, destructor order, panic during destruction, and inability to escape scratch references.

---

# Phase 4 — CompactBytes

Add:

```rust
CompactBytes<'arena>
```

optimized for byte payloads.

Use small inline storage plus arena-backed long storage.

Choose the inline capacity by benchmark rather than assumption; evaluate at least 12, 16, 20 and 24 bytes against wrapper-size growth.

Required API:

```text
new
with_capacity
from_slice
len
capacity
is_empty
as_slice
push
extend_from_slice
truncate
clear
reserve
shrink_to_fit
split/consume helpers where they can remain safe
```

Implement std-compatible byte-slice traits.

Use this as the preferred storage primitive for:

- protocol payloads;
- log argument bytes;
- chunk bodies;
- compact OS strings on Unix.

Do not automatically replace temporary `Vec<u8>` if benchmarks show fixed arena metadata costs more than the native allocation for that workload.

---

# Phase 5 — CompactVecDeque / bounded ring

Add:

```rust
CompactVecDeque<'arena, T>
```

with a circular arena-owned allocation.

Required API compatibility target:

```text
new
with_capacity
len
capacity
is_empty
front
front_mut
back
back_mut
push_front
push_back
pop_front
pop_back
get
get_mut
iter
iter_mut
clear
truncate
reserve
shrink_to_fit
make_contiguous
retain
```

Also add an explicitly bounded form suitable for logging:

```rust
CompactRing<'arena, T>
```

or a max-length policy on `CompactVecDeque`.

A bounded ring must make eviction semantics explicit.

On eviction, dropped values must be destroyed exactly once before their slots become reusable.

## Correctness model

Track logical head + initialized length independently from physical capacity.

Growth must preserve logical ordering.

Wrapping moves must not duplicate ownership.

Panic in `Drop` must not make already-removed entries live again.

Zero-sized element behavior must be defined and tested.

## Real-world fixture

Add a fixture reproducing the `rideshare-bot` shape:

```text
1,000 retained log-like records
push-back
oldest-entry eviction
snapshot iteration
clear
reuse
```

Compare logical behavior against `std::collections::VecDeque`.

---

# Phase 6 — CompactHashMap and CompactHashSet

Add:

```rust
CompactHashMap<'arena, K, V, S = ...>
CompactHashSet<'arena, T, S = ...>
```

Do not copy `std::HashMap`'s internal representation blindly.

Use a compact open-addressed table with arena-owned metadata and entries.

The implementation must have explicit handling for:

```text
empty
occupied
deleted/tombstone
rehash/grow
hash collision
removal
drop
panic during Hash/Eq
```

Prefer one compact metadata/control allocation plus one entry allocation unless benchmarks prove a combined allocation materially better.

## Hashing

For the std facade, default to a DoS-resistant randomized hasher compatible with ordinary application expectations.

Do not introduce a deterministic weak default merely to reduce metadata.

Allow user-supplied `BuildHasher`.

If keeping `compact_collections` usable without std later is desired, separate the generic map from the facade's default hasher.

## API target

Implement the high-value std surface:

```text
new
with_capacity
with_hasher
with_capacity_and_hasher
len
capacity
is_empty
insert
get
get_mut
contains_key
remove
remove_entry
clear
reserve
shrink_to_fit
iter
iter_mut
keys
values
values_mut
entry
retain
```

Implement `HashSet` as a thin map-backed abstraction.

## Panic safety

This is a critical correctness area.

Hashing and equality can panic.

Never mark a slot occupied or destroy the old value until every potentially panicking lookup/comparison needed for the operation has completed.

Use explicit insertion/removal guards where state transitions cross panic-capable user code.

## Differential testing

Run randomized operation streams against `std::HashMap`/`HashSet`.

Include:

- deliberate all-keys-collide hasher;
- repeated grow/shrink;
- removal/reinsertion;
- non-`Copy` drop counters;
- panicking `Hash`;
- panicking `Eq`;
- allocation exhaustion.

---

# Phase 7 — CompactOsString and CompactPathBuf

Add std-backed platform-aware compact equivalents:

```rust
CompactOsString<'arena>
CompactPathBuf<'arena>
```

and borrowed views where useful.

## Platform representation

On Unix targets, store exact OS bytes.

Use:

```rust
std::os::unix::ffi::OsStrExt
std::os::unix::ffi::OsStringExt
```

On Windows, store exact wide units and use the corresponding Windows `OsStr`/`OsString` APIs.

Do not assume UTF-8.

Do not use lossy conversion internally.

Do not persist the internal platform representation as a portable file format.

## Path API

Implement the subset heavily used by ordinary services:

```text
new
from
as_path
as_os_str
push
pop
set_file_name
set_extension
join
parent
file_name
file_stem
extension
components
is_absolute
is_relative
starts_with
ends_with
display
```

Operations that require mutation/growth remain fallible with explicit arena context underneath.

Add facade aliases:

```rust
OsString
PathBuf
```

inside `compact_std`.

## Tests

Include Unix non-UTF8 path bytes.

If CI can test Windows, include non-Unicode wide paths there.

Path behavior should be differential-tested against `std::path::PathBuf`.

---

# Phase 8 — collection construction traits

Introduce explicit arena-aware equivalents of allocation-requiring std traits.

Proposed internal traits:

```rust
FromIteratorIn<'arena, T>
ExtendIn<'arena, T>
CloneIn<'arena>
ToCompactStringIn<'arena>
```

Names may change, but semantics must remain explicit.

Example APIs:

```rust
CompactVec::from_iter_in(iter, arena)
value.clone_in(arena)
value.to_compact_string_in(arena)
```

These explicit APIs are the primitive operations.

`arena!` may later rewrite ordinary syntax to them.

Do not implement ordinary `Clone` for an arena owner by silently allocating from hidden state.

For types whose clone is truly allocation-free, ordinary `Clone` remains acceptable.

---

# Phase 9 — formatting

Add arena-backed formatting without global context.

Provide:

```rust
format_in!(arena, "...", args...)
```

implemented using a compact string writer.

The writer must:

- preserve UTF-8;
- propagate arena exhaustion;
- leave a valid prefix/string after error;
- not hide allocation failure.

Then teach `arena!` to rewrite recognized:

```rust
format!(...)
```

to `format_in!(arena, ...)`.

Do not rewrite arbitrary user macros.

Add compile-fail diagnostics when `format!` expansion cannot safely use the current arena.

---

# Phase 10 — `vec!`, `to_string`, clone and iterator collection rewriting

Expand `arena!` conservatively.

## `vec!`

Support:

```rust
vec![]
vec![a, b, c]
vec![value; count]
```

by rewriting to compact constructors.

For repeated-element form, preserve ordinary Rust evaluation rules:

- evaluate `value` once where std does;
- require the equivalent clone capability;
- propagate allocation failure safely.

Do not accidentally evaluate expressions more times than native `vec!`.

## `to_string`

For known compact-compatible receivers inside `arena!`, rewrite:

```rust
value.to_string()
```

to the explicit arena-aware string conversion.

Do not rewrite a call if binding/type classification is ambiguous.

## clone

For known compact owning receivers whose clone requires allocation, rewrite:

```rust
owner.clone()
```

to:

```rust
owner.clone_in(arena)?
```

Preserve normal `.clone()` for native values.

## collect

Recognize:

```rust
iterator.collect::<Vec<_>>()
iterator.collect::<String>()
iterator.collect::<HashMap<_, _>>()
iterator.collect::<HashSet<_>>()
```

when the target type is a known compact facade type.

Lower through explicit `FromIteratorIn`.

Do not infer arbitrary helper return types.

## Additional common methods

Add macro support only after explicit APIs exist and are tested:

```text
extend
resize
resize_with
retain
dedup
sort
sort_by
sort_by_key
append
split_off
```

Do not attempt full std parity in one syntactic pass if semantics are not exact.

---

# Phase 11 — direct Serde support

Create a new workspace crate:

```text
crates/compact_serde
```

and re-export its primary APIs from `compact_std` behind a `serde` feature if feature gating is useful.

Do not deserialize to native `String`/`Vec` and then copy as the normal path.

The primary design must use `serde::de::DeserializeSeed` carrying an arena.

Proposed concepts:

```rust
CompactDeserialize<'de, 'arena>
CompactDeserializeSeed<'arena, T>
```

Exact trait spelling may vary.

Built-in support must include:

```text
bool
integer/floating scalars
Option
tuples
CompactString
CompactBytes
CompactVec
CompactVecDeque
CompactHashMap
CompactHashSet
CompactOsString where format semantics permit it
generated compact structs/enums
```

## Generated struct support

Extend the macro crate with either:

```text
#[derive(CompactDeserialize)]
```

or an option on `#[compact]`.

Prefer a distinct derive if that keeps `#[compact]` responsibilities simpler.

Generated deserialization must construct compact fields directly as the Serde visitor reads them.

Do not build an intermediate native struct.

Support Serde field attributes required by common configurations:

```text
rename
rename_all
default
deny_unknown_fields
skip/default where semantically appropriate
```

Do not claim support for arbitrary Serde attributes until tested.

## JSON/TOML helpers

Provide ergonomic helpers, potentially:

```rust
compact_serde::json::from_slice_in(...)
compact_serde::json::from_str_in(...)
compact_serde::toml::from_str_in(...)
```

If `serde_json`/`toml` are optional dependencies, keep them feature-gated.

These helpers must stream/visit directly into arena-backed values.

## Failure behavior

Malformed input, duplicate fields, unknown fields, depth errors, arena exhaustion and custom visitor errors must leave the arena/store in a valid state.

Use transaction/scratch semantics or owner guards so partially built data is destroyed/reclaimed.

## Real-world fixture

Add a reduced `local-service-orchestrator` configuration fixture containing:

```text
system_id: String
Vec<String>
HashSet-like validation input
PathBuf fields
nested structs
Option fields
```

Deserialize TOML directly into compact representation.

Add a JSON node registry equivalent.

---

# Phase 12 — frozen immutable arena

This is the highest-risk part of the plan.

Do not make `Arena`, `ArenaState`, `ArenaAllocation`, `CompactVec`, or another mutable owner automatically `Send`/`Sync`.

Introduce a separate immutable representation.

Proposed types:

```rust
FrozenArena
FrozenVec<T>
FrozenString
FrozenBytes
FrozenMap<K, V>
FrozenSet<T>
FrozenPathBuf
FrozenRoot<T>
```

Names can be adjusted, but the mutable/frozen distinction must remain obvious.

## Freeze model

Freezing is irreversible.

Conceptually:

```text
Mutable build state
        ↓ freeze()
immutable backing
        ↓
FrozenArena
```

A frozen arena contains no mutable allocator operation reachable through safe APIs.

Do not share `UnsafeCell<ArenaInner>` mutation across threads.

Freeze must either:

1. convert mutable owner representations into immutable offset-only frozen handles, or
2. build directly into a freeze-safe representation.

Do **not** simply cast an `ArenaAllocation` into a `Send + Sync` object.

## Frozen value contract

Introduce an explicit freeze-safe contract.

Prefer a sealed safe trait for library-owned primitives/handles where possible.

If an unsafe public trait is necessary, document requirements at least as strictly as `CompactValue`.

A frozen value must not contain:

```text
mutable native pointer ownership
interior mutability that violates Sync
non-thread-safe reference-counting
address-sensitive self references
destructors requiring mutable arena allocator access
borrowed native references with invalid lifetime
```

A frozen arena should normally be reclaimable by dropping the entire backing rather than individually releasing blocks.

This means frozen values should preferably be immutable offset/length descriptors without per-allocation Drop reclamation.

## Compaction during freeze

Consider freeze as an opportunity to compact live data into a dedicated immutable backing.

Preferred correctness model:

```text
mutable source arena
    ↓ traversal/copy using generated freeze traits
dense frozen destination
```

rather than preserving mutable allocator headers/free holes.

Benefits:

- no free list in frozen storage;
- no mutable allocator state;
- denser layout;
- clearer Send/Sync proof;
- stale mutable owners cannot alias frozen objects.

The source remains valid until the freeze operation commits.

If destination allocation/copy fails, return an error and keep source state valid.

## Threading

`FrozenArena` may implement `Send + Sync` only after all reachable representation invariants are immutable and audited.

Use compile-time assertions.

Test actual:

```rust
Arc<FrozenArena>
```

access from multiple threads.

No lock should be required for immutable reads.

## Integration target

This directly enables the `local-service-orchestrator` pattern:

```text
parse configuration
build compact mutable graph
freeze
Arc<FrozenArena>
share across worker threads/tasks
```

---

# Phase 13 — frozen cheap-clone/interned data

Build immutable shared handles on top of the frozen arena.

Provide compact, `Copy`/cheaply cloneable frozen references for strings and other immutable values.

Example conceptual representation:

```text
offset: u32
len: u32
```

or a denser safe representation where possible.

Integrate the existing interning concept so repeated immutable strings can share one payload.

This is valuable for:

```text
configuration strings
repository/branch names
system IDs
paths
health URLs
log catalog strings
```

Do not add reference counts for immutable objects inside a frozen arena; arena lifetime owns the entire backing.

---

# Phase 14 — FFI helpers

Keep arena internals behind Rust.

Add explicit helpers for exposing temporary borrowed native views:

```text
as_ptr
as_slice
as_str
with_ffi_bytes
```

where ordinary borrow lifetimes are sufficient.

For FFI outputs that must survive after the Rust call returns, require an explicit copied/exported native allocation and matching free function.

Do not expose raw arena offsets as Swift/JNI/C object identities.

Add documentation using `rideshare-bot` patterns:

```text
compact internally
repr(C) at boundary
copy only when caller lifetime requires it
```

---

# Phase 15 — real-world compatibility fixture

Add a new fixture:

```text
fixtures/drop_in_patterns
```

It must contain representative code distilled from the two inspected applications without depending on those private repositories.

Include:

```text
1. rideshare log ring
   VecDeque / bounded eviction
   compact strings/arguments
   snapshot iteration

2. chunk reassembler
   Vec<Option<Bytes>>
   out-of-order chunks
   duplicates
   completed body

3. service configuration
   strings
   paths
   nested vectors
   maps/sets
   TOML direct deserialization

4. frozen configuration
   freeze
   Arc
   multi-thread reads

5. scratch transformations
   temporary packet/format construction

6. ordinary arena! syntax
   vec!
   format!
   clone
   collect
   to_string
   HashMap
   HashSet
   VecDeque
   PathBuf
```

The fixture should look intentionally like normal Rust.

Its purpose is to detect ergonomic regressions that isolated unit tests will not catch.

---

# Phase 16 — compatibility regression fixture

Create a separate immutable fixture:

```text
fixtures/v2_1_contract
```

Populate it only with syntax and public APIs documented by V2.1.0.

Once created, do not modernize it to newer shorthand.

Every future 2.x change must compile and run this fixture unchanged.

This is the executable definition of the frozen V2.1 source contract.

---

# Unsafe-code correctness program

The new feature set substantially increases the amount of correctness-sensitive code.

Before V2.2 can be considered complete, establish a dedicated unsafe-code validation program.

## Miri

Add a nightly CI job that installs Miri and runs appropriate core/collection tests.

Prioritize:

```text
ArenaAllocation movement
drop
growth
shrink
reuse
VecDeque wrap/grow
HashMap insert/remove/rehash
scratch destructor stack
CompactStore reattachment
freeze copy/commit
frozen reads
```

Miri failures block release.

Do not waive a Miri failure because normal tests pass.

## Model-based allocator tests

Add randomized deterministic operation testing against a simple reference allocator model.

Operations:

```text
allocate
release
grow
shrink
write/read sentinels
fragment
coalesce
tail release
reuse
```

After every operation verify:

```text
no overlapping live allocations
all live ranges in bounds
free ranges disjoint
free ranges sorted/coalesced
cursor in bounds
initialized <= capacity
live data unchanged unless operation specified mutation
```

Use a seeded PRNG and print the seed/operation trace on failure.

## Property testing

Add `proptest` or equivalent as a dev dependency.

Use it for:

```text
Vec versus std::Vec
VecDeque versus std::VecDeque
HashMap versus std::HashMap
HashSet versus std::HashSet
String versus std::String
PathBuf versus std::PathBuf
Serde round-trip/logical equality
packed bits
```

Do not use wall-clock timing or real sleeps.

## Fuzzing

Add `cargo-fuzz` targets or equivalent for:

```text
allocator operation sequences
packed bit operations
HashMap collision/removal sequences
Serde JSON input
Serde TOML input
chunk-like byte structures
frozen graph decoding/access
```

Fuzz tests should avoid external services.

## Panic-safety tests

Create custom test values with:

```text
Drop counters
panicking Drop
panicking Hash
panicking Eq
panicking Display
```

Validate that after catching unwind where Rust permits it:

```text
no double drop
no owner duplication
collection remains destructible
allocator remains internally valid
```

If a particular operation is intentionally not unwind-safe, document that precisely and consider requiring `panic = abort` only if unavoidable.

Prefer unwind safety.

## ZST and alignment

Every generic owning collection must test:

```text
()
ZST with Drop
align(16)
align(32)
align(64)
```

Do not assume `size_of::<T>() > 0`.

## Allocation exhaustion

Every allocating public operation needs a deterministic bounded-arena failure test.

Verify old logical state survives when promised.

---

# Concurrency safety program

Mutable arena types must continue failing `Send`/`Sync` expectations.

Add compile-time tests that mutable arena owners cannot accidentally cross threads.

Frozen types must have explicit positive assertions only after freeze invariants are complete.

Test:

```text
Arc<FrozenArena> shared by many reader threads
parallel reads of strings/vectors/maps
drop after all clones disappear
```

No test should rely on sleeps for ordering.

Use barriers/channels where synchronization is needed.

Because frozen state is immutable, prefer proving absence of mutation over introducing locks.

---

# Macro correctness program

Every new rewrite requires:

```text
positive runtime fixture
compile-fail ambiguity test
native-value non-rewrite test
shadowing test
move test
branch-join test where relevant
closure test where relevant
evaluation-order test
side-effect-count test
```

For `vec![expr; n]`, verify `expr` evaluation matches std semantics.

For `format!`, verify argument evaluation order.

For `.clone()`, verify native clones remain native.

For `.collect()`, verify an explicitly native `std::vec::Vec` is never rewritten.

The macro must continue to prefer a diagnostic over incorrect inference.

---

# Serde correctness program

Test:

```text
empty input
malformed syntax
truncated syntax
unknown fields
duplicate fields
default fields
renames
nested structures
very long strings
non-ASCII strings
zero-length collections
large collections
arena exhaustion at every construction stage
nested failure cleanup
JSON numeric boundaries
TOML integer boundaries
```

If parser recursion/depth limits are provider-controlled, preserve them.

Do not disable parser safety limits to mimic native allocation.

---

# Hash-table security requirements

Default std-facing maps must not introduce easy hash-flooding regressions.

Use a randomized/DoS-resistant default hasher.

Document that callers choosing a deterministic custom hasher assume its collision/security characteristics.

Do not expose uninitialized/tombstone bytes through safe iteration.

Rehash must be failure-safe:

```text
allocate destination first
fully validate capacity
move entries without fallible post-move operations
commit owner replacement last
```

---

# Frozen-arena security requirements

Frozen input must never be treated as trusted serialized data.

`FrozenArena` is an in-process representation, not a file parser.

Do not add APIs that map arbitrary untrusted bytes and reinterpret them as frozen values.

A frozen backing is created only through trusted library construction/freeze APIs.

If persistent mapped arenas are desired in a future project, design a separate validated format.

---

# Memory/benchmark program

Extend the benchmark suite.

Measure at least:

```text
Vec
VecDeque
bounded ring
HashMap
HashSet
String
Bytes
PathBuf
Serde config parse
freeze
frozen read
scratch allocation
format!
collect
```

Track:

```text
payload bytes
arena used bytes
fixed metadata
allocation count where measurable
high-water use
fragmentation
runtime
```

Add real-world scenarios:

```text
1,000 log entries
chunk transfer with many fragments
service config with paths/strings
large immutable frozen config
```

Never encode benchmark sizes as compatibility requirements.

Use benchmarks to choose inline capacities and load factors, not intuition.

---

# Performance guardrails

Do not regress the V2.1 32-bit offset contract.

Do not add one native heap allocation per compact object.

Do not add hash-table or registry lookup to ordinary offset dereference.

Do not add locking to single-threaded mutable arena operations.

Do not add locking to immutable frozen reads.

Do not force all byte buffers through compact allocation if inline/native representation is demonstrably cheaper.

Do not sacrifice correctness merely to preserve a wrapper-size target.

---

# Documentation changes

After implementation, update:

`README.md`

to show near-drop-in `arena!` examples.

`ARCHITECTURE.md`

to document:

```text
mutable arena
persistent CompactStore
scratch arena
freeze pipeline
frozen representation
maps/deques/path storage
Serde construction
```

`SAFETY.md`

to add exact contracts for:

```text
StoreRoot
scratch values/destructor records
frozen values
Send/Sync boundary
HashMap panic safety
Serde partial construction
```

`BENCHMARKS.md`

with reproducible new benchmarks.

Do not list older release compatibility.

The wording remains:

```text
V2.1.0 is the compatibility baseline for 2.x.
```

When V2.2 is actually released, it may be identified as the current release, but V2.1.0 remains the frozen source contract being preserved.

Do not add a migration guide from pre-V2.1 experimental versions.

---

# Versioning

Do not bump the workspace version at the beginning of implementation merely to mark work in progress.

Complete and validate the feature set first.

Immediately before the final release commit:

1. verify latest `main`;
2. rerun all compatibility tests;
3. update workspace/package versions consistently to the selected V2.2 release version;
4. update `Cargo.lock`;
5. update current-release documentation without weakening the V2.1 compatibility statement.

---

# Integration order

The implementation order is intentionally constrained.

Phase 0 must complete first.

Traits and persistent ownership foundations should land before higher-level syntax.

Recommended integration sequence:

```text
0  baseline repair
1  owner-backed std traits
2  CompactStore / persistent attach
3  scratch
4  CompactBytes
5  VecDeque/ring
6  HashMap/HashSet
7  OsString/PathBuf
8  allocation-aware construction traits
9  formatting
10 arena! expanded syntax
11 direct Serde
12 frozen arena
13 frozen cheap handles/interning
14 FFI helpers
15 real-world fixture
16 immutable V2.1 contract fixture
final hardening / docs / release
```

Frozen arena depends on understanding all representations that need immutable equivalents and therefore should not be implemented first even though it is a major user-facing feature.

Serde should be implemented before freeze so frozen-config tests can exercise the intended real-world path:

```text
text
→ direct compact deserialize
→ validate
→ freeze
→ Arc
```

---

# Shared-file ownership

Because this is intentionally one giant plan, implementation should avoid simultaneous edits to central hotspots.

Exclusive central ownership during relevant phases:

```text
crates/compact_core/src/allocation.rs
crates/compact_core/src/arena.rs
crates/compact_macros/src/arena.rs
crates/compact_std/src/lib.rs
crates/compact_std/src/prelude.rs
```

New collection modules can be developed independently after their core prerequisites are integrated, but their facade exports should be integrated centrally.

Do not let separate executors independently redesign `ArenaAllocation`, `CompactValue`, arena reattachment, or freeze semantics.

Any contradiction involving those interfaces must be resolved centrally before dependent work continues.

---

# Search boundaries for executors

Executors should begin with the exact files/symbols named in the relevant phase.

Broader repository search is justified only when:

```text
a symbol moved;
a compilation error exposes another call site;
a test identifies a compatibility dependency;
the current implementation contradicts a verified plan fact;
correctness requires inspecting another unsafe path;
a public re-export needs updating;
a generated macro path depends on another crate.
```

Do not re-explore the entire repository for every collection.

---

# Non-goals

Do not implement:

```text
a replacement kernel/runtime;
global allocator replacement;
transparent interception of arbitrary std allocation outside arena!;
binary ABI compatibility with std containers;
persistent raw arena files;
cross-process arena sharing;
mutable lock-free shared arenas;
automatic unsafe CompactValue implementations for arbitrary user types;
a hidden TLS/global current arena;
old pre-V2.1 compatibility aliases.
```

Literal type identity with `std::Vec`, `std::String`, etc. is not required.

The target is source-level near-drop-in ergonomics inside an explicit arena domain.

---

# Final validation matrix

Before calling the implementation complete, run:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features
```

Run every fixture individually.

Run the immutable V2.1 compatibility fixture unchanged.

Run the real-world drop-in fixture.

Run release benchmarks.

Run nightly Miri coverage.

Run deterministic property/model tests.

Run fuzz targets for an agreed bounded campaign before release.

Run platform builds for every platform the newly added `OsString`/`PathBuf` implementation claims to support.

At minimum, explicitly validate the target platforms used by the motivating applications:

```text
Linux
Android
iOS
```

If Windows support is implemented, it must have Windows-specific path tests before being documented as supported.

---

# Final diff checklist

Before the implementation is committed as complete, verify:

- V2.1 fixture is unchanged.
- `Offset32<T>` remains four bytes.
- no mutable arena type became `Send`/`Sync`.
- no hidden global/TLS arena was introduced.
- no native heap fallback was introduced.
- no old `ABI_V1`/pre-V2.1 compatibility API remains.
- every new unsafe block contains a local safety explanation.
- every unsafe trait has complete safety documentation.
- every drop-bearing relocation path has exact-once tests.
- every allocating operation has bounded-arena failure coverage.
- map/set operations have collision and panic tests.
- scratch cleanup has panic/drop tests.
- Serde partial builds reclaim all owned allocations.
- frozen values contain no mutable allocator ownership.
- `FrozenArena` is only `Send + Sync` after the immutable proof and tests exist.
- FFI APIs do not expose arena offsets as external object handles.
- all new aliases/re-exports are present in both facade and prelude where intended.
- docs describe only the current supported 2.x contract and V2.1 compatibility baseline.
- benchmarks are reproducible and clearly non-normative.
- no temporary planning files remain in the final implementation commit.

---

# Execution handoff

Implementation should begin with:

> Implement PLAN.md exactly. Verify latest main first. Restore and validate the V2.1.0 baseline before feature work. Preserve the V2.1.0 source contract. Stay within the named phase scope unless code, compilation, tests, or safety require expansion. Never solve Send/Sync or lifetime problems with unchecked transmute or by weakening ownership invariants. Run deterministic tests after every phase.

For substantial phases, use isolated branches/worktrees.

Each executor must report:

```text
changed files
commit SHA
tests run
test results
deviations
new assumptions
unresolved safety questions
```

Integrate phases in the order specified above.

After every core-runtime phase, rerun all existing collection tests before continuing.

After every macro phase, rerun all trybuild and fixture tests.

After Serde and freeze, run the complete workspace plus real-world fixture before continuing.

The integrating/orchestrating model must independently compare the final diff with this plan and resolve contradictions centrally rather than allowing a phase to silently redesign the architecture.

At the end:

1. review the complete implementation diff;
2. run the entire validation matrix;
3. review every new unsafe block;
4. verify V2.1 compatibility fixture unchanged;
5. delete `PLAN.md`;
6. commit the implementation without planning files remaining.
