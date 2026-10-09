# PLAN_OWNERSHIP.md — Resolved Handles and Borrowing Contract

## Scope
Architectural design and contracts for common cage resolution and validation. Read `crates/compact_backend_std/src/cage.rs` and `crates/compact_collections/src/{vec.rs,deque.rs,hash_map.rs}`; inspect `CageAllocation<T>` accessors, `read_header`, `validate_typed_header`, `CompactVec::as_mut_slice`, `CompactVecDeque::with_view`. Allocator owns cage.rs implementation; collections owns consumers. This workstream defines shared interfaces and tests/proof obligations, not competing edits to them.

## Facts
Full per-operation header validation is inlined yet costly in A4; borrowed deque view yielded major speedup; B6 slice updates nearly match native indexed updates. Existing internal validation ensures typed extent and lifetime correctness and cannot be casually removed.

## Proposed experiments
Define safe immutable/exclusive borrow-scoped resolved allocation types; validate offset and typed header once when opening view, then reuse checked bounds for operations until view ends. Benchmark generic view versus collection-specialized views, repeated IndexMut, iterators, append, retain and dispatch. Inspect AArch64/x86-64 codegen, binary size, inlining and monomorphization. Prefer internal reusable contract only when it demonstrably helps multiple consumers; otherwise retain specialized code.

## Safety and transitions
Externally supplied/reconstructed offsets always validated. A valid mutable view proves exclusive access with Rust lifetime bounds, no reallocation/growth while live, no aliasing with other views, no stale pointers, no retaining pointer inside frozen owner, panic/unwind leaves initialized length and metadata valid. Explicitly describe behavior across cage relocation/lifetime (if supported), drop and concurrent ownership transfer. No public unchecked fast path; determine which checks may be amortized by proof, not assertion.

## Memory/compatibility
Prefer stack-scoped handles and unchanged 4-byte owners/12-byte deque. Any new retained native pointer/metadata requires quantified cross-framework memory comparison and approval. Preserve std-like ergonomics where practical; distinguish source, ABI and persistence migrations.

## Deliverable
Write contract decision note with exact proposed types/signatures, verified consumer call sites, safety argument, timing/codegen and memory comparisons, rejected variants and owner handoff instructions. Implementation gated on allocator interface sign-off.
