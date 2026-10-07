# Workstream Plan: compact_backend_std

## Objective

Implement the first hosted execution backend for `compact_core` using ordinary Rust `std`.

This crate supplies stable backing memory and ergonomic hosted construction while keeping all compact representation rules owned by `compact_core`.

It must be a normal publishable Cargo library consumable through path, Git, or registry dependencies.

## Starting files and symbols

Proposed files:

```text
crates/compact_backend_std/Cargo.toml
crates/compact_backend_std/src/lib.rs
crates/compact_backend_std/src/memory.rs
crates/compact_backend_std/tests/integration.rs
examples/basic_std_arena.rs
```

Proposed primary symbols:

- a stable-memory owner such as `StdBacking`;
- an ergonomic hosted arena constructor or owner such as `StdArena`;
- re-exports of common `compact_core` types where that improves ordinary hosted use without obscuring ownership.

Names may change for idiomatic Rust, but the responsibilities must remain.

## Verified repository facts

- The repository had no implementation before the V1 planning set.
- `compact_backend_std` is the first consumer of the CORE backing contract.
- Backend independence is authoritative: this crate may depend on CORE, but CORE must never depend on this crate.
- V1 general compact references remain 32-bit byte offsets inside a <=4 GiB arena.
- No custom compiler or global allocator takeover is required.

## Dependency

Read and obey:

- `PLAN.md`;
- `PLAN_CORE.md`;
- the finalized `compact_core::backing` contract;
- the finalized V1 ABI documentation.

If the backing contract cannot be implemented safely using portable `std`, stop changing the contract locally and report the mismatch to the integrator. CORE owns the interface.

## Write scope

Own:

- `crates/compact_backend_std/**`;
- std-backend integration tests/examples;
- minimal coordinated root workspace metadata required to register the crate.

Do not edit CORE implementation except through a centrally reconciled interface revision.

## Required behavior

### 1. Stable standard-library backing

Provide an owned backing allocation whose base address does not change for its lifetime.

Requirements:

- requested fixed capacity;
- correct alignment;
- stable base pointer;
- deterministic release on drop;
- no relocation after arena creation;
- clean allocation failure reporting;
- capacity validation against the V1 compact address domain.

The first version should prefer fixed capacity over dynamic growth.

Do not silently replace/reallocate the backing buffer when more space is needed.

### 2. No platform-specific dependency

Implement with portable stable Rust `std` facilities where possible.

The crate should compile on ordinary:

- Linux;
- Windows;
- macOS.

Do not introduce mmap, VirtualAlloc, Mach VM, or raw syscalls merely to support huge lazy reservations in V1.

Those belong to future specialized backends.

### 3. Hosted arena ownership

Expose an API that makes backing lifetime safe and ergonomic.

A typical user should be able to create an arena from this crate without manually wiring raw pointers.

Conceptually:

```rust
let mut arena = compact_backend_std::StdArena::with_capacity(...)?;
```

or an equivalent owner/context type.

Do not create an unsound self-referential type just to achieve this syntax.

If a backing owner and arena must remain distinct to preserve lifetimes, prefer a slightly more explicit safe API.

### 4. Backend cost stays out of hot dereference path

After construction, ordinary compact offset resolution must not call into a dynamic backend interface on every access if a base pointer/length can safely be held in the arena.

Do not add:

- global registries;
- locks;
- hash tables;
- per-dereference allocation;
- dynamic lookup by arena ID.

Backend work should primarily occur during construction and destruction.

### 5. Capacity semantics

Support practical capacities up to the V1 representable limit subject to:

- host address-space availability;
- allocator limits;
- platform behavior.

Do not test the maximum by physically exhausting RAM.

Document the distinction between:

- V1 logical maximum arena span;
- requested backing capacity;
- host allocation success;
- physical memory commitment behavior of the chosen std allocation strategy.

If portable `std` cannot efficiently reserve enormous sparse arenas, accept that limitation in this backend. Future VM-specific backends can optimize it without changing CORE.

### 6. Native interoperability demonstration

Integration tests/examples must prove that a normal hosted Rust program can:

1. create std-backed compact memory;
2. allocate/store through CORE;
3. hold a four-byte `Offset32<T>`;
4. resolve it into a normal borrowed native reference;
5. call ordinary Rust code with that borrow;
6. mutate safely through an exclusive borrow;
7. drop the arena/backing without leaks or use-after-free.

For contiguous initialized payloads, demonstrate zero-copy native slice borrowing if CORE supports it.

### 7. Cargo/import usability

This crate must be directly consumable as an ordinary Rust dependency.

A hosted consumer should be able to use a declaration such as:

```toml
[dependencies]
compact_backend_std = { path = "../compact-std-library-rust/crates/compact_backend_std" }
```

and then ordinary source imports such as:

```rust
use compact_backend_std::StdArena;
```

Use the actual finalized public name.

Where ergonomic and unambiguous, re-export common CORE public types so hosted users can begin from the backend crate without importing every type separately.

Do not hide CORE itself; advanced users must remain able to depend directly on `compact_core`.

### 8. Future facade compatibility

Do not make `compact_backend_std` the permanent top-level API namespace.

Keep it possible for a later `compact_std` facade crate to depend on/re-export:

- `compact_core`;
- `compact_backend_std`;
- future compact containers;
- future macros.

Avoid public names that would force breaking changes merely to introduce that facade.

## Allocation and cleanup safety

If using raw allocation APIs internally:

- allocate and deallocate with matching `Layout`;
- handle allocation failure correctly;
- preserve size/alignment metadata needed for deallocation;
- prevent double free;
- prevent use after drop;
- avoid integer overflow when constructing layout.

Every unsafe block must document its invariants.

If safe std-owned storage can meet the stable-address contract more simply, prefer it.

## Failure behavior

Test/document:

- zero capacity;
- smallest valid capacity;
- representative normal capacity;
- invalid/excessive capacity;
- arena exhaustion;
- allocation failure path without trying to exhaust system RAM;
- required alignment;
- backing cleanup after normal use;
- cleanup after early error paths.

## Platform compatibility

The implementation must avoid assumptions that only hold on one host OS.

CI should eventually cover:

- Linux stable Rust;
- Windows stable Rust;
- macOS stable Rust.

V1 does not need OS-specific performance tuning.

If a portability limitation is discovered, document it rather than silently adding target-specific behavior to CORE.

## Consumer fixture

Add a minimal external-consumer test or fixture outside the crate's own module tests that proves:

- Cargo can resolve the library normally;
- public imports work;
- no workspace-private feature is required;
- a std-backed arena can be constructed and used.

Prefer a workspace example or dedicated fixture that can be built in CI without publishing.

README usage should show:

1. path dependency;
2. Git dependency form;
3. ordinary `use` statements.

Do not claim crates.io availability until actually published.

## Tests

### Lifecycle

- construct backing;
- create/use arena;
- drop safely;
- repeat construction/destruction.

### Capacity

- exact small capacity;
- exhaustion;
- rejected over-limit requests;
- no relocation.

### Alignment

Allocate representative aligned values and verify resolved native addresses meet alignment.

### Integration with CORE

Assert:

- four-byte offset representation;
- allocate/resolve round trip;
- native immutable borrow;
- native exclusive mutable borrow;
- multiple independent std arenas;
- packed primitive use through a real arena where meaningful.

### Packaging

Validate package metadata and package assembly/dry run.

Build the external consumer fixture independently of internal module visibility.

## Validation commands

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p compact_backend_std
cargo check -p compact_core --no-default-features
```

Also run suitable package validation, for example:

```bash
cargo package -p compact_core --allow-dirty
cargo package -p compact_backend_std --allow-dirty
```

Use `--no-verify` only if absolutely necessary and document why; otherwise package verification should compile.

Build/run the external consumer/example as part of validation.

## Performance constraints

Do not benchmark before correctness is established.

Architecturally ensure:

- std backing does not add per-access locks;
- offset resolution remains base + offset in the common path;
- native compatible borrows remain zero-copy;
- no hidden allocation occurs on every compact read/write.

Later benchmarks may compare against ordinary `std` allocation, but performance tuning must not weaken the V1 ABI/safety invariants.

## Non-goals

Do not implement:

- raw Linux syscall allocation;
- mmap;
- VirtualAlloc;
- Mach VM;
- embedded memory;
- shared-memory backends;
- process-global allocator replacement;
- compact collections;
- procedural macros;
- whole-program packing inference;
- persistent arenas;
- >4 GiB compact references.

## Expected handoff

Report:

- changed files;
- commit SHA;
- exact std allocation strategy;
- how base-address stability is guaranteed;
- practical capacity limitations;
- package/import validation;
- platform checks performed;
- integration tests run;
- any requested CORE contract changes;
- unresolved portability or ownership concerns.
