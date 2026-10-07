# Workstream Plan: compact_core

## Objective

Implement the backend-independent V1 compact-memory foundation.

`compact_core` defines the compact ABI, arena safety model, canonical 32-bit offset model, packed scalar primitives, and native borrowed-view primitives that all future containers, macros, and execution backends will consume.

The crate must compile as `#![no_std]` and as a normal publishable Cargo library.

## Starting files and symbols

All paths and symbols below are proposed additions because the repository had no implementation before the planning set.

Create:

```text
crates/compact_core/Cargo.toml
crates/compact_core/src/lib.rs
crates/compact_core/src/abi.rs
crates/compact_core/src/backing.rs
crates/compact_core/src/arena.rs
crates/compact_core/src/offset.rs
crates/compact_core/src/layout.rs
crates/compact_core/src/packed.rs
crates/compact_core/src/native.rs
crates/compact_core/src/error.rs
```

Proposed primary symbols:

- `CompactAbiVersion` or equivalent V1 ABI marker;
- `BackingRegion` / `StableBacking` trait or equivalent minimal backend contract;
- `Arena`;
- `Offset32<T>`;
- checked allocation/layout helper(s);
- `BitField`/packed-word helper(s);
- bit-width/storage-word selection helpers;
- native view/resolution methods;
- compact error enum.

Names may be adjusted for idiomatic Rust, but the responsibilities and invariants must remain.

## Verified repository facts

- The repository was empty before planning.
- No prior ABI, allocator, trait, or public API exists.
- No compatibility migration is required.
- V1 can establish the canonical compact memory rules deliberately.
- The authoritative plan fixes the general arena reference model to a 32-bit byte offset and a <=4 GiB arena.
- Backend independence is a hard requirement.

## Write scope

This workstream owns only:

- `crates/compact_core/**`;
- any root Cargo workspace entry strictly required to register this crate, coordinated through the integrator;
- core-focused tests owned by this crate.

Do not modify the std backend implementation except for coordinated interface changes approved centrally.

## Read-only dependencies

Read:

- `PLAN.md`;
- `PLAN_BACKEND_STD.md`;
- root workspace metadata once created.

Do not broadly search unrelated code unless repository architecture changed after the plan was committed.

## Required behavior

### 1. no_std foundation

`compact_core` must use `#![no_std]`.

Do not add a hidden default `std` feature that changes core semantics.

Use `core` facilities only unless an optional future extension is explicitly introduced by a later plan.

### 2. ABI V1 marker and documentation

In `abi.rs`, define/document V1 representation rules.

At minimum specify:

- offset payload width: 32 bits;
- offset unit: bytes;
- arena maximum span;
- chosen null/sentinel convention;
- bit numbering convention;
- packed-word interpretation;
- alignment rules;
- whether endianness affects any externally visible representation;
- what is stable ABI versus implementation detail.

Expose a version marker/constant suitable for later compatibility checks.

Do not promise persistent/on-disk binary compatibility in V1.

### 3. Backend memory contract

Define the smallest safe contract by which an external backend can supply an arena with stable backing memory.

The contract must allow CORE to know:

- base address;
- usable byte length;
- backing lifetime;
- stability/non-relocation guarantee;
- alignment validity.

Do not expose:

- `std::alloc` types if `core::alloc` suffices;
- mmap;
- VirtualAlloc;
- file descriptors;
- OS handles;
- allocator-specific policy.

Prefer an ownership model that makes premature backing destruction impossible or statically constrained.

### 4. Arena

Implement a fixed-capacity arena over one stable backing region.

V1 may use monotonic/bump allocation.

The arena must:

- validate backing capacity against the V1 address domain;
- maintain checked allocation cursor arithmetic;
- honor requested alignment;
- refuse allocations that cannot be represented;
- never relocate issued storage;
- expose deterministic used/remaining/capacity metrics;
- clearly define zero-sized allocation behavior;
- clearly define exhaustion behavior;
- prevent safe resolution of bytes outside the arena.

Do not add per-object free unless correctness requires it.

### 5. Offset32<T>

Implement the canonical general-purpose compact reference.

Representation requirements:

- exactly 4 bytes;
- type marker must add no stored size;
- no native pointer;
- no arena pointer;
- no backend identity stored per reference;
- no mandatory global/table lookup for dereference.

Construction:

- safe constructors only from validated arena allocations or validated offsets;
- arbitrary raw construction must be restricted or unsafe;
- define nullable/sentinel semantics explicitly.

Resolution:

- always requires an arena/context capable of proving the offset belongs to the accessible region;
- immutable resolution produces a lifetime tied to the arena/backing borrow;
- mutable resolution requires exclusive access sufficient to preserve Rust aliasing;
- alignment and bounds invariants must be checked or previously established.

Do not implement `Deref` if it would require hidden ambient arena context or weaken lifetime guarantees.

### 6. Arena identity and cross-arena misuse

A four-byte `Offset32<T>` cannot carry a full arena identity.

Design safe APIs so using an offset with an unrelated arena cannot become memory-unsafe.

Options may include:

- only producing/resolving offsets through arena-scoped capabilities;
- validation against allocation metadata in debug/testing layers;
- lifetime/generative arena tokens if practical.

Do not add a per-reference native arena pointer merely to solve identity.

Document residual logical misuse that cannot be prevented without increasing representation size, but safe APIs must still prevent UB.

### 7. Initialization and Drop

Do not expose initialized references to uninitialized storage.

Explicitly decide and document V1 behavior for destructors.

Acceptable initial designs include:

- safe typed allocation only for values that can be bulk-discarded without requiring `Drop`; or
- destructor registration with reliable reverse-order/defined cleanup.

Do not present ordinary ownership semantics for `Drop` types while silently skipping destructors.

If V1 constrains safe typed storage to non-dropping data, expose that limitation clearly and leave an extension point for future destructor-aware arenas.

### 8. Packed scalar primitives

Implement efficient packed field helpers over:

- `u8`;
- `u16`;
- `u32`;
- `u64`.

Required operations:

- define/validate an offset + width;
- extract field;
- checked insert;
- preserve all neighboring bits;
- read/write one-bit booleans;
- handle full-width fields without invalid shifts;
- reject a value that does not fit rather than truncating.

Favor const-generic descriptors where this stays readable and stable-Rust compatible, but do not force const generics if they complicate the V1 API.

### 9. Automatic-layout support primitives

Provide pure helpers needed by future macros/layout generators.

At minimum:

- bits required for a proven unsigned maximum;
- smallest backing word selection;
- checked alignment helpers;
- bit-range validation.

Boundary expectations:

```text
max 0       -> explicitly defined convention
max 1       -> 1 bit
max 3       -> 2 bits
max 7       -> 3 bits
max 255     -> 8 bits
max 256     -> 9 bits
max 65535   -> 16 bits
max 65536   -> 17 bits
u32::MAX    -> 32 bits
u64::MAX    -> 64 bits
```

Storage word selection:

```text
1..=8   -> u8
9..=16  -> u16
17..=32 -> u32
33..=64 -> u64
```

Do not implement arbitrary multi-word layout synthesis in V1.

### 10. Boolean-packing foundation

A compact boolean logically occupies one bit.

Provide primitives that allow future generated layouts to group booleans into backing words without corrupting neighboring fields.

For exclusive/non-concurrent storage, generated operations should be able to compile to simple masks/shifts/read-modify-write.

Do not make all packed fields atomic.

If V1 exposes atomic helpers at all, keep them separate from ordinary packed words and only use target-supported atomic widths. Otherwise defer concurrent packed mutation while documenting the extension point.

### 11. Native borrowed views

Provide the low-level APIs needed for the native boundary.

Where initialized contiguous representation is native-compatible, allow zero-copy:

- `&T`;
- `&mut T` with exclusivity;
- `&[T]`;
- `&mut [T]` with exclusivity.

Raw native pointers must be clearly unsafe or otherwise constrained.

Returned native references must never outlive the arena/backing.

Do not attempt owned `std::Vec`/`String` conversion in this crate.

## Error model

Create a small `no_std` error enum.

Cover at least:

- backing too large;
- zero/invalid capacity where disallowed;
- allocation exhausted;
- offset overflow;
- alignment error;
- invalid offset;
- out-of-bounds resolution;
- invalid bit range;
- packed value does not fit;
- initialization/layout violation.

Do not allocate error strings.

## Concurrency

V1 CORE should not imply thread-safe mutation unless explicitly proven.

Audit auto-traits:

- do not manually implement `Send`/`Sync` merely for convenience;
- ensure backing/arena types only acquire these auto-traits when their invariants support them;
- packed ordinary words remain non-atomic.

Future atomic packed-field types must layer on top without changing V1 non-atomic representations.

## Persistence / crash recovery

Out of scope.

V1 arena bytes are in-memory runtime representation only.

Do not claim:

- restart persistence;
- cross-process portability;
- stable disk serialization;
- architecture-independent snapshots.

## Unsafe-code requirements

Every unsafe block must state:

- bounds precondition;
- alignment precondition;
- initialization state;
- aliasing/exclusivity requirement;
- lifetime relationship;
- backing stability assumption.

Keep unsafe code concentrated in small implementation modules.

Safe APIs must not permit:

- reading uninitialized bytes as `T`;
- forged arbitrary valid offsets;
- offset arithmetic wraparound;
- overlapping mutable references;
- lifetime escape;
- use after backend drop.

## Tests

### Representation tests

Assert:

```rust
size_of::<Offset32<u8>>() == 4
size_of::<Offset32<u64>>() == 4
```

and analogous representative types.

If V1 promises an `Option<Offset32<T>>` niche size, assert it. Otherwise explicitly do not promise it.

### Arena tests

Cover:

- smallest supported backing;
- first allocation;
- multiple allocations;
- alignment 1 and representative larger powers of two;
- exact-capacity allocation;
- one-byte-over exhaustion;
- arithmetic near `u32` boundaries without allocating huge physical memory where possible via test backing;
- zero-sized allocation;
- multiple independent arenas.

### Offset tests

Cover:

- allocation -> offset -> resolution round trip;
- immutable value read;
- exclusive mutation;
- invalid raw offset;
- end boundary;
- out of bounds;
- misalignment;
- cross-arena misuse remains safe.

### Packing tests

For `u8/u16/u32/u64`:

- first bit;
- last bit;
- middle range;
- full word;
- zero value;
- maximum fitting value;
- one-over rejected;
- neighboring bits remain unchanged.

Boolean tests must show multiple boolean fields sharing one word can be toggled independently without losing other bits.

### Width-selection tests

Test every boundary around 8/16/32/64-bit transitions.

### Native-view tests

Verify:

- same underlying content;
- zero-copy pointer identity where promised;
- mutation updates arena storage;
- slice length and initialization bounds;
- borrow/lifetime safety through compile-fail tests or Miri where appropriate.

### Miri

Run Miri against unsafe-heavy arena/offset/native tests when supported.

Prefer targeted tests rather than making unsupported targets block all development.

## Cargo/package requirements

`compact_core` must be a normal library package.

Its `Cargo.toml` should include appropriate metadata such as:

- package name;
- version;
- edition;
- license;
- repository;
- description/categories/keywords if useful.

Do not require a custom rustc wrapper or build script for normal use.

An external crate must be able to write:

```rust
use compact_core::{Arena, Offset32};
```

or equivalent public names.

Validate packaging with Cargo's package/dry-run tooling as appropriate.

## Validation commands

```bash
cargo fmt --all -- --check
cargo check -p compact_core --no-default-features
cargo test -p compact_core
cargo clippy -p compact_core --all-targets -- -D warnings
```

Add targeted Miri commands after concrete tests exist.

Also validate that an external consumer fixture can depend on the crate through a path dependency without workspace-private APIs.

## Non-goals

Do not implement:

- `CompactVec`;
- `CompactString`;
- `CompactBox`;
- maps/sets;
- procedural macros;
- global/TLS arena context;
- source rewriting;
- arbitrary code range inference;
- OS allocation;
- raw syscalls;
- std facade behavior;
- native-owned collection conversion.

## Expected handoff

Report:

- changed files;
- commit SHA;
- exact sentinel/null choice;
- exact backing contract;
- destructor policy;
- arena capacity semantics;
- unsafe blocks and invariants;
- package/import validation;
- tests/Miri run;
- deviations and unresolved assumptions.
