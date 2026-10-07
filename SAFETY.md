# Safety

This document defines the safety-relevant V2.1.0 contracts. V2.1.0 is the only
supported contract.

## Arena lifetime

All arena-owned values must be destroyed before the backing storage ceases to
exist.

The lifetime branding on `Arena`, offsets, and owning allocations is intended
to enforce this in safe Rust. Do not use unsafe code to extend an arena-backed
reference or owner beyond its actual backing lifetime.

The backing address must remain stable for the entire arena lifetime.

## Ownership

Owned arena allocations have exactly one safe owner token.

- Owner tokens are not `Copy` or `Clone`.
- Moving an owner transfers ownership.
- Dropping an owner drops its initialized values and releases its allocation.
- Safe code cannot release an arbitrary raw offset as though it were an owner.

Do not duplicate, forge, or resurrect an owner token through unsafe code.

## `CompactValue`

`CompactValue` is an unsafe trait because containers may move values between
arena slots using raw pointer reads and writes while preserving ordinary Rust
ownership.

A correct implementation must guarantee all of the following:

- the value is valid at its normal Rust alignment in arena storage;
- moving it from one valid slot to another preserves all invariants;
- it does not require a stable address or pinning;
- it does not contain self-references that become invalid when moved;
- any embedded native references have lifetimes correctly represented by the
  Rust type;
- its destructor is safe to run while the arena backing is alive;
- moving the value does not duplicate external ownership or destructor
  responsibility.

An incorrect `unsafe impl CompactValue` can cause undefined behavior.

### Typical valid shapes

Primitive scalars and generated compact handles are supported directly. A
custom aggregate can be valid when every field has move-safe ownership and the
aggregate has no address-sensitive invariant.

Example shape:

```rust
struct Record {
    count: u32,
    flags: u16,
    child: CompactOption<'static, u8>, // illustrative shape only
}
```

The exact lifetime parameters in real code must still be correct.

### Invalid without additional proof

Do not mark a value compact-safe merely because it compiles if it contains
state such as:

- pointers into itself;
- intrusive links that encode its own address;
- pinning requirements;
- callbacks or foreign objects whose validity depends on the current address;
- native references whose true lifetime is not represented in the type.

## Relocation

Collection growth performs all fallible allocation work before moving the first
element whenever possible.

Once relocation starts, element transfer uses non-fallible pointer moves. The
source initialized length is cleared before values are read so the old owner
cannot drop moved values a second time.

Do not add a new relocation path that can return an error after partially
moving ownership unless it also provides a correct rollback guard.

## Destruction and panic

Before dropping one initialized element, containers reduce the recorded
initialized prefix so unwinding cannot cause the same element to be dropped a
second time.

Custom destructors must still obey ordinary Rust safety requirements.

Applications that require abort-only behavior may choose `panic = "abort"`,
but V2.1.0 does not require every application to use abort semantics.

## Offsets and stale storage

An `Offset32<T>` is not an ownership proof.

Reusable allocation means an address can later contain unrelated data after
the original owner releases it. APIs that require ownership validate an
allocator-issued owner token rather than trusting a raw offset.

Handle types that need stale-instance protection, such as slab handles, include
allocation identity in addition to their logical slot generation.

Do not construct raw offsets through unsafe code unless all allocation,
lifetime, alignment, and type invariants are independently guaranteed.

## Threading

Arena ownership and owning compact allocations are not a cross-thread
ownership model. Do not bypass their non-thread-safe design with unsafe
`Send` or `Sync` implementations unless the entire allocator and lifetime
model has been redesigned and re-audited.

## Persistence and IPC

Do not persist raw arena bytes as a durable format.

The arena contains allocator metadata, target-native byte ordering, runtime
ownership state, and process-local representation assumptions. Raw backing
bytes are not a stable disk, IPC, network, or cross-process ABI.

Serialize logical values into a dedicated external format instead.

## Macro safety boundary

`arena!` is intentionally conservative. If it cannot prove that a receiver is
a supported compact binding, it diagnoses the ambiguity or requires explicit
`*_in` calls.

Do not weaken this behavior by guessing that unknown syntax is compact.

Explicit APIs are the correctness fallback for helper-returned values,
ambiguous control flow, mutating closure captures, and unsupported source
patterns.

## Unsafe-code review checklist

Before changing unsafe runtime code, verify:

1. allocation extent, alignment, and initialized length remain consistent;
2. ownership is neither duplicated nor lost;
3. every live value is dropped exactly once;
4. moved-from slots are never read or dropped as initialized values;
5. reused storage cannot validate a stale owner or protected handle;
6. every returned reference is bounded by the real arena and owner lifetime;
7. failure before commit leaves prior logical state valid;
8. backing storage remains stable while any branded object exists.
