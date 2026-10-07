# V2.3 architecture

V2.3 has one process-wide compact cage. The cage is a stable byte allocation;
all retained compact addresses are 32-bit offsets into that allocation. Native
Rust references, allocator bookkeeping, syscalls, and FFI continue to use
ordinary native pointers.

## Runtime

`CompactRuntime::init(CageConfig::new(capacity))` creates the cage once using
a process-wide `OnceLock`. The requested capacity must be at least 64 bytes and
must fit the 32-bit address domain. Initialization fails if the runtime is
already initialized, and the backing remains alive until process termination.
No public reset or teardown operation exists.

Runtime state contains the one native base pointer, capacity, and a mutex for
allocator metadata. The allocator uses a monotonic cursor and coalescing free
ranges. Allocation, resize, and release update that metadata under the lock.
Reading or mutating a live value through its unique owner does not acquire the
allocator lock. Cage exhaustion is an error; allocations never silently move
to the native heap.

## Offsets and owners

`Offset32<T>` is a four-byte non-owning descriptor. `OffsetSlice32<T>` and
`ByteRange32` store an offset and length in eight bytes. Zero is reserved for
null. Safe code cannot construct an arbitrary non-null offset; the unchecked
constructors require the caller to prove that the target is live and remains
owned.

`CageAllocation<T>` is a unique four-byte owner. It contains an offset and a
zero-sized type marker, while the allocation header stores the block extent,
alignment prefix, element capacity, and initialized length. `Option` uses the
zero offset niche and remains four bytes. Dropping an owner drops its
initialized values and returns its block to the allocator.

`CompactBox<T>` and `CompactVec<T>` each contain one optional owner and are
four bytes. `CompactVecDeque<T>` adds a head and length and is twelve bytes.
Inline-first types, hash tables, slabs, and path/string wrappers include the
extra metadata required by their behavior. Collection methods return
allocation errors explicitly.

The cage base is not stored in compact values or owners. Access resolves an
owner offset against the base in runtime state for the duration of a Rust
borrow. Compact byte offsets are not native addresses and must not cross an
FFI boundary as pointers.

## Collection storage

Owning collections keep their payloads in cage allocations. Types that need
initialized-prefix tracking use the allocation header; circular queues track
their head and logical length; hash tables use compact control bytes and
`MaybeUninit` entry slots. Hash maps use randomized SipHash keys by default.
Slab generations protect independently copyable slot handles from reuse.

`CompactValue` is the unsafe relocation contract. Values must be valid at
their normal alignment in the cage, must not depend on their address or
pinning, must not contain native pointers or references, and must be safe to
destroy while the cage exists. The caller of an unsafe implementation is
responsible for upholding these conditions.

## Scratch allocation

`ScratchRegion` owns one cage block and advances a stack-held byte cursor.
Capacity is rounded to whole `u64` slots. Byte slices are zero-filled;
`alloc_value` accepts `Copy + CompactValue` values with alignment at most
eight. Returned mutable references borrow the region, and dropping it returns
the whole block to the cage.

## Frozen graphs

`FrozenBuilder` uses a temporary native construction buffer. `finish` copies
the completed bytes into one `CageAllocation<u64>` and returns a
`FrozenGraph<T>` that owns that single block. The builder and its temporary
buffer are then dropped. Frozen descriptors contain offsets, lengths, and a
graph identity; reads validate identity, alignment, and range before forming
a reference.

Frozen data is immutable after finishing. `FrozenValue` requires `Copy`,
`Send`, `Sync`, `'static`, alignment no greater than eight, no native pointers
or interior mutability, and no destructor. The graph can therefore be shared
for lock-free reads. Unique graph identifiers prevent a descriptor from one
graph being accepted by another.

## Serde and packed layouts

Direct Serde visitors create supported compact values in the initialized
process cage. They own each partial value as it is built, so a parse or
allocation error drops the partial result. The parser helpers need no
allocation-context argument.

`#[compact]` emits packed byte storage and checked accessors for supported
structs and fieldless enums. It uses bit offsets and widths rather than
references to runtime storage. `#[compact(soa)]` can also emit primitive
column storage.

## Native boundary

An FFI callback receives a temporary native borrow resolved from the compact
owner. The pointer is valid only during that borrow and while the owner is not
relocated or dropped. `FfiByteBuffer` is an explicit native allocation for
interfaces that retain memory; it is freed through its matching library
function.

Compact cage bytes are process-local runtime representation, not a stable
serialization or cross-process ABI. Use an external serialization format for
persistent or transferred data.
