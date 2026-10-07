# Safety contracts

This document describes the unsafe invariants for the V2.3 process-wide cage.
The crate-level APIs are safe only while these invariants hold.

## Compact values

Implementing `CompactValue` is unsafe. A value must remain valid when moved
between aligned cage slots with raw reads and writes. It must not be
self-referential, address-sensitive, or require pinning. It must not store
native pointers or references. Its destructor must be valid while the process
cage remains alive.

Raw pointers and references are not compact offsets. Store an `Offset32<T>` or
another compact descriptor when a value needs to name cage data. Resolve that
offset only while an owner or graph holds the target alive.

## Runtime and allocator

The process runtime is initialized once. Its backing allocation must remain at
the same address until process termination. The runtime owns the sole retained
native cage base pointer. Cage capacity is bounded by the 32-bit offset space,
and offset zero is reserved.

Allocator metadata is protected by one mutex. No user code or value destructor
runs while the allocator lock is held. A live owner exclusively controls its
allocation header's initialized count and contents. Shared reads are permitted
only through shared borrows; mutation requires the unique owner borrow.

An allocation offset is issued only after the allocator reserves a complete
block and writes its aligned header. The owner cannot be safely copied. On
drop, it lowers the initialized count before each destructor call and releases
the block exactly once. Free ranges are merged only after the owner has
finished destruction.

The process never moves or resets cage storage. Consequently a safe reference
formed from an owner remains backed for the reference lifetime. Unsafe offset
resolution functions require callers to prove that an offset names an
initialized value or byte range, that it is in bounds, and that its owner
remains alive and borrowed for the returned lifetime.

## Owner transfer and destruction

`CageAllocation<T>` uses raw reads and writes to move initialized values
between cage slots. Every move clears the source initialized count before
transferring elements, then records the destination count after all writes.
Operations that can invoke user destructors use guards so unwinding does not
drop an element twice or leave later initialized elements untracked.

Collections that store `MaybeUninit<T>` maintain their own initialized-slot
state. Their `Drop` implementations clear each logical slot before dropping
its value. Relocation allocates replacement storage first, then moves values
without invoking user code between source removal and destination
initialization.

Compact collection wrappers may themselves be stored as `CompactValue` only
because their fields are compact offsets/scalars or inline `CompactValue`
elements. Their destructors retain sole ownership of nested allocations.

## Scratch regions

Scratch storage is one ordinary cage allocation. Each returned mutable slice
or value is bounded by a mutable borrow of its `ScratchRegion`; it cannot
outlive the region through safe code. Scratch byte storage is initialized
before returning a byte slice. Typed scratch values must be `Copy`, satisfy
`CompactValue`, fit in the reserved extent, and require alignment no greater
than the `u64` backing alignment.

Scratch does not run destructors for arbitrary stored values. Its typed API is
restricted to `Copy` values, and byte access returns initialized bytes.

## Frozen graphs

Implementing `FrozenValue` is unsafe. Values must be `Copy + Send + Sync +
'static`, pointer-free, immutable, free of destructor obligations, and aligned
to at most eight bytes. Their bytes must represent valid values after copying
into the graph's aligned storage. The derive requires every field type to
implement `FrozenValue`, in addition to the root type meeting the trait's
supertrait bounds.

Frozen descriptors include a graph identifier, offset, and length. Graph
reads check that the identifier matches, arithmetic does not overflow, the
range stays inside the completed graph, and typed offsets satisfy alignment
before creating a reference. Graph storage is immutable after `finish` and is
released when its unique `FrozenGraph` owner is dropped.

## Packed values and macros

`#[compact]` accepts only field types for which its generated representation
has a defined encoder and decoder. Accessors bounds-check bit ranges, validate
declared maxima, and reject invalid enum discriminants. The macro does not
emit native pointers. Its generated compact companion types implement
`CompactValue` because they contain packed bytes and supported compact string
owners only.

## FFI

Native pointers returned by scoped FFI callbacks are valid only for the
callback's borrow. The caller must not retain them, free them, or use them
after the owner is dropped or relocated. `FfiByteBuffer` owns a native `Vec`
allocation and must be released exactly once with the matching free function.
Never pass a cage offset as a native pointer.

## Persistence

Raw cage bytes are not a persistent file, IPC, network, or cross-process
format. They contain runtime layout and target-native representations. Use a
defined external serialization format when data must survive process exit or
move across process or machine boundaries.
