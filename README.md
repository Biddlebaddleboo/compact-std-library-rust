# compact-std-library-rust

`compact_std` provides familiar Rust collection APIs over a scoped compact
arena. Storage uses 32-bit byte offsets; application code keeps ordinary Rust
control flow and opts into compact layouts with procedural macros.

## Supported contract

**V2.1.0 is the only supported contract.**

The documented V2.1.0 source surface is the compatibility baseline for future
2.x releases. Valid code that uses documented V2.1.0 APIs and syntax should
continue to compile and preserve documented behavior across 2.x releases.

A future release may reject previously accepted code when doing so is required
to fix memory unsafety, a security defect, or behavior that contradicted the
documented V2.1.0 contract.

The stable source contract includes:

- `arena!` syntax and its documented rewriting behavior;
- facade names such as `Vec`, `String`, and `Box`;
- documented collection methods and explicit `*_in` APIs;
- documented `#[compact]` and `#[compact(soa)]` syntax;
- `Offset32<T>` as a four-byte, 32-bit byte offset;
- offset zero as the null sentinel;
- the one-arena logical address limit of 2^32 bytes;
- documented error and ownership behavior.

Internal representation is not a source-compatibility promise. Allocation
headers, owner-token sizes, collection handle sizes, free-list structure,
allocator policy, macro-analysis internals, and packed-access implementation
may change without changing documented V2.1.0 source syntax or behavior.

## Workspace crates

- `compact_core` is the backend-independent `#![no_std]` runtime using only
  `core`.
- `compact_backend_std` provides stable backing memory using `std`.
- `compact_collections` provides arena-owned compact collections and handles.
- `compact_serde` provides direct arena-aware Serde deserialization.
- `compact_frozen` copies compact graphs into immutable shared backing.
- `compact_macros` provides `#[compact]` and lexical `arena!` rewriting.
- `compact_std` is the std-backed convenience facade that re-exports the
  runtime, collections, macros, and prelude.

`compact_macros` is a procedural-macro crate and therefore uses `std` on the
compiler host. `compact_std` is intentionally a std-backed facade. Only
`compact_core` currently guarantees a target-side `no_std` contract.

## Memory model

One arena addresses at most 2^32 bytes. `Offset32<T>` is exactly four bytes,
and offset zero is reserved as the null sentinel. Compact references resolve
relative to the arena backing rather than storing native pointers.

The arena owns reusable allocation state. Released blocks are coalesced and can
be reused; tail allocations can grow in place where capacity permits. Owning
containers run destructors and release their allocation when dropped.

Arena memory is an in-process runtime representation. It is **not** a
persistent file format, IPC format, network format, or cross-target binary
format. Packed numeric storage uses target-native byte order.

Arenas and owning compact allocations are intentionally single-owner and are
not a cross-thread ownership mechanism.

Use `arena.scratch(capacity, |scratch| { ... })` for temporary work. It creates
a nested arena in a parent-owned allocation and releases that allocation when
the callback returns. Scratch scopes can nest and can host compact vectors,
strings, byte ranges, and other arena-owned values. Their higher-ranked
lifetimes prevent scratch references and owners from escaping.

## Persistent in-process stores

`CompactStore<T>` owns a stable backing and preserves allocator state across
callbacks. Its root is restricted to `Copy + CompactValue + 'static` values, so
the store never erases a destructor obligation or keeps an arena-branded value
inside itself. Each access rebrands a checked root handle for that callback:

```rust
let mut store = CompactStore::<u32>::build(4096, |arena| arena.alloc_value(41))?;
let value = store.with(|arena, root| root.get(arena).map(|value| *value))??;
assert_eq!(value, 41);
```

`with_mut` supports controlled updates and further arena allocations. The
store is an in-process owner; dropping it releases the complete backing. Raw
arena bytes are still not a durable or cross-process format.

See [ARCHITECTURE.md](ARCHITECTURE.md) for allocator and representation
details, and [SAFETY.md](SAFETY.md) for the unsafe and lifetime contracts.

## Regular Rust style

```rust
use compact_std::prelude::*;

StdArena::with_capacity(4096, |arena| -> Result<()> {
    arena!(arena, {
        let mut values = Vec::new();
        values.push(10_u32)?;
        values.push(20)?;
        assert_eq!(values.get(1)?, Some(&20));

        let mut text = String::from("hello")?;
        text.push_str(" compact")?;
        assert_eq!(text.as_str()?, "hello compact");
        Ok(())
    })
})??;
```

`arena!` rewrites supported compact constructors and arena-dependent methods
inside its lexical block. It does not install global or thread-local ambient
state.

`arena!` rewrites `vec![]`, list, and repeat forms to compact vectors. It also
rewrites known compact owners' `.clone()`, compact strings' `.to_string()`,
and iterator `.collect::<...>()` calls whose target is a compact facade type.
These constructors and conversions propagate allocation errors with `?`.
Explicit `std::vec!` and native collection targets remain native. When a
receiver or target type is ambiguous, use an explicit compact type annotation
or the corresponding arena-aware API.

### `arena!` binding rules

The macro tracks lexical bindings, moves, shadowing, tuple destructuring,
reassignment, branches, loops, and supported closure reads conservatively.

A procedural macro cannot infer arbitrary helper-function return types. When a
helper returns a compact collection, provide an explicit compact type
annotation:

```rust
let mut values: Vec<'_, u32> = make_values(arena)?;
values.push(1)?;
```

If control flow makes a receiver ambiguous, use an explicit type annotation or
the corresponding `*_in(..., arena)` API.

Read-only closure captures can use supported compact values. Allocation-bearing
macro rewrites inside closures are rejected so closure traits and arena
borrowing stay explicit; use `format_in!`, `FromIteratorIn`, `CloneIn`, and
other explicit arena-aware APIs there.

Unknown macro token streams are not rewritten.

## Collections

Generic owning collections accept values implementing the unsafe
`CompactValue` contract. Primitive values, supported tuples/arrays/options,
generated compact handles, and compact owner wrappers implement it directly.

`CompactVec<T>` owns a reclaimable contiguous allocation. Growth first tries
in-place resize, otherwise allocates replacement storage and moves initialized
values without duplicating ownership. `pop`, `truncate`, `clear`, and drop
preserve ordinary destructor ownership.

`CompactString` stores up to twelve UTF-8 bytes inline and owns reclaimable
arena bytes after promotion. `CompactBytes` stores up to twenty arbitrary
bytes inline and owns reclaimable arena storage for longer payloads.

`CompactSmallVec<T, N>` stores up to `N` values inline before promotion.

`CompactVecDeque<T>` provides a growable circular queue, and `CompactRing<T>`
provides a fixed-capacity log buffer that drops its oldest entry before reuse.

`CompactSlab` combines allocation identity with slot generations so stale
handles remain invalid after slot or allocation reuse.

`CompactInterner` uses a linear scan intended for small intern sets.

`CompactHashMap<K, V>` and `CompactHashSet<T>` use arena-owned open-addressed
tables. Their default hasher is randomized; custom `BuildHasher` values are
supported. Operations that can allocate, such as insertion, reserve, and
shrink, take the arena explicitly. Lookup and iteration validate the owning
arena before returning borrowed values.

`CompactOsString` and `CompactPathBuf` preserve native Unix bytes and Windows
wide units without converting through UTF-8. Their borrowed `CompactOsStr` and
`CompactPath` views stay tied to the compact owner; mutating and joining paths
take the arena and report allocation errors.

## Direct Serde deserialization

Enable `serde` for `CompactDeserialize` and its derive, `json` for JSON helpers,
and `toml` for TOML helpers:

```toml
compact_std = { version = "2.1.0", features = ["json", "toml"] }
```

The visitors use `DeserializeSeed` to build compact strings, bytes, vectors,
deques, maps, sets, paths, and OS strings directly in the supplied arena.
Scalar values, `Option`, and tuples are supported as nested values. For
example, `compact_std::json::from_slice_in::<MyConfig<'_>>(bytes, arena)`
returns the compact configuration or a parse/allocation error.

`#[derive(CompactDeserialize)]` supports named structs and unit enums. The
supported attributes are `rename`, `rename_all`, `default` (including a
function path), `deny_unknown_fields`, `skip`, and `skip_deserializing` on
struct fields; enum variants support `rename` and container `rename_all`.
Unsupported Serde attributes and unsupported enum payloads produce compile
errors. Path and OS string inputs use the format's UTF-8 string representation.
The JSON and TOML modules are optional features, so applications can use the
core Serde traits without pulling in either parser.

## Frozen configuration graphs

`#[derive(CompactFreeze)]` generates a lifetime-free companion made from
immutable frozen handles. `freeze_in` copies the live compact graph into a new
backing and returns its arena and typed root:

```rust
let (frozen, root) = freeze_in(&config, arena)?;
let frozen = std::sync::Arc::new(frozen);
let config = root.get(&frozen)?;
let id = config.system_id().as_str(&frozen)?;
```

`FrozenArena` owns only immutable values and offset descriptors. It has no
allocator state or per-value reclamation, so `Arc<FrozenArena>` supports
lock-free reads from worker threads. Frozen handles validate their arena
identity before resolving an offset. The mutable source remains valid after a
successful copy and if freezing fails. Frozen string, byte, OS-string, and path
handles are `Copy`; the builder interns equal byte payloads so repeated
configuration and catalog strings share one immutable range.

The derive supports named structs with scalars, `Option`, tuples, arrays,
nested derived structs, and `CompactString`, `CompactBytes`, `CompactVec`,
`CompactVecDeque`, `CompactHashMap`, `CompactHashSet`, `CompactOsString`, and
`CompactPathBuf` fields. Unit enums must also implement `Copy`. Frozen maps and
sets currently use linear lookup.
`FrozenPathBuf` and `FrozenOsString` preserve platform-native code units and
can be copied back to native standard-library types when needed. `FrozenValue`
is an unsafe extension contract; read [SAFETY.md](SAFETY.md) before
implementing it for a custom type.

## Native interface boundaries

Keep application data compact while it stays in Rust, and define `#[repr(C)]`
records for the fields that cross a native boundary. For a synchronous call,
`CompactBytes::with_ffi_bytes` and `CompactString::with_ffi_bytes` lend the
initialized bytes to the callback; the native callee must finish using the
pointer before the callback returns. `as_ptr`, `as_slice`, and `as_str` expose
ordinary borrowed views when the caller can keep the Rust owner alive.

When native code must retain bytes after the call, copy them explicitly with
`FfiByteBuffer::copy_from_slice`, then release the allocation exactly once with
`compact_std_ffi_bytes_free`. The exported value contains a native pointer,
length, and capacity; its fields are an ownership token and must not be changed
or freed twice. Arena offsets are never native object identities.

`FromIteratorIn`, `ExtendIn`, and `CloneIn` make allocation-aware collection
operations explicit. `ToCompactStringIn` formats through
`CompactStringWriter`, preserving arena exhaustion as a collection error.
These traits are the primitive operations used by `arena!` syntax lowering.

`format_in!(arena, ...)` returns `Result<CompactString>` and writes UTF-8
directly into compact storage. It preserves a valid prefix if the arena runs
out of space. Inside `arena!`, recognized `format!(...)` calls lower to this
fallible form.

```rust
use compact_std::prelude::*;

StdArena::with_capacity(16 * 1024, |arena| {
    let mut counts = HashMap::new();
    counts.insert(42_u32, 1_u32, arena)?;
    counts.insert(42, 2, arena)?;
    assert_eq!(counts.get(&42, arena)?, Some(&2));
    Ok::<_, CollectionError>(())
})??;
```

Borrowed `&[T]`, `&[u8]`, and `&str` views remain tied to the wrapper and
arena borrow.

## `CompactValue` safety

Custom types may implement:

```rust
unsafe impl CompactValue for MyType {}
```

only when the type satisfies the complete safety contract. In particular, a
compact value must remain valid when moved between arena slots, must not depend
on its own address or require pinning, and must be safe to destroy while the
arena backing remains alive.

An incorrect implementation can cause undefined behavior. Address-sensitive,
self-referential, or incorrectly-lifetimed pointer/reference types must not be
declared compact-safe merely because their fields happen to be movable.

See [SAFETY.md](SAFETY.md) before implementing `CompactValue` for a custom
type.

## Generated compact layouts

```rust
use compact_std::prelude::*;

const MAX_RETRIES: u64 = 7;

#[compact]
struct Job {
    #[max = MAX_RETRIES]
    retries: u64,
    active: bool,
    name: String,
}
```

`Job` remains a native logical struct. `job.compact_in(arena)` returns a
generated compact handle with checked getters and setters.

Booleans and explicitly bounded unsigned integers are packed LSB-first.
`String` payloads are copied into arena storage and borrowed back as `&str`.
`#[hot]` and `#[cold]` fields receive separate arena ranges. Fieldless enums
receive compact discriminant wrappers and can be fields in generated layouts.

`#[compact(soa)]` additionally generates a scalar-column collection; boolean
columns use a compact bit vector.

The supported struct surface is named, non-generic structs whose fields are
`bool`, fixed-width integer scalars, `String`, or a fieldless enum
implementing the generated `CompactEnum` contract. SoA supports `Copy`
scalar fields. Unsupported pointers, references, generic layouts, payload
enums, and unsupported bounds produce compile errors.

## Allocation accounting

`Arena::used_bytes()` reports the current high-water prefix of the backing.
It may decrease when released tail allocations contract the arena. Free holes
inside that prefix can already be reusable.

`Arena::remaining_bytes()` includes both unused tail capacity and reusable
free ranges.

Allocation failure is explicit and does not silently fall back to the native
heap.

## Validation

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

Macro compile-fail diagnostics live under `crates/compact_std/tests/ui`.
Release-mode benchmark instructions and current measurements are recorded in
[BENCHMARKS.md](BENCHMARKS.md).
