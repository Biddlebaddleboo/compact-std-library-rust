# compact-std-library-rust

`compact_std` provides familiar Rust collection APIs over a scoped compact
arena. Storage uses 32-bit byte offsets; application code keeps ordinary Rust
control flow and opts into compact layouts with procedural macros. The project
does not replace Rust's standard library or process allocator.

## Workspace crates

- `compact_core` is the backend-independent `no_std` runtime. It owns the V1
  ABI, checked allocations, offset views, initialized byte ranges, and packed
  bit access.
- `compact_backend_std` provides fixed stable memory backed by `std`.
- `compact_collections` provides `CompactBox`, `CompactVec`, `CompactString`,
  `CompactBitVec`, slabs, nullable offsets, and interning.
- `compact_macros` provides `#[compact]` and lexical `arena!` rewriting.
- `compact_std` re-exports the runtime, collections, macros, and prelude.

One arena addresses at most 2^32 bytes. Offset zero remains the null sentinel,
`Offset32<T>` remains four bytes, and the core stays backend-independent. Arena
allocations use `Copy` values so native destructors are never silently skipped.
Compact strings and collections own only arena-relative byte storage.

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

`arena!` rewrites supported constructors and arena-dependent methods only
inside its block. It does not install ambient state. `vec![]` is rejected in
an arena block because it would allocate a native `Vec`; use `Vec::new()` and
`push` instead.

`CompactVec<T>` has twelve-byte offset/length/capacity metadata and requires
`T: Copy`. Capacity doubles on growth; because a monotonic arena cannot reclaim
old buffers, total vector payload allocations remain below twice the final
capacity. `CompactSmallVec<T, N>` keeps up to `N` values in its own inline
storage, then promotes to a compact arena vector. `CompactString` stores up to
twelve UTF-8 bytes inline in a sixteen-byte handle and grows into initialized
arena byte storage. Borrowed `&[T]`,
`&[u8]`, and `&str` views are zero-copy and scoped to arena borrows.

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
`JobCompact` handle with checked getters and setters. Booleans and explicitly
bounded unsigned integers are packed LSB-first. String payloads are copied to
arena bytes and borrowed back as `&str`. `#[hot]` and `#[cold]` fields receive
separate arena ranges. Fieldless enums receive compact discriminant wrappers
and can also be fields in other generated layouts.

`#[compact(soa)]` additionally generates a scalar-column collection; boolean
columns use a bit vector. The current macro supports named, non-generic structs
whose fields are `bool`, fixed-width integer scalars, `String`, or an enum
implementing the generated `CompactEnum` contract. SoA currently supports only
`Copy` scalar fields. Unsupported pointers, references, generic layouts, and
payload enums produce compile errors. Bounds must be const
expressions; signed bounded fields need an explicit minimum and are not yet
supported.

The compact interner is optional and linearly scans compact range descriptors,
avoiding a separate hash table for small sets. `CompactSlab` uses generation-
checked handles and retires a slot before its generation can wrap.

## Validation

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features
cargo run --manifest-path fixtures/regular_rust_style/Cargo.toml
cargo run --manifest-path fixtures/macro_layouts/Cargo.toml
```

`fixtures/consumer` checks low-level imports; `fixtures/regular_rust_style` and
`fixtures/macro_layouts` exercise the facade and generated layouts. Macro
compile-fail diagnostics live under `crates/compact_std/tests/ui`.
