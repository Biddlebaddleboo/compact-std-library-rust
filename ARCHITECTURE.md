# V2.4 architecture

## Freeze status

The retained architecture is frozen after the native comparison and
validation pass documented in [BENCHMARKS.md](BENCHMARKS.md). Changes to the
pointer model, retained owners/headers, or process-wide cage contract are
scoped to V3. Bounded global allocator metadata and allocator-policy
optimizations remain within V2.4 when they preserve those retained layouts and
ownership rules.

V2.4 keeps a normal 64-bit Rust process and one stable process-wide cage.
Native references, `usize`, libc, syscalls, and third-party dependencies keep
the platform ABI. Retained cage addresses, owners, links, and descriptors use
32-bit offsets or counts.

## Runtime and allocator

`CompactRuntime::init` creates the cage once through a process-wide `OnceLock`.
The backing allocation stays at one address until process exit; there is no
reset or public teardown. Runtime state retains one native base pointer, its
configured capacity, and one mutex protecting allocator scalars. Reading live
values does not acquire that mutex.

The allocator contains a `u32` high-water cursor, `u32` live-byte count, an
ordered general free-list head, and four bounded exact-size class heads for
32, 40, 112, and 528-byte blocks. Every class can hold at most 32 blocks.
Free blocks hold an eight-byte `{ next, len }` node at their start. The general
list is sorted by offset and remains the first-fit fallback. A class owns its
blocks exclusively; a batch release or resize drains the classes before
coalescing adjacent ranges, then may cache exact-size results again. The tail
contracts once after a batch. There is no native map, set, or heap allocation
for allocator bookkeeping.

`CompactRuntime::with_batched_releases` collects up to 64 temporary extents on
the stack and releases each chunk under one `AllocatorTransaction`. Nested
teardown joins the same thread-local collector. Contiguous tail chunks contract
directly; other chunks are sorted, checked against current free ranges, and
coalesced before allocator state changes. The optional `allocator-telemetry`
feature adds counters to global runtime state and benchmark output only; it
does not change retained owner or header layouts.

The common live header is four `u32` fields and is exactly 16 bytes:

| Field | Meaning |
| --- | --- |
| `block_len` | Full reserved extent, including alignment prefix |
| `prefix` | Bytes from block start to the header |
| `capacity` | Typed element capacity |
| `initialized` | Initialized element count |

The block start is derived from the data offset, header size, and prefix. The
header stores no magic value, native pointer, or redundant block-start offset.
The cage config is limited to the `u32` cursor range; the compact offset domain
itself remains 32 bits.

## Pointer resolution and ownership

The kernel has one conversion from a cage offset to a temporary native pointer:

```text
retained:  offset32
borrowed:  cage_base + offset32 -> pointer/reference
after:     no pointer retained by cage-aware state
```

`CageAllocation<T>` stores one nonzero offset and a zero-sized type marker. It
is non-copyable, so Rust move semantics transfer its unique ownership. Its
capacity and initialized length live in the allocation header. `Drop` lowers
the initialized count before each destructor call, then returns the block
through the active release batch or the allocator's free structures.

Safe owner methods tie returned references to the owner borrow. The public
unchecked offset resolvers are unsafe and require the caller to prove liveness,
initialization, bounds, and the returned borrow lifetime. Compact offsets are
not C, Swift, JNI, or syscall pointers.

## Representation table

| Type | Size |
| --- | ---: |
| `Offset32<T>` | 4 B |
| `OffsetSlice32<T>` | 8 B |
| `ByteRange32` | 8 B |
| `CageAllocation<T>` | 4 B |
| `Option<CageAllocation<T>>` | 4 B |
| `CompactBox<T>` | 4 B |
| `CompactVec<T>` | 4 B |
| `CompactVecDeque<T>` | 12 B |
| `AllocationHeader` | 16 B |
| `FreeNode` | 8 B |
| `FrozenVec<T>` | 8 B |
| `FrozenString` | 8 B |
| `FrozenBytes` | 8 B |

`usize` remains valid for ordinary scalar values and temporary indexing. The
rule applies to retained addressing and links, not every integer field.

## Collections

Vectors keep capacity and initialized length in their allocation header, so
the owner remains four bytes. Deques add a `u32` head and length. Hash tables
keep controls and entries in cage allocations; their length and tombstone
counts are `u32`. The randomized SipHash builder retains only two integer keys.

Small vectors keep an inline initialized count as `u32`; their inline array is
part of the wrapper. Strings and bytes use inline payloads, then promote to a
four-byte cage owner. OS strings and paths store exact platform bytes in
`CompactBytes`; native `OsString`/`PathBuf` values are temporary conversions.

Slab handles contain a slot index, generation, and slab identity, all `u32`.
The generation rejects stale handles after slot reuse. The slab ID remains
because handles are copyable and can be presented to any slab; it rejects a
valid-looking handle from another slab. This identity is local to slabs and is
not attached to ordinary cage owners.

## Scratch regions

`ScratchRegion` owns one block in the same cage. Its controller stores a `u32`
cursor and capacity. It returns zero-filled byte slices or aligned
`Copy + CompactValue` values with alignment at most eight. Returned borrows
cannot outlive the controller. Scratch does not run destructors for arbitrary
values.

## Frozen graphs

`FrozenBuilder` uses a temporary native `Vec<u64>` and copies the completed
graph into one cage allocation. The retained typed-slice, string, and byte
descriptors each contain only `{ offset: u32, len: u32 }`.

Graph reads accept a reference to a descriptor stored inside that graph. The
kernel checks that the descriptor reference lies within the graph's owned byte
range, then checks its target range and alignment. This temporary in-graph
reference provides graph identity without storing a graph ID in each
descriptor. A copied descriptor outside the graph is rejected, as is a
reference to a descriptor in a different graph. Returned data borrows the
`FrozenGraph` and can be shared for immutable parallel reads.

`FrozenValue` remains an unsafe contract: values are `Copy + Send + Sync +
'static`, pointer-free, immutable, have no destructor, and are aligned to at
most eight bytes. The derive enforces the field trait bounds for generated
types.

## Serde, macros, and native boundaries

Direct Serde visitors construct compact fields in the initialized process
cage. Partial values own their allocations and are dropped on parse or
allocation errors. Parser APIs need no runtime argument.

`#[compact]` emits packed bytes and checked accessors. It does not rewrite
lifetimes or add arena/runtime parameters. Generated retained state contains
compact values, offsets, packed scalars, or inline payloads.

FFI callbacks receive temporary native borrows resolved from an owner. A
foreign caller that needs retained memory receives an explicit native copy
through `FfiByteBuffer`; a cage offset is never exported as a pointer.

## Portable implementation

The V2.4 retained architecture remains frozen. Implementation hot paths use
temporary borrow-bound resolved views, batch append writers, and direct deque
iteration without adding state to retained owners. Hash control-byte probing
keeps the `EMPTY`/`FULL`/`TOMBSTONE` format and has a portable scalar reference,
an AArch64 NEON classifier, and an x86-64 SSE2 classifier. The SIMD code only
classifies copied 16-byte groups; probing order and key equality remain in
ordinary Rust. No inline assembly is used.

Compact cage bytes are process-local runtime representation, not a stable file,
IPC, network, or cross-target format. Use an external serialization format for
data that must persist or move between processes or machines.
