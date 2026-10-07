# V2.4 safety contracts

The safe APIs rely on the invariants below. Unsafe code is concentrated in the
cage kernel and in narrowly scoped container guards/iterators.

## Compact values

Implementing `CompactValue` is unsafe. Implementors must contain no retained
native pointers or references, self-references, address-sensitive state, or
pinning requirements. They must remain valid when moved with raw reads and
writes between correctly aligned cage slots. Nested owners must also be
cage-safe, and destructors must remain valid while the process cage exists.
Integer scalars are allowed; an integer used as a disguised native pointer is
not.

The trait cannot prove these facts for manual implementations. Generated code
enforces the structural bounds it controls. `CageAllocation<T>` is non-copyable
and ties safe slice/value borrows to its owner borrow.

## Cage pointers and allocator

The cage backing is one stable raw allocation kept alive by `OnceLock`; the
runtime retains its single native base pointer and releases it with the same
eight-byte-aligned layout. The canonical resolver forms temporary pointers
from `base + offset32`; collections do not retain those pointers. Unchecked
resolvers are unsafe because the caller must prove the target is live,
initialized, in bounds, and remains owned for the returned lifetime.

`CageState` has manual `Send` and `Sync` implementations because its raw base
pointer cannot express the cage's range ownership in Rust's type system. The
allocator mutex protects allocator scalars and free-list writes. Live data
ranges are disjoint; owner headers are changed through an exclusive owner
borrow or inside an allocator critical section. Unique owners govern data
mutation, and `CageAllocation<T>` inherits thread-safety bounds from `T`
through `PhantomData`.

Allocator scalars and free-range links are protected by one mutex. No user
code or destructor runs under that lock. Free blocks contain initialized
`u32` links and lengths. General-list ranges are sorted by offset; when the
optional size-class policy is enabled, class ranges are exclusively owned by
their class, with no overlap between classes or the general list. Each class
holds at most 32 ranges. Batch release and resize drain the classes before
checking and coalescing adjacent ranges. The complete incoming batch is
validated for bounds, overlap, and accounting before allocator state is
committed. Live and free extents account for the entire high-water prefix.
There is no native allocation registry. Safe owners are valid because their
offsets are private, issued by allocation, and transferred by Rust moves.

The release collector is a stack-local array of at most 64 extent descriptors.
A thread-local cell points to it only for the outermost batched operation on
that thread; nested teardown reuses the active collector. A process-wide atomic
count is only a lookup hint: when it is zero, allocation skips TLS; when it is
nonzero, the allocation still consults only its own thread-local cell. The
scope guard clears that cell and updates the hint before its stack storage
leaves scope, then flushes pending descriptors during both normal return and
unwind. Destructors execute before each allocator transaction begins. A
contiguous tail batch is validated and contracted directly; other batches are
sorted and fully checked before free-list or class-bin links change. If a batch
fails validation while cleanup is unwinding, each remaining descriptor is
attempted individually.

A pending extent remains accounted as live and is reachable only through its
thread's stack-local collector. Allocation can claim it only when the new
request produces the same block length and satisfies alignment. The collector
removes the extent before the allocator writes a fresh header and publishes the
new owner; no global live-byte transition occurs during this reuse. Removing
the descriptor prevents a later flush from freeing it. If no exact match exists,
the normal mutex-protected allocator path runs. No other thread can inspect or
claim a pending extent.

The live header is exactly four `u32` fields: block length, alignment prefix,
capacity, and initialized count. Its start is derived from the owner offset and
prefix. Typed accesses verify the header's capacity fits the reserved block.
Offset arithmetic is checked before allocation metadata is updated.

Hot owner methods can form a temporary resolved view containing the native
data pointer and one copied header. The view carries the shared or exclusive
owner borrow in `PhantomData`, is stack-local, and cannot be stored in a compact
value. Bulk append guards publish only the successfully written initialized
prefix if an iterator or element constructor panics.

`CageAllocation<T>` receives automatic `Send`/`Sync` behavior from `T` through
its `PhantomData<T>`. Moving an owner between threads is safe when `T: Send`;
shared reads require `T: Sync`. Mutation remains governed by exclusive Rust
borrows. The allocator mutex synchronizes allocation and release, not the
contents of live values.

## Initialization, movement, and destruction

The header's initialized count is authoritative. `push` writes a slot before
publishing the new count. `pop` removes the slot from the initialized prefix
before reading it. Truncation lowers the count before each destructor call; a
guard drops the remaining prefix if a destructor panics. Owner drop keeps the
block live until all initialized values have been handled.

Relocation allocates the destination first. Once moving starts, raw reads and
writes do not invoke user code. The source count is cleared before transfer and
the destination count is published after all values are written. Collections
that store `MaybeUninit<T>` maintain their own logical initialized-slot state
and clear that state before moving or dropping an element. A deque ring move
copies at most two disjoint initialized spans into the new allocation, then
clears the source length before replacing its storage; this is a move under the
`CompactValue` relocation contract and does not require `T: Copy`.

`CompactVecDeque`, `CompactSmallVec`, and hash collections use drop guards so
one panicking destructor does not cause a later element to be dropped twice.
`CompactVec::retain` uses an in-place compaction guard for values without
destructors. If its predicate unwinds, the guard restores the kept prefix and
unvisited tail as the initialized vector contents. For drop-bearing values,
the method stages owned values in a temporary native vector; its retain logic
repairs the survivor sequence if a predicate or destructor unwinds, and a
compact-vector guard moves survivors back without allocating or dropping them
twice. Their child allocations join a thread-local release batch only after each
destructor returns. The guards hold temporary raw pointers tied to an exclusive
borrow and never store pointers in compact state.

## Scratch

Scratch is one normal cage owner. Its controller tracks a `u32` cursor and
capacity. Returned references borrow the controller. Byte slices are
zero-initialized before exposure. Typed scratch values are restricted to
`Copy + CompactValue` with alignment no greater than eight; scratch does not
accept arbitrary destructor-bearing values.

## Frozen graphs

Implementing `FrozenValue` is unsafe. Values are `Copy + Send + Sync + 'static`,
pointer-free, immutable, have no destructor obligations, and are aligned to at
most eight bytes. Their bytes must remain valid after copying to graph storage.

Frozen typed-slice, string, and byte descriptors retain only an offset and
length. Access methods take a reference to a descriptor stored inside the
graph, verify that the descriptor itself lies in the graph's owned byte range,
then validate the target range and typed alignment. This rejects a descriptor
reference from another graph without retaining graph identity. Returned
references borrow the graph, whose one cage allocation remains alive.

`FrozenGraphView` resolves the graph byte slice once and applies the same
descriptor identity, bounds, and alignment checks against that borrowed byte
range. Its lifetime is tied to the graph; it stores no persistent state in the
graph or descriptor. Hash control classification receives a bounded 16-byte
slice when the group is contiguous and uses an initialized scratch group when
it wraps or is partial. Architecture-specific classifiers therefore never
load across cage/table bounds. The portable scalar classifier defines the same
masks and remains the reference implementation.

## Collections and macros

Collection owners retain offsets, `u32` counts/indices, inline data, and
per-slab stale-handle metadata. Native references in iterators and OS/path
views are temporary borrow state. Hash table control bytes identify initialized
entries; a slot is marked vacant before its pair is moved or dropped.

`#[compact]` generated values contain packed bytes and cage-safe owners. The
macro rejects unsupported retained fields rather than introducing pointers,
arena arguments, or native allocation. `FrozenValue` derive requires every
field to implement the unsafe frozen contract.

## Native interfaces and persistence

FFI callbacks may use a native pointer only while the scoped owner borrow is
live. `FfiByteBuffer` is an explicit native copy and must be released exactly
once by its matching free function. Never pass a cage offset as a native
pointer or retain a callback pointer across owner relocation or drop.

Raw cage bytes are process-local runtime representation. They are not a file,
IPC, network, or cross-target format. Serialize logical data through a defined
external format when it must outlive the process or move between machines.
