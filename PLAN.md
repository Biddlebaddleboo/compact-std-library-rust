# V2.1 Plan: Reclaimable, Generic, Optimized Compact Rust

## Objective

Harden the V2 compact runtime introduced by commit:

`b79f9ceaee8443e44be7733080e70ab06c76cf69`

so that ordinary compact Rust code no longer pays the major remaining costs identified in review:

1. dynamic collections must not permanently leak every superseded growth allocation inside the arena;
2. generic compact containers must support appropriate non-`Copy` values without silently skipping destructors or violating ownership;
3. `arena!` must correctly follow ordinary Rust bindings, moves, shadowing, destructuring, and supported helper-returned compact values instead of tracking only simple constructor-bound local names;
4. packed-field access must use word-oriented fast paths rather than per-bit loops on common layouts.

Primary design principle remains:

> Compact Rust changes storage semantics, not ordinary programming semantics.

Do not redesign the compact ABI or introduce the proposed future 16-bit/multi-arena addressing model in this work.

---

## Repository baseline

Repository:

`Biddlebaddleboo/compact-std-library-rust`

Target branch:

`main`

Verified head when this plan was prepared:

`b79f9ceaee8443e44be7733080e70ab06c76cf69`

Commit:

`feat: add compact arena runtime and macros`

Before implementation, verify latest `main` and reconcile any relevant changes.

---

## Authoritative invariants

Preserve all current V1 addressing invariants:

- `Offset32<'arena, T>` remains exactly four bytes.
- Arena references remain 32-bit byte offsets.
- offset zero remains the null sentinel.
- one arena remains limited to the V1 <= 2^32-byte address space.
- `compact_core` remains `#![no_std]`.
- `Arena` remains single-owner and non-`Send`/non-`Sync`.
- normal reference resolution remains arena-base + offset, with no registry or hash lookup.
- packed bit numbering remains LSB-first.
- native word byte order remains target-native.
- compact objects must not be passed under false native/std/C ABI assumptions.
- no TLS/global “current arena” may be introduced.
- allocation failure remains explicit.

Do not alter `Offset32` representation to solve reclamation.

---

# Verified repository facts

## Arena allocation

`crates/compact_core/src/arena.rs`

`Arena::allocate` is currently a monotonic bump allocator using one `cursor`.

It has no:

- free operation;
- reusable free-range index;
- realloc-in-place operation;
- allocation ownership token;
- coalescing;
- rollback/reclamation of superseded dynamic buffers.

`Arena::used_bytes()` therefore measures the high-water bump cursor rather than live allocation bytes.

## CompactVec

`crates/compact_collections/src/vec.rs`

`CompactVec<'arena, T>` currently requires:

`T: Copy`

Its physical metadata target is already correct:

- storage offset/range;
- length;
- capacity;

for a general 12-byte representation.

`reserve_in()` geometrically grows by allocating a replacement and copying the initialized prefix.

The old allocation becomes permanently unreachable but remains charged against arena capacity.

## CompactString

`crates/compact_collections/src/string.rs`

`CompactString` has:

- 12-byte inline payload;
- 16-byte total handle;
- long-string `offset + len + capacity` stored in the payload.

Growth also allocates replacement byte storage without reclaiming the previous long-string allocation.

## Other generic containers

`CompactBox`, `CompactSmallVec`, and `CompactSlab` currently use `T: Copy`.

This avoids destructor loss but excludes otherwise arena-compatible move-only values.

`CompactSlab` already has reusable object slots and generation checking; preserve those safety properties.

## Macro tracking

`crates/compact_macros/src/arena.rs`

`ArenaRewrite` currently records:

`HashMap<String, LocalKind>`

and learns compact locals primarily from direct statements such as:

```rust
let values = Vec::new();
```

It is name-based rather than binding/scope aware.

It does not provide full handling for:

- moves;
- aliases;
- shadowing across lexical scopes;
- tuple/struct destructuring;
- branch joins;
- supported helper-returned compact values;
- reassignment changing the compact/noncompact status of a binding.

## Packed access

`crates/compact_core/src/packed.rs`

`read_bits()` and `write_bits()` currently loop one bit at a time.

They are correct and preserve neighboring bits, including cross-byte fields, but common generated compact-field access pays unnecessary loop/index/mask overhead.

## Existing user-facing baseline

`fixtures/regular_rust_style/src/main.rs`

already demonstrates the desired style:

```rust
arena!(arena, {
    let mut values = Vec::new();
    values.push(10_i32)?;
    values[0] = 11;

    let mut message = String::new();
    message.push_str("compact")?;
});
```

V2.1 must preserve or improve this style.

---

# Workstreams

## Workstream A — Runtime, Ownership, Reclamation, and Packed Performance

See:

`PLAN_RUNTIME_OWNERSHIP.md`

Owns:

- reusable arena allocation;
- allocation metadata and reclamation contracts;
- dynamic-buffer replacement/reallocation;
- generic arena-safe ownership;
- destruction semantics;
- `CompactBox`;
- `CompactVec`;
- `CompactSmallVec`;
- `CompactSlab`;
- `CompactString`;
- related generated SoA storage where affected;
- packed-bit fast paths;
- runtime benchmarks.

This workstream owns all changes to `compact_core` allocation/safety contracts.

## Workstream B — Arena Macro Binding Analysis

See:

`PLAN_MACRO_TRACKING.md`

Owns:

- lexical binding/scope analysis inside `arena!`;
- compact-kind propagation through moves and assignments;
- destructuring;
- shadowing;
- supported helper-return inference/annotation;
- diagnostics for ambiguous or unsupported cases;
- macro fixtures and compile-fail coverage.

Workstream B may prototype independently, but final integration must target the finalized runtime APIs from Workstream A.

---

# Shared interfaces and ownership

## `compact_core` owns

- arena storage;
- allocation descriptors;
- reusable free ranges;
- allocation/reallocation/release primitives;
- byte-range safety;
- offset resolution;
- packed-bit primitives.

## `compact_collections` owns

- element initialization state;
- element destruction;
- container growth policy;
- capacity semantics;
- dynamic ownership;
- when a released range is safe to return to the arena;
- compact collection APIs.

## `compact_macros` owns

- syntax analysis;
- local binding identity;
- scope tracking;
- rewriting;
- diagnostics.

Do not make the macro responsible for memory reclamation or destructor logic.

---

# Required allocator model

The arena must gain bounded reusable storage without becoming a general-purpose heavyweight allocator.

The implementation should prefer a compact free-range allocator suitable for the arena's expected allocation patterns:

- bump allocation remains the fast path when tail space is available;
- released ranges become reusable;
- adjacent free ranges should coalesce where practical;
- resizing should support in-place extension when the allocation is the current tail or directly followed by reusable free space;
- otherwise allocate replacement, complete move/copy, commit metadata, then release the old range;
- allocator bookkeeping must not consume normal native heap allocations per compact allocation.

The executor must measure at least:

- metadata overhead;
- fragmentation;
- allocation/release latency;
- repeated vector-growth arena consumption.

Do not simply attach a native `HashMap` of every allocation to `Arena`.

---

# Reallocation invariant

For every dynamic container:

1. validate target capacity and byte size;
2. attempt safe in-place growth when possible;
3. otherwise reserve destination storage;
4. move/copy initialized elements according to the element ownership contract;
5. establish the replacement's complete initialization state;
6. commit container metadata;
7. release the old allocation only after successful commit.

On failure before step 6:

- old metadata remains authoritative;
- old elements remain valid;
- no element may be double-dropped;
- partially initialized destination elements must be cleaned up when required.

---

# Generic value ownership model

Do not solve non-`Copy` support by merely deleting `T: Copy`.

Define an explicit trait/internal contract for values that may safely reside in compact-owned arena storage.

The design must distinguish at minimum:

### Trivially relocatable values

Examples:

- integers;
- offsets;
- generated compact handles;
- other values with no destructor and no address-sensitive self-reference.

These may use bytewise relocation/copy where valid.

### Move/drop arena values

Values that have meaningful Rust `Drop` semantics but can safely be moved between arena slots.

Containers must:

- move, not duplicate, values;
- drop initialized live elements exactly once;
- drop removed/truncated/cleared values;
- clean up partially moved destinations after failure.

### Rejected values

Values whose correctness depends on:

- stable native address;
- self-reference;
- pinning;
- unsupported borrowed native references;
- other invariants the compact runtime cannot preserve.

These must fail at compile time through trait bounds or explicit APIs.

Do not promise transparent support for arbitrary `T`.

---

# Scope-end destruction

Current arena teardown simply discards backing memory.

Once non-`Copy` values are supported, destructor obligations must be explicit.

Choose and document one authoritative model.

Preferred model:

- compact owning wrappers retain Rust object-level ownership;
- their `Drop` implementations resolve live arena allocations and drop live elements when the wrapper itself is dropped;
- arena-owned value types that can outlive their wrapper only through compact handles must not create hidden destructor obligations;
- no native destructor may depend on the arena after the backing is gone.

If Rust lifetime/drop-order restrictions make this representation impossible for a particular wrapper, redesign that wrapper rather than silently omitting `Drop`.

Add compile-fail tests for unsupported destructor/lifetime combinations.

---

# Dynamic container goals

## CompactVec

Preserve the 12-byte general handle if possible.

Required behavior:

- `new`;
- `with_capacity`;
- `reserve`;
- `push`;
- `pop`;
- `truncate`;
- `clear`;
- indexed access;
- mutable indexed access;
- slices;
- iteration.

After V2.1:

- repeated growth must reuse/release old storage;
- dropping the vector must release its backing allocation;
- `truncate`/`clear` must drop removed nontrivial values;
- growth must move non-`Copy` elements safely;
- failed reserve/growth leaves the old vector unchanged.

## CompactString

Preserve current inline representation unless benchmarks prove a better representation.

Required:

- inline strings still allocate nothing;
- inline -> arena transition is failure-safe;
- arena -> larger arena transition releases old storage;
- clearing a heap-backed string may release storage rather than permanently abandon it;
- UTF-8 validity remains enforced.

## CompactSmallVec

Promotion must:

- move initialized values correctly;
- not duplicate/drop values twice;
- release heap backing when appropriate;
- preserve inline fast path.

## CompactBox

Support suitable non-`Copy` values.

Dropping a box must:

- run `T`'s destructor exactly once when required;
- return its allocation to reusable arena storage.

## CompactSlab

Preserve:

- generation checks;
- stale-handle rejection;
- slot retirement before generation wrap.

Allow suitable non-`Copy` values.

Removing occupied values must move/drop correctly.

Dropping the slab must drop all remaining occupied values.

---

# Packed-field performance

Optimize:

`crates/compact_core/src/packed.rs`

without changing observable bit layout.

Keep the existing generic correctness path if useful, but add fast paths for fields that can be covered by one native integer load/store.

Required cases:

- 1 byte;
- 2 bytes;
- 4 bytes;
- 8 bytes.

For an arbitrary `(bit_offset, width)`:

- determine the minimal containing byte span;
- when <= 8 bytes, load into a `u64` or narrower scalar using explicit native-byte-order handling;
- shift/mask once;
- on write, preserve all neighboring bits;
- use unaligned-safe loads/stores or byte assembly where alignment cannot be guaranteed;
- retain a safe fallback for edge cases if necessary.

Generated macro code should use these optimized primitives rather than duplicating bit logic.

Add benchmark cases for:

- 1-bit boolean;
- 3-bit bounded integer;
- cross-byte field;
- 16-bit aligned field;
- 32-bit aligned field;
- repeated generated getter/setter loops.

Do not use `#[inline(always)]` broadly.

Use ordinary `#[inline]` only on tiny hot primitives where benchmarks justify it.

---

# Performance acceptance criteria

Do not require a specific CPU-dependent percentage as a correctness gate.

Require instead:

- no regression in representation size;
- no per-bit loop for common <=64-bit field access;
- geometric vector growth no longer causes cumulative dead allocations proportional to every historical capacity;
- a repeated grow/shrink/grow test demonstrates reuse;
- benchmark output is recorded in implementation notes;
- allocator metadata remains materially smaller than using a native allocation record per compact object.

---

# Macro integration requirements

After Workstream A APIs stabilize, Workstream B must ensure syntactic sugar rewrites to the new generic APIs without weakening ownership semantics.

`arena!` must not:

- clone move-only values to make rewriting easier;
- introduce hidden `unsafe`;
- suppress `Drop`;
- convert allocation failure into panic;
- introduce global state.

---

# Compatibility

Existing V2 code should continue to compile where semantics remain valid.

Maintain:

- `compact_std::prelude::*`;
- `Vec`, `String`, `Box` facade aliases;
- `new_in` baseline APIs;
- lexical `arena!`;
- `#[compact]`;
- `#[compact(soa)]`;
- hot/cold annotations;
- existing packed layout constants.

Breaking internal APIs are permitted where necessary to establish correct allocator/ownership contracts.

Avoid unnecessary public API breakage.

---

# Deterministic tests

Use deterministic local tests only.

No sleeps or external services.

At minimum add regression coverage for:

- vector grows repeatedly without cumulative unreclaimed old buffers;
- vector frees backing and another allocation reuses it;
- grow failure preserves old vector;
- non-`Copy` element drop count is exactly correct;
- vector truncate drops exactly removed elements;
- clear drops exactly live elements;
- vector relocation neither leaks nor double-drops;
- box destructor executes once;
- small-vector promotion moves rather than duplicates;
- slab drops occupied non-`Copy` values;
- string growth reclaims old arena storage;
- allocator free-range coalescing;
- in-place tail extension;
- fragmentation fallback;
- packed fields at every relevant byte boundary;
- neighboring packed bits preserved;
- old generated layout fixtures produce identical encoded values.

Use test drop counters and locally injected allocation-failure limits rather than process-global heuristics.

Run Miri on focused unsafe ownership/relocation tests when available.

---

# Validation commands

Run from repository root:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p compact_core --no-default-features
cargo run --manifest-path fixtures/consumer/Cargo.toml
cargo run --manifest-path fixtures/regular_rust_style/Cargo.toml
cargo run --manifest-path fixtures/macro_layouts/Cargo.toml
```

Add and run the new macro-tracking fixture defined in `PLAN_MACRO_TRACKING.md`.

Run focused Miri tests if the toolchain provides Miri.

Run runtime microbenchmarks in release mode and record comparative results in the implementation report.

---

# Integration order

1. verify latest `main`;
2. implement reusable arena allocation contracts;
3. validate allocator/reallocation invariants;
4. introduce arena-safe generic ownership contract;
5. migrate `CompactBox`;
6. migrate `CompactVec`;
7. migrate `CompactSmallVec`;
8. migrate `CompactSlab`;
9. migrate `CompactString` to reclaimable dynamic storage;
10. optimize packed primitives;
11. freeze new public runtime/container APIs;
12. implement improved `arena!` binding analysis;
13. update fixtures/docs;
14. run full cross-workstream validation;
15. review unsafe code specifically for move/drop/reallocation failure paths.

Workstream B can proceed in an isolated branch after the runtime call shapes are frozen.

---

# Parallel-safety and shared-file ownership

## Runtime workstream exclusive write ownership

- `crates/compact_core/src/arena.rs`
- new allocator modules under `compact_core`
- `crates/compact_core/src/packed.rs`
- `crates/compact_collections/src/boxed.rs`
- `crates/compact_collections/src/vec.rs`
- `crates/compact_collections/src/small.rs`
- `crates/compact_collections/src/slab.rs`
- `crates/compact_collections/src/string.rs`
- collection/runtime tests

## Macro workstream exclusive write ownership

- `crates/compact_macros/src/arena.rs`
- macro tracking helpers/modules
- macro compile-fail/pass tests
- macro tracking fixture

## Central integration ownership

- `crates/compact_std/src/lib.rs`
- `crates/compact_std/src/prelude.rs`
- root `README.md`
- root `Cargo.toml` if fixture/workspace changes are required

Do not allow both workstreams to independently redesign shared facade signatures.

---

# Explicit non-goals

Do not implement:

- 16-bit local references;
- 4-byte slot addressing;
- multi-arena global references;
- a custom compiler;
- MIR/rustc plugins;
- automatic hotness inference;
- a native heap replacement for the whole process;
- arbitrary `Pin`/self-referential value support;
- concurrent/atomic arenas;
- serialization ABI guarantees.

---

# Final-diff checklist

Before considering implementation complete:

- V1 `Offset32` remains four bytes.
- no global arena registry or TLS exists.
- old vector/string growth buffers are actually releasable/reused.
- reusable allocator metadata is bounded and compact.
- generic container support does not merely remove `Copy`.
- all live nontrivial elements are dropped exactly once.
- failed growth cannot leak ownership or corrupt old metadata.
- `arena!` handles moves/shadowing/destructuring cases in its declared support set.
- unsupported macro cases produce clear compile-time diagnostics.
- packed common paths no longer iterate bit-by-bit.
- generated layout bit representation is unchanged.
- all existing fixtures still pass.
- new regression fixtures pass.
- unsafe blocks have local invariants documented.
- README describes the actual supported ownership and reclamation model.

---

# Execution handoff

Implement `PLAN.md` exactly.

1. Verify latest `main` first and reconcile relevant changes.
2. Read `PLAN_RUNTIME_OWNERSHIP.md` before changing core/container code.
3. Read `PLAN_MACRO_TRACKING.md` before changing `arena!`.
4. Keep each workstream in an isolated branch/worktree once shared interfaces are frozen.
5. Each executor must report:
   - changed files;
   - commit SHA;
   - tests run;
   - benchmark results where relevant;
   - deviations;
   - unresolved assumptions.
6. Integrate runtime/ownership first.
7. Integrate macro tracking second.
8. Run complete workspace and fixture validation.
9. Independently compare the final diff against all three planning files.
10. Resolve plan contradictions centrally rather than allowing one workstream to silently redesign architecture.
11. Review all unsafe ownership/reallocation code.
12. Delete all `PLAN*.md` files before the final implementation commit.
13. Commit the implementation without planning files remaining.
