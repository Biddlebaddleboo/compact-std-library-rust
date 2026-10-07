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

Native `vec![]` is rejected inside `arena!` because it would allocate a
native `Vec`. Use `Vec::new()` plus `push`, or call explicit `*_in`
methods when the macro cannot prove a receiver is compact.

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

Read-only closure captures can use supported compact values. Moving or mutating
captured compact owners through macro sugar is rejected; use explicit `*_in`
APIs instead.

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
