# Architecture

This document describes the current V2.1.0 architecture. V2.1.0 is the only
supported contract.

## Addressing

Compact arena references use 32-bit byte offsets.

- `Offset32<T>` is four bytes.
- Offset zero is reserved as the null sentinel.
- One arena addresses at most 2^32 bytes.
- Resolution is arena base plus checked offset.
- Normal dereference does not require a global registry, hash table, or
  thread-local current arena.

Packed fields use LSB-first bit numbering. Multi-byte packed words use the
target's native byte order.

Arena bytes are runtime memory, not a persistent or cross-target storage
format.

## Stable backing and arena state

The backing address must remain stable for the arena lifetime.

Allocator state is stored inside the backing so owning tokens can reclaim
storage even if the stack `Arena` value itself moves. The allocator tracks the
backing, current high-water cursor, sorted free list, and next allocation
identity.

The minimum usable backing is `MIN_ARENA_BYTES`.

### Scratch scopes

`Arena::scratch(capacity, callback)` reserves a parent-owned byte allocation
and initializes a separate nested arena inside it. The nested arena has its own
allocator state, so releasing the scratch block cannot invalidate unrelated
parent allocations. Nested scratch calls are supported. Ordinary compact
owners run their destructors before the nested backing is released, and the
callback lifetime prevents scratch-branded values from escaping.

### Persistent `CompactStore`

`CompactStore` owns a fixed `StdBacking` and initializes a separate persistent
header containing a magic value, ABI version, capacity, and validity marker.
Every callback validates this header and the allocator free list before
reattaching. Ordinary `with_arena` scopes keep their existing fresh-state
layout and behavior.

The store keeps only a private raw root offset. `with` and `with_mut` create a
freshly branded `RootHandle` after reattachment and validate it before calling
user code. The initial `StoreRoot` contract is limited to `Copy +
CompactValue + 'static`, so roots have no destructor to skip when their
generative lifetime is erased. Arena-branded references and handles cannot
escape either callback. Dropping the store releases the whole backing.

## Allocation

New tail allocations use a bump-style path. Released allocations become
reusable ranges stored in arena memory.

The free list is:

- sorted by offset;
- first-fit;
- coalesced on release;
- stored without one native heap allocation per free range.

If a released range reaches the current tail, the high-water cursor contracts.
Adjacent tail free ranges are folded into that contraction.

Owned allocations have internal metadata for block extent, alignment prefix,
element capacity, and initialized length. These fields are implementation
details and are not part of the stable source contract.

## Resize

An owned allocation can grow without moving when:

- it is at the current tail and backing capacity permits growth; or
- the immediately following free range is large enough.

Shrinking can return a sufficiently large suffix to the free list.

Collections must use allocator APIs rather than manipulate free-list metadata
directly.

## Ownership

`ArenaAllocation<T>` is the unique owner of one typed arena allocation. It is
non-`Copy` and non-`Clone`.

Dropping the token:

1. drops every value in the initialized prefix exactly once;
2. returns the allocation to the reusable allocator.

Moving the token transfers ownership.

Safe release is therefore coupled to an allocator-issued owner token rather
than an arbitrary `(offset, length)` supplied by the caller.

`Offset32<T>` is a reference-like offset, not an owning token.

## Generic compact values

Owning generic containers require `T: CompactValue`.

The unsafe contract requires values to remain valid when moved between compact
slots with Rust move semantics. Address-sensitive, pinned, and
self-referential values are outside that contract unless their implementation
can independently prove the required invariants.

See [SAFETY.md](SAFETY.md).

## Collections

### CompactVec

`CompactVec<T>` owns one optional `ArenaAllocation<T>`.

Growth:

1. computes required capacity;
2. tries in-place resize;
3. allocates replacement storage if needed;
4. moves the initialized prefix;
5. transfers ownership to the replacement.

Allocation failure occurs before element movement, so the existing value
sequence remains intact.

`pop` moves the final element out. `truncate`, `clear`, and drop destroy
removed elements exactly once.

### CompactString

Strings up to twelve UTF-8 bytes stay inline. Longer strings use an owned arena
byte allocation.

`clear` retains heap capacity. `shrink_to_fit_in` can release unused
capacity or return to inline representation.

### CompactSmallVec

Initialized values stay in inline storage until promotion. Promotion allocates
first and then moves each initialized value into arena storage.

### CompactBytes

`CompactBytes` keeps twenty bytes inline. This uses the same 24-byte wrapper
size as 12- and 16-byte candidates on the measured 64-bit target; a 24-byte
inline payload increased the wrapper to 32 bytes. Longer payloads use one
reclaimable `ArenaAllocation<u8>`. The benchmark shows heap-backed compact
buffers can be slower than native `Vec<u8>` for short-lived payloads, so they
are intended where inline payloads or arena ownership are useful.

### CompactVecDeque and CompactRing

The deque tracks a physical head and logical length over one
`ArenaAllocation<MaybeUninit<T>>`. Growth first makes wrapped values
contiguous, then extends the allocation in place or moves the initialized
prefix to a replacement. `make_contiguous` rotates the slots and returns a
mutable slice. Zero-sized entries use the same finite logical capacity and
drop rules as other entries.

`CompactRing` allocates its full maximum length at construction. A full ring
removes and drops the oldest entry before writing into that slot. If the
removed entry's destructor panics, the entry remains removed and the new value
is not inserted.

### CompactSlab

A slab combines slot generations with the owning allocation identity. A handle
from another slab or from storage that has been released and reused is
rejected.

### CompactInterner

The interner owns each canonical byte sequence and searches entries linearly.
It is intended for small intern sets where a hash table would cost more
metadata than it saves.

## Packed access

Common packed reads and writes operate on byte spans and scalar words rather
than looping one bit at a time. A field that needs nine bytes, such as a
64-bit value starting at a nonzero intra-byte offset, uses a correctness-first
fallback.

The public bit layout remains LSB-first.

## Macro architecture

`arena!` performs conservative lexical source analysis at compile time. It
tracks compact, native, moved, ambiguous, and unknown bindings across lexical
scopes.

It intentionally does not try to become a Rust type checker. Arbitrary helper
return types are resolved through explicit type annotations or explicit
`*_in` APIs.

Ambiguity is diagnosed rather than guessed.

## Threading

Arena ownership is intentionally single-threaded. The runtime does not install
a global or thread-local current arena and does not use cross-thread ownership
as part of the V2.1.0 contract.

## What may change within 2.x

The following are implementation details and may change while preserving the
V2.1.0 source contract:

- owner-token physical size;
- collection handle physical size;
- allocation-header representation;
- free-list representation and search strategy;
- coalescing implementation;
- packed-field fast paths;
- macro internal analysis representation.

The four-byte `Offset32<T>` representation and documented V2.1.0 source
behavior are part of the supported contract.
