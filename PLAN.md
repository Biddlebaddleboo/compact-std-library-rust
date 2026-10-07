# V2 Plan: Regular Rust in a Compact Arena

## Objective

Evolve `compact-std-library-rust` from a low-level compact-memory runtime into an ergonomic compact Rust environment where application code remains as close as practical to ordinary Rust while owned data is physically stored inside the compact arena.

Primary design principle:

> Compact Rust changes storage semantics, not ordinary programming semantics.

Target experience:

```rust
use compact_std::prelude::*;

compact_std::arena!(arena, {
    let mut users = Vec::new_in(arena);

    users.push(User {
        name: String::from_in("Alice", arena),
        active: true,
        retries: 3,
    })?;

    users[0].active = false;
});
```

The long-term ergonomic target may permit even less explicit arena plumbing where proc-macro rewriting can do so safely:

```rust
compact_std::arena!(arena, {
    let mut users = Vec::new();

    users.push(User {
        name: String::from("Alice"),
        active: true,
        retries: 3,
    });
});
```

Do not require a rustc fork, MIR plugin, custom compiler, patched toolchain, or whole-program transformation.

Use:

- ordinary Rust libraries;
- normal Cargo dependencies;
- procedural macros where syntax/layout generation is needed;
- explicit native boundaries when standard-library ABI compatibility is required.

## Repository baseline

Repository:

`Biddlebaddleboo/compact-std-library-rust`

Target branch:

`main`

Verified current head when this plan was prepared:

`e452964cb9b11c1644ad4e6aab70944ffa045d0a`

Commit:

`feat: implement compact memory V1`

Current workspace:

```text
compact_core
compact_backend_std
```

Existing V1 planning files have been deleted as intended.

Before implementation, verify latest `main` and reconcile relevant changes.

## Authoritative V1 invariants

Do not redesign the following:

1. General arena-relative references remain 32-bit byte offsets.
2. `Offset32<'arena, T>` remains exactly four bytes.
3. One arena remains limited to the V1 <= 2^32-byte logical address space.
4. Offset zero remains reserved as the null sentinel.
5. `compact_core` remains backend-independent and `#![no_std]`.
6. Native addresses are computed from arena base + compact byte offset.
7. No global arena registry, hash lookup, lock, or dynamic dispatch is added to ordinary dereference.
8. Packed scalar storage uses `u8`, `u16`, `u32`, and `u64`.
9. Packed-field numbering remains LSB-first.
10. Native word byte order remains target-native unless a future explicit serialization layer says otherwise.
11. Unknown value ranges are never narrowed speculatively.
12. Compact data must not be passed to native libraries under false std/C ABI assumptions.

## User-facing design goal

Most code operating entirely inside an arena should look structurally like ordinary Rust.

Ordinary concepts should have compact equivalents:

```text
std::Box       -> compact Box
std::Vec       -> compact Vec
std::String    -> compact String
Option<Box<T>> -> compact nullable offset where applicable
references to arena-owned objects -> arena-relative compact references internally
boolean state -> packed bits in generated compact layouts
bounded integers -> minimum proven storage width
small strings/vectors -> inline storage where beneficial
```

Application code should continue to use familiar:

- methods;
- indexing;
- iteration;
- `for`;
- `match`;
- arithmetic;
- comparisons;
- borrowing;
- slices;
- strings;
- enums;
- structs;
- generics where practical.

Low-level `Offset32`, raw byte ranges, masks, and bit shifts should normally remain hidden behind compact types and generated accessors.

## Proposed workspace

Add:

```text
crates/
  compact_core/
  compact_backend_std/

  compact_collections/
    Cargo.toml
    src/
      lib.rs
      boxed.rs
      vec.rs
      string.rs
      small.rs
      slab.rs
      intern.rs

  compact_macros/
    Cargo.toml
    src/
      lib.rs
      compact.rs
      arena.rs
      layout.rs

  compact_std/
    Cargo.toml
    src/
      lib.rs
      prelude.rs
```

Possible test fixtures:

```text
fixtures/
  consumer/
  regular_rust_style/
  macro_layouts/
```

Dependency direction:

```text
compact_core
   ↑
compact_backend_std

compact_core
   ↑
compact_collections

compact_core + compact_collections
   ↑
compact_std

compact_macros
   ↓ generates code using public compact_core /
      compact_collections / compact_std contracts

compact_std
   re-exports compact_macros where appropriate
```

Do not introduce a dependency from `compact_core` to collections, macros, std backend, or facade.

## Workstreams

### Workstream A — Runtime and Collections

See:

`PLAN_RUNTIME_CONTAINERS.md`

Owns:

- byte-level arena access;
- allocation descriptors needed by containers;
- compact ownership primitives;
- `CompactBox`;
- `CompactVec`;
- `CompactString`;
- inline/small storage;
- slabs;
- interning;
- compact optional/reference representation;
- native zero-copy views.

### Workstream B — Layout Macros and Ergonomics

See:

`PLAN_MACROS_ERGONOMICS.md`

Owns:

- `#[compact]`;
- bounded-field metadata;
- constant-driven packing;
- boolean grouping;
- compact enum generation;
- arena-scoped syntax assistance;
- `compact_std`;
- prelude;
- std-like naming;
- generated native boundary adapters;
- optional explicit hot/cold and SoA transformations.

Workstream B may prototype against agreed interfaces, but final integration depends on the public runtime/container APIs from Workstream A.

## Shared interfaces and ownership

### CORE owns

- `Arena`;
- `Offset32`;
- byte/range validation;
- physical arena storage access;
- bitfield primitives;
- raw/typed native views;
- compact ABI constants.

### COLLECTIONS owns

- compact ownership semantics;
- dynamic-capacity metadata;
- element movement within arena storage;
- compact string/vector representations;
- slab/free-slot reuse;
- intern tables.

### MACROS owns

- source annotation parsing;
- generated compact layouts;
- generated field access;
- compile-time range handling;
- generated conversions.

### `compact_std` owns

- ergonomic names;
- prelude;
- facade exports;
- arena-facing user experience.

Do not let macros duplicate allocation logic or bit-manipulation rules already owned by runtime crates.

## Required syntax philosophy

Prefer normal Rust APIs.

Example compact `Vec`:

```rust
let mut values = Vec::new_in(arena);
values.push(10)?;
values.push(20)?;

for value in values.iter(arena) {
    // ordinary Rust
}
```

Later macro-assisted syntax may shorten arena arguments where context makes them unambiguous.

Do not implement syntax tricks that:

- hide fallible allocation without a defined error strategy;
- create ambient global arena state;
- make lifetimes unsound;
- silently allocate outside the arena.

## Arena context model

Avoid TLS/global-current-arena state.

The `arena!` macro may:

- bind an explicit arena identifier;
- rewrite supported constructors;
- introduce a local context object;
- generate lexical helper imports.

It must not make compact allocation depend on invisible process-global state.

If fully transparent `Vec::new()` cannot be made sound and predictable, retain `Vec::new_in(arena)` as the core API and let macro sugar transform source locally.

## Memory/locality priorities

V2 should deliberately improve both total RAM and cache behavior.

Priority order:

1. 32-bit arena offsets;
2. packed fields;
3. compact container metadata;
4. contiguous dense allocation;
5. small-string/small-buffer optimization;
6. reusable slab slots;
7. interning/deduplication;
8. optional hot/cold splitting;
9. optional struct-of-arrays generation.

Avoid optimizations that materially complicate normal code unless they provide measurable benefit.

## Compact metadata targets

### Dynamic vector

Prefer:

```text
offset   u32
len      u32
capacity u32
```

for a 12-byte general representation where practical.

Permit narrower specialized metadata where bounds are statically proven, but do not make variable-width metadata part of the general V2 ABI.

### Immutable slice/string reference

Prefer:

```text
offset u32
len    u32
```

8 bytes.

### Optional compact reference

Use the V1 zero offset as `None` where representation permits.

Do not add a separate tag byte unless required.

### Small storage

`CompactString` / compact vector variants may store small payloads inline.

Choose the exact inline capacity from benchmarks and alignment/layout constraints rather than an arbitrary aesthetic target.

## Compile-time constants

Rust `const` values and const expressions must be first-class sources of proven bounds.

Example:

```rust
const MAX_RETRIES: u64 = 7;

#[compact(max = MAX_RETRIES)]
retries: u64
```

Generated layout:

```text
retries -> 3 physical bits
logical API -> u64
```

The original logical constant remains an ordinary Rust `const`, preserving rustc/LLVM constant folding and dead-code optimization.

Generated packing metadata should itself use const-evaluable expressions where possible.

## Byte-level access

V2 must incorporate the previously identified byte-access hardening.

Initialized compact allocations must support zero-copy read-only byte views.

Safe unrestricted mutable bytes must not be provided for arbitrary Rust types because arbitrary byte patterns may violate validity.

Support:

- safe initialized `&[u8]` views where valid;
- safe mutable bytes for byte-valid storage;
- raw `MaybeUninit<u8>` inspection;
- explicit unsafe mutation escape hatch for arbitrary typed storage;
- direct packed-field access against underlying byte storage.

Arena-wide initialized `&[u8]` must not be provided because alignment padding and uninitialized allocations may exist.

## Slabs and reusable storage

Add a compact slab abstraction for repeated same-layout objects.

Goals:

- dense placement;
- low metadata;
- O(1)-style slot reuse;
- compact free-list indices;
- stable handles where required;
- no general allocator metadata per object.

Free-list links should use the smallest representation consistent with the slab's declared maximum population.

Prevent stale-handle unsafety. If slot reuse can cause logical ABA/stale-handle bugs, use generation counters where required by safe APIs.

## Interning

Provide optional arena-local interning for immutable:

- strings;
- byte blobs;
- identifiers.

Requirements:

- deduplicate repeated payloads;
- store one canonical arena allocation;
- return compact IDs/handles;
- no requirement that every compact string be interned;
- deterministic equality semantics;
- table metadata cost must not outweigh likely savings for tiny sets.

Hashing/table implementation may use native temporary computation while canonical payload remains compact.

## Hot/cold layout support

Do not automatically infer hotness.

Provide future-compatible explicit annotations such as conceptually:

```rust
#[compact]
struct Node {
    #[hot]
    key: u32,

    #[hot]
    next: Option<...>,

    #[cold]
    debug_name: String,
}
```

Macro-generated hot/cold splitting is permitted when the developer explicitly requests it.

The public logical API should continue to appear as one logical object where practical.

Do not implement profile-guided automatic hotness inference in V2.

## Struct-of-arrays support

Permit explicit macro-generated SoA containers for scan-heavy records.

Example conceptual annotation:

```rust
#[compact(soa)]
struct Position {
    x: i32,
    y: i32,
    active: bool,
}
```

The generated collection may physically store:

```text
x[]
y[]
active bitset
```

while exposing logical record accessors/iterators.

Do not transparently convert arbitrary ordinary structs to SoA without explicit opt-in.

## Error handling

Allocation remains fallible.

Do not hide arena exhaustion.

Compact collection APIs should use a coherent error type or error-conversion path that includes:

- arena exhausted;
- capacity overflow;
- offset overflow;
- invalid range;
- invalid compact layout;
- value exceeds declared bound.

Macro-generated setters for bounded fields must reject out-of-range values rather than truncate.

## State transitions and failure safety

For mutations such as vector growth:

1. validate arithmetic;
2. reserve/allocate destination;
3. move/copy elements;
4. update metadata only after destination is valid;
5. never leave metadata pointing at partially initialized storage.

If allocation fails, existing collection state must remain valid.

For slab reuse:

- slot free/occupied state must transition consistently with the single-owner arena API;
- safe APIs must never expose uninitialized freed contents.

## Drop and ownership

The existing V1 `Copy` restriction cannot remain the only ownership model once `CompactString`, `CompactVec`, and `CompactBox` exist.

Do not solve this by storing arbitrary native-pointer-owning Rust types directly inside the compact arena.

Instead distinguish:

- trivially stored compact values;
- compact owning wrappers whose owned payload is arena-managed;
- native values requiring actual Rust `Drop`.

Compact containers should normally own arena storage logically without requiring native destructors for every payload allocation.

If actual destructors are required for contained logical values, define an explicit drop strategy before accepting those types.

No silent destructor omission.

## Cache-locality validation

Add benchmarks or deterministic layout checks for representative cases:

- linked graph using native pointers vs 32-bit compact offsets;
- many short strings;
- many small records with booleans;
- dense slab objects;
- interned repeated strings;
- AoS vs explicit SoA scan.

Benchmarking is informative, not an ABI requirement.

Record:

- payload bytes;
- metadata bytes;
- allocations;
- bytes touched per logical iteration where practical.

## Compatibility tests

Add examples demonstrating that normal Rust coding patterns remain recognizable:

- loops;
- iterator traversal;
- indexing;
- string manipulation;
- nested compact containers;
- optional compact references;
- enums;
- generated packed structs;
- passing compact strings as `&str` to ordinary Rust APIs;
- passing compact byte vectors as `&[u8]`.

## Non-goals

Do not implement:

- rustc fork;
- MIR plugin;
- LLVM pass;
- automatic whole-program range inference;
- automatic hot/cold profiling;
- transparent rewriting of arbitrary crates without annotations/import changes;
- transparent FFI ABI substitution;
- process-global current arena;
- >4 GiB arena pointers;
- variable-width general pointers.

## Integration order

1. Verify latest `main`.
2. Implement core byte/range hardening.
3. Freeze collection-facing allocation interfaces.
4. Implement `CompactBox`.
5. Implement `CompactVec`.
6. Implement `CompactString`.
7. Add small-buffer/inline representation.
8. Add slab/intern primitives.
9. Integrate macro-generated layouts.
10. Add `compact_std` facade/prelude.
11. Add `arena!` syntax assistance.
12. Add explicit SoA/hot-cold generation.
13. Run cross-workstream tests and memory/layout benchmarks.

## Parallel safety

After the collection-facing CORE interfaces are frozen:

- runtime/container implementation can proceed separately from macro parser/code-generation scaffolding;
- macro work must consume, not redesign, collection contracts;
- `compact_std` facade integration should happen after names and constructors stabilize.

Shared hotspots:

```text
crates/compact_core/src/lib.rs
Cargo.toml
README.md
```

Assign final ownership of these integration files to the central/orchestrating executor.

## Validation

At minimum:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features
cargo test -p compact_core
cargo test -p compact_collections
cargo test -p compact_macros
cargo test -p compact_std
cargo run --manifest-path fixtures/regular_rust_style/Cargo.toml
```

Use Miri for unsafe core/container primitives where practical.

No external services or real sleeps.

## Final-diff checklist

- [ ] V1 offset ABI unchanged.
- [ ] `Offset32<T>` still four bytes.
- [ ] Core remains no_std/backend-independent.
- [ ] Byte access respects initialization validity.
- [ ] General compact Vec metadata is no larger than intended without documented reason.
- [ ] Compact String supports zero-copy `&str`.
- [ ] Compact containers do not require native pointer-width metadata per element.
- [ ] Allocation failure leaves existing state valid.
- [ ] Small-object optimization has deterministic representation rules.
- [ ] Slab reuse cannot expose freed/uninitialized values safely.
- [ ] Interning canonicalizes payloads correctly.
- [ ] `#[compact]` never silently truncates.
- [ ] bool fields pack to bits where allowed.
- [ ] const bounds feed compile-time layout generation.
- [ ] application-facing examples look recognizably like ordinary Rust.
- [ ] native interoperability uses explicit borrowed views.
- [ ] no hidden global arena.
- [ ] no compiler fork or custom toolchain requirement.

## Execution handoff

Implement `PLAN.md` exactly.

Verify latest `main` first.

Read both workstream plans before implementation.

Use isolated worktrees/branches once shared runtime interfaces are frozen.

Each executor must report:

- changed files;
- commit SHA;
- tests;
- deviations;
- unresolved assumptions.

Integrate runtime/container changes before final macro/facade integration.

Resolve architectural contradictions centrally rather than letting a workstream silently redesign the ABI.

After final implementation review:

1. independently verify this plan against the final diff;
2. run cross-workstream tests;
3. review unsafe code;
4. delete all `PLAN*.md`;
5. commit production code without planning files remaining.
