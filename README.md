# compact-std-library-rust

`compact_std` provides familiar Rust collection APIs backed by one process-wide
compact cage. Compact addresses are 32-bit byte offsets; Rust references and
native interfaces continue to use ordinary native pointers.

## V2.3 contract

V2.3 uses one cage address space per process. Initialize it once with an
explicit capacity before creating compact values:

```rust
use compact_std::prelude::*;

fn main() -> Result<()> {
    CompactRuntime::init(CageConfig::new(64 * 1024 * 1024))?;

    let mut values = Vec::new();
    values.push(10_u32)?;
    values.push(20_u32)?;
    assert_eq!(values.get(1), Some(&20));

    let text = String::from_str("compact cage")?;
    assert_eq!(text.as_str(), "compact cage");
    Ok(())
}
```

Initialization is one-time and the cage lives until process exit. The runtime
does not reserve the full 4 GiB offset range automatically. Allocation-bearing
operations return errors when the configured cage is exhausted; compact
collections do not fall back to the native heap.

The workspace crates are:

- `compact_core`: `no_std` offsets, ABI definitions, and packed-field
  primitives.
- `compact_backend_std`: process runtime, allocator, four-byte owners, and
  scratch regions.
- `compact_collections`: compact owners and collection types.
- `compact_serde`: direct Serde construction into compact values.
- `compact_frozen`: immutable graphs in one cage allocation.
- `compact_macros`: packed layouts and compact deserialization derives.
- `compact_std`: the convenience facade and prelude.

## Memory model

`Offset32<T>`, `OffsetSlice32<T>`, and `ByteRange32` store offsets and lengths,
not native pointers. Offset zero is reserved as null. A process runtime holds
the cage base once; an offset is resolved only while an owner or graph keeps
its allocation alive.

`CageAllocation<T>` is a unique, non-copy owner represented by one four-byte
offset. It tracks capacity and initialized length in allocation metadata,
runs destructors exactly once, and returns released blocks to the synchronized
cage allocator. `CompactBox<T>` and `CompactVec<T>` are four bytes. Other
collections add only the metadata their layout requires.

Compact storage is an in-process runtime representation. It is not a durable
file, IPC, network, or cross-target format. Serialize logical data into an
external format when it needs to outlive the process or move between systems.

## Collections

The facade exposes compact `Box`, `Vec`, `String`, `VecDeque`, `HashMap`,
`HashSet`, `OsString`, and `PathBuf` aliases, alongside explicit `Compact*`
types. Allocation-bearing operations are fallible. `CompactBytes` stores short
payloads inline, while `CompactString` uses small-string storage. The crate
also provides `CompactSmallVec`, `CompactRing`, `CompactBitVec`,
`CompactSlab`, and `CompactInterner`.

The default map and set hasher is randomized. A custom `BuildHasher` can be
provided when needed. Slab handles use slot generations to reject stale
handles after slot reuse.

## Packed layouts

`#[compact]` generates a packed companion type and checked accessors without
runtime allocation parameters:

```rust
use compact_std::compact;

#[compact]
struct RetryState {
    enabled: bool,
    #[max = 7]
    retries: u8,
}

let mut packed = RetryState { enabled: true, retries: 3 }.compact()?;
packed.set_retries(4)?;
assert!(packed.enabled()?);
```

The macro supports named, non-generic structs with supported scalar, enum,
and string fields, fieldless enums, hot/cold sections, and primitive-column
SoA generation with `#[compact(soa)]`. Unsupported layouts fail at compile
time.

## Direct Serde

Enable the optional parser features as needed:

```toml
compact_std = { version = "2.3.0", features = ["json", "toml"] }
```

`json::from_str`, `json::from_slice`, and `toml::from_str` build supported
compact fields directly in the initialized cage. `#[derive(CompactDeserialize)]`
supports named structs and unit enums. It accepts field `rename`, `default`,
`skip`, and `skip_deserializing`; struct `rename_all` and
`deny_unknown_fields`; and enum-variant `rename` with container `rename_all`.
Unsupported attributes produce a compile error. Partial values are dropped if
deserialization fails.

## Scratch regions

`ScratchRegion` reserves a bounded block from the same cage and provides
zero-filled byte slices or aligned `Copy + CompactValue` values. The region
releases its block when dropped. Scratch references borrow the region and
cannot outlive it.

## Frozen graphs

Frozen graphs are immutable, shareable values owned by one cage allocation.
Build descriptors and a root, then finish the graph:

```rust
use compact_std::{CageConfig, CompactRuntime, FrozenBuilder, FrozenValue, FrozenVec};

#[derive(Clone, Copy, FrozenValue)]
struct Catalog { ids: FrozenVec<u32> }

fn frozen_example() -> std::result::Result<(), std::boxed::Box<dyn std::error::Error>> {
    CompactRuntime::init(CageConfig::new(1024 * 1024))?;
    let mut builder = FrozenBuilder::new()?;
    let ids = builder.store_slice(&[3_u32, 5, 8])?;
    let graph = builder.finish(Catalog { ids })?;
    assert_eq!(graph.slice(graph.root().ids)?, &[3, 5, 8]);
    Ok(())
}
```

Frozen descriptors validate their graph identity and bounds. Frozen values
must be copyable, pointer-free, immutable, and have alignment no greater than
eight bytes. A completed graph can be shared for concurrent reads when its
root satisfies the `FrozenValue` contract.

## Native interfaces

`CompactBytes::with_ffi_bytes` and the corresponding string and path views
resolve a native pointer only for the duration of a callback. Do not retain
that pointer beyond the borrow or across an operation that may relocate the
owner. `FfiByteBuffer` makes an explicit native copy for APIs that must retain
memory; the caller releases it with the matching free function.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the representation and allocator,
[SAFETY.md](SAFETY.md) for unsafe-code contracts, and
[BENCHMARKS.md](BENCHMARKS.md) for benchmark status and commands.
