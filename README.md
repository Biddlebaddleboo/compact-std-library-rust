# compact-std-library-rust

V1 provides a backend-independent, `no_std` compact arena core and a hosted
adapter using ordinary Rust `std` allocations. It is a memory runtime
foundation; it does not replace Rust's standard library or process allocator.

## Workspace crates

- `compact_core` defines the V1 ABI, scoped arena, four-byte offsets, native
  borrows, checked layout helpers, and packed `u8`/`u16`/`u32`/`u64` fields.
- `compact_backend_std` provides fixed stable backing storage and a closure
  API for hosted applications.

One arena can address at most 2^32 bytes. Offset zero is reserved as null;
allocations use byte offsets and account for alignment. Offset zero is
reserved, so the smallest usable backing is two bytes. Arena use is scoped to
a callback, so safe offsets cannot be resolved through another arena. Core
allocations accept `Copy` values and never run destructors. Uninitialized
allocations are represented as `MaybeUninit<T>` until explicitly written.

Arena bytes are an in-memory runtime representation. V1 does not define a
persistent file format, cross-process representation, or cross-target packed
word byte order. Packed fields use LSB-first bit numbering within native
numeric words.

## Hosted example

```rust
use compact_backend_std::StdArena;
use compact_core::{Offset32, Result};

let result = StdArena::with_capacity(4096, |arena| -> Result<u32> {
    let value: Offset32<'_, u32> = arena.alloc_value(41)?;
    *arena.get_mut(value)? += 1;
    Ok(*arena.get(value)?)
})??;

assert_eq!(result, 42);
```

`StdBacking` can also own memory across multiple scoped arena operations:

```rust
use compact_backend_std::StdBacking;

let mut backing = StdBacking::with_capacity(4096)?;
let value = backing.with_arena(|arena| {
    let offset = arena.alloc_value(42_u32)?;
    Ok::<_, compact_backend_std::CoreError>(*arena.get(offset)?)
})??;
```

The standard backend uses a fixed `Vec<MaybeUninit<u8>>` allocation. Moving the
owner does not move its heap buffer; it never grows after construction. The
logical V1 limit is 4 GiB, but an allocation still depends on host address
space and allocator availability. This portable backend does not reserve huge
sparse virtual ranges, so large requests may consume substantial virtual or
physical memory depending on the platform allocator.

## Cargo dependencies

Path dependencies during local development:

```toml
[dependencies]
compact_core = { path = "../compact-std-library-rust/crates/compact_core" }
compact_backend_std = { path = "../compact-std-library-rust/crates/compact_backend_std" }
```

Git dependencies before a registry release:

```toml
[dependencies]
compact_core = { git = "https://github.com/Biddlebaddleboo/compact-std-library-rust", package = "compact_core" }
compact_backend_std = { git = "https://github.com/Biddlebaddleboo/compact-std-library-rust", package = "compact_backend_std" }
```

The crates are not claimed to be published on crates.io. `fixtures/consumer`
is a standalone Cargo project that checks the public imports and hosted use
without relying on workspace-private APIs.
