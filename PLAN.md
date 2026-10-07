# V1 Implementation Plan: compact_core + compact_backend_std

## Objective

Build the first stable foundation of `compact-std-library-rust` as a backend-independent compact memory runtime.

V1 establishes:

- a `#![no_std]`-capable `compact_core`;
- a `compact_backend_std` adapter for ordinary Rust `std` hosts;
- one canonical compact arena model with a hard 4 GiB maximum address space;
- 32-bit byte offsets for arena-relative references;
- stable-address allocation;
- compact bit-packing primitives using `u8`, `u16`, `u32`, and `u64`;
- boolean packing at one logical bit per boolean where no stronger packing rule already applies;
- explicit native-boundary APIs;
- ABI/versioning rules so later compact collections, macros, compiler-assisted tooling, and alternate backends can build on V1 instead of replacing it.

This milestone intentionally does not implement a complete alternative Rust standard library. It establishes the compact memory model that later std-like types will use.

## Repository baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`

Target branch: `main`

Verified before planning commit:

- GitHub reports the repository is empty.
- There are no commits.
- There are no refs.
- There are no existing files or compatibility constraints.
- Every implementation path named below is therefore a proposed addition.

Implementers must verify latest `main` before starting and reconcile any new architecture with this plan.

## V1 architectural invariants

### 1. Compact storage is arena-local

Only data deliberately stored in a compact arena uses the compact ABI.

Outside the arena:

- normal Rust remains normal Rust;
- native pointers remain native-width;
- external crates keep their native layouts;
- OS and FFI APIs keep their documented ABI.

The compact model must not require takeover of Rust's process-global allocator.

### 2. Backend independence is mandatory

`compact_core` must not depend on:

- `std`;
- libc;
- Linux syscalls;
- Windows APIs;
- macOS APIs;
- any allocator implementation;
- any filesystem/network/thread runtime.

It receives stable backing memory through a small backend contract.

`compact_backend_std` implements that contract using ordinary Rust `std`.

Future backends may use raw syscalls, platform VM APIs, embedded static memory, shared memory, or custom allocators without changing the compact ABI.

### 3. One V1 arena = at most 4 GiB

V1 uses one simple addressing model:

- arena-relative byte offset;
- stored as `u32`;
- one arena addresses no more than 2^32 bytes under the selected sentinel convention;
- no region IDs;
- no variable-width arena pointers;
- no scaled offsets;
- no hierarchical addressing.

The process itself is not limited to 4 GiB and may contain multiple independent arenas.

### 4. Compact references are physically 32-bit

The canonical general-purpose compact reference type must occupy exactly four bytes.

It must not embed:

- an 8-byte native pointer;
- an arena pointer;
- a backend pointer;
- a hidden global registry key that adds lookup cost to every dereference.

The common resolution model should be conceptually:

`native_address = arena_base + u32_offset`

subject to bounds, alignment, lifetime, initialization, and aliasing invariants.

### 5. Logical type, stored type, and execution type are distinct

V1 should make later automatic packing possible by keeping these concepts separate:

- logical type: what application code conceptually sees;
- stored type: the compact arena representation;
- execution type: the temporary value used in CPU registers or native Rust expressions.

A value may occupy a small bitfield in arena storage while being widened to `u32` or `u64` during computation.

### 6. Narrowing is conservative

Never shrink a value because current observations happen to be small.

Narrowing is legal only when the complete valid domain is established by:

- an explicit invariant;
- a compact type;
- a container invariant;
- an enum/boolean domain;
- generated metadata;
- or later static analysis that can prove the bound.

Unknown means wide.

No silent truncation and no runtime "promotion" model in V1.

### 7. Packing uses practical scalar words

V1 packing primitives support:

- `u8`;
- `u16`;
- `u32`;
- `u64`.

Future generated layouts should choose the smallest practical backing word for a related group:

- 1..=8 bits -> `u8`;
- 9..=16 bits -> `u16`;
- 17..=32 bits -> `u32`;
- 33..=64 bits -> normally `u64` on 64-bit targets.

Do not use `#[repr(packed)]` as the main mechanism.

### 8. Booleans pack by default

For compact layouts, a boolean consumes one logical bit unless:

- it is already absorbed into another valid packed metadata word;
- ABI/layout requirements prohibit packing;
- concurrency requirements justify isolation.

Unrelated compact booleans should be grouped into the smallest suitable `u8/u16/u32/u64` backing word.

V1 must provide the bit primitives necessary for this policy even though automatic source rewriting comes later.

### 9. Concurrency must preserve neighboring packed fields

Exclusive/non-shared mutation may use ordinary read-modify-write bit operations.

Future shared compact objects may use atomic backing words and generated atomic bit operations.

V1 must not claim that arbitrary packed storage is concurrently mutable by default.

If atomics are introduced in V1, they must preserve unrelated neighboring bits and use target-supported widths. Otherwise, define concurrency as a later layer and keep V1 single-owner by default.

### 10. Native boundaries are explicit

Borrowing from compact storage into native Rust should be zero-copy where representation permits.

Owned conversion may allocate/copy.

A compact layout must never be passed to an external library as though it were ABI-compatible with `std::Vec`, `std::String`, `Box`, C structs, or foreign pointer-bearing layouts.

## Proposed workspace

```text
Cargo.toml
PLAN.md
PLAN_CORE.md
PLAN_BACKEND_STD.md

crates/
  compact_core/
    Cargo.toml
    src/
      lib.rs
      abi.rs
      backing.rs
      arena.rs
      offset.rs
      layout.rs
      packed.rs
      native.rs
      error.rs

  compact_backend_std/
    Cargo.toml
    src/
      lib.rs
      memory.rs
```

Implementation may simplify module boundaries if needed, but dependency direction must remain:

`compact_backend_std -> compact_core`

Never the reverse.

## Compact ABI V1

Create an explicit ABI/version concept in `compact_core`.

At minimum document:

- offset width;
- offset unit (bytes);
- maximum arena span;
- sentinel/null representation;
- alignment rules;
- endianness assumptions for packed words;
- bit numbering convention;
- representation guarantees that are intentionally stable;
- representation details that remain implementation-defined.

Expose a V1 version marker/constant/type so future incompatible compact encodings can coexist or be rejected cleanly.

Do not promise cross-process/on-disk persistence yet.

The goal is source and architectural forward compatibility, not frozen persistent serialization.

## Shared interfaces

### Backing memory contract

CORE owns the contract.

It must provide enough information to establish:

- stable native base address;
- usable byte length;
- lifetime/ownership relationship;
- required alignment guarantees.

It must not expose backend-specific concepts.

### Arena

The arena must:

- reject unsupported backing sizes;
- perform checked offset/alignment arithmetic;
- never relocate issued storage;
- distinguish initialized from uninitialized storage;
- prevent safe resolution outside bounds;
- tie native borrows to arena/backing lifetime;
- define zero-sized allocation behavior;
- define exhaustion behavior deterministically.

Initial policy may be monotonic/bump allocation.

### Offset32<T>

The canonical compact reference must:

- be exactly four bytes;
- carry type information without increasing stored size;
- have restricted checked/unsafe raw construction;
- resolve through an arena;
- not depend on ambient TLS/global arena state in core;
- not implement misleading pointer semantics that cannot preserve lifetimes safely.

### Packed primitives

Provide deterministic bitfield helpers for `u8/u16/u32/u64`.

Required operations:

- validate bit range;
- extract;
- checked insert;
- boolean read/write;
- preserve neighboring bits;
- support full-word fields safely;
- compute bits required for a proven unsigned maximum;
- select the smallest standard backing word.

### Native views

Provide low-level safe APIs for initialized memory to become:

- `&T`;
- `&mut T` only with exclusive access;
- `&[T]`;
- `&mut [T]` where valid.

Raw pointers belong behind clearly unsafe APIs.

## Workstreams

### A. compact_core

See `PLAN_CORE.md`.

Owns the compact ABI, arena, offset model, bit packing, native borrowed views, and no-std safety model.

### B. compact_backend_std

See `PLAN_BACKEND_STD.md`.

Consumes CORE's backing contract and supplies fixed stable memory using ordinary `std`.

## Dependency and integration order

1. Establish CORE ABI and backing contract.
2. Implement CORE arena and `Offset32<T>`.
3. Implement std backing against the frozen minimum contract.
4. Complete CORE packed/layout/native primitives.
5. Run cross-crate tests.
6. Review unsafe invariants and representation-size assertions.
7. Only then consider performance benchmarks.

BACKEND_STD may proceed independently only after the backing contract is stable.

## Future-version compatibility requirements

V1 must deliberately leave extension points for:

- `CompactBox`;
- `CompactVec`;
- `CompactString`;
- compact maps/sets/queues;
- compact Rc/Arc-like ownership;
- typed slab/index structures;
- `#[compact]` procedural macros;
- arena-scoped source rewriting;
- generated packed structs;
- conservative range analysis;
- automatic boolean grouping;
- atomic packed-field APIs;
- native ownership translation;
- Linux raw-syscall backend;
- Windows backend;
- macOS backend;
- embedded/static-memory backend;
- multiple arenas in one process.

Future versions should be able to add these without changing V1's core rule that general arena references are 32-bit byte offsets into a <=4 GiB arena.

Avoid sealing public traits/types unless sealing is necessary for soundness.

Prefer versioned extension traits or new modules over retroactively changing V1 representation semantics.

## Explicit V1 non-goals

Do not implement:

- a rustc fork;
- MIR integration;
- whole-program range inference;
- procedural macros;
- a complete std replacement;
- compact collections beyond private test helpers;
- global-current-arena state;
- automatic field syntax rewriting;
- raw syscall backends;
- platform-specific VM reservation;
- >4 GiB arena pointers;
- variable-width pointers;
- persistent file-format guarantees;
- cross-process arena sharing.

## Safety requirements

Unsafe code will be necessary.

Every unsafe block must document:

- bounds invariant;
- alignment invariant;
- initialization invariant;
- lifetime invariant;
- aliasing invariant;
- why backend memory cannot move.

Safe APIs must not permit:

- arbitrary forged valid offsets;
- cross-arena memory unsafety;
- overlapping mutable native references;
- reads of uninitialized bytes;
- offset wraparound;
- references escaping the backing lifetime.

Run Miri where practical.

## Tests

Required categories:

- `compact_core` no-std build;
- `size_of::<Offset32<T>>() == 4`;
- arena capacity/exhaustion;
- alignment boundaries;
- offset round trip;
- invalid offset rejection;
- multiple arenas;
- packed u8/u16/u32/u64 extraction/insertion;
- boolean bit packing;
- neighboring-bit preservation;
- bit-width boundary calculations;
- native borrow lifetime/content tests;
- std backend lifecycle/integration;
- Miri coverage for unsafe primitives where practical.

No real sleeps or external services.

## Validation commands

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features
cargo test -p compact_core
cargo test -p compact_backend_std
```

Add Miri commands once the implementation layout supports them.


## Cargo distribution and consumer import

V1 must be consumable as ordinary Rust library crates rather than requiring a custom compiler, patched toolchain, or repository-local source copy.

Requirements:

- `compact_core` and `compact_backend_std` are normal Cargo packages with complete package metadata.
- Package/crate names must permit normal imports such as `use compact_core::...` and `use compact_backend_std::...`.
- Keep dependencies publishable: no accidental absolute filesystem paths, generated local-only dependencies, or unpublished cyclic workspace assumptions.
- A normal external Cargo project must be able to depend on V1 through a path dependency immediately, a Git dependency from this repository, and eventually a registry dependency without source changes.
- `compact_backend_std` should re-export the common core types needed by hosted users when doing so does not create ambiguous APIs, so a typical std-hosted application does not need unnecessary duplicate imports.
- Do not rely on `cargo install` for library consumption; `cargo install` is principally for binaries. The supported library installation/import model is Cargo dependency resolution (`cargo add`, `Cargo.toml` path/git/registry dependency) followed by ordinary `use` imports.
- Keep the workspace structured so a future top-level `compact_std` facade package can be added as the ergonomic default dependency without changing `compact_core` ABI or backend contracts.
- Add a minimal external-consumer fixture/example that compiles as a separate crate and proves the public crates can be imported and used without workspace-private APIs.
- Run `cargo package` or `cargo package --allow-dirty`/equivalent dry validation for publishable packages as appropriate; do not publish to a registry in this milestone.
- Document path and Git dependency examples in the V1 README.

Expected consumer shape:

```toml
[dependencies]
compact_core = { path = "../compact-std-library-rust/crates/compact_core" }
compact_backend_std = { path = "../compact-std-library-rust/crates/compact_backend_std" }
```

or, when consumed from Git/registry, the equivalent dependency declarations.

Expected source-level usage must be ordinary Rust imports, for example:

```rust
use compact_backend_std::StdArena;
use compact_core::Offset32;
```

The precise hosted constructor/type name may differ after the backing ownership design is finalized, but no custom compiler invocation may be required.

## Performance constraints

Do not add expensive machinery to the hot dereference path.

Common compact reference resolution must not require:

- locking;
- allocation;
- hash lookup;
- registry lookup;
- dynamic dispatch.

Packed reads/writes should inline to ordinary load/mask/shift/store operations.

The std backend should matter primarily at arena construction/destruction, not every access.

## Final-diff checklist

- [ ] compact_core is no_std.
- [ ] compact_core has no std/backend/platform dependency.
- [ ] Offset32<T> is exactly four bytes.
- [ ] One arena is capped by the V1 32-bit byte-offset model.
- [ ] Stable-address invariant is enforced.
- [ ] Checked arithmetic prevents offset wraparound.
- [ ] Safe code cannot resolve uninitialized/out-of-range memory.
- [ ] u8/u16/u32/u64 packing primitives exist.
- [ ] Boolean packing primitives preserve neighboring bits.
- [ ] Unknown ranges are never silently narrowed.
- [ ] Native borrowed views do not copy compatible contiguous data.
- [ ] ABI V1 conventions are documented/versioned.
- [ ] Future compact containers/macros/backends can build on V1 without changing the canonical offset model.
- [ ] Workspace tests/clippy/no-std checks pass.

## Execution handoff

Implement `PLAN.md` exactly. Verify latest `main` first.

Read `PLAN_CORE.md` before implementing CORE and `PLAN_BACKEND_STD.md` before implementing the std backend.

Stay within scope unless compilation, tests, soundness, or changed repository architecture requires expansion.

Executors must report changed files, commit SHA, tests, deviations, and unresolved assumptions.

Integrate CORE before dependent BACKEND_STD changes.

Resolve architectural contradictions centrally.

After implementation and final verification, delete all `PLAN*.md` files before the production implementation commit.
