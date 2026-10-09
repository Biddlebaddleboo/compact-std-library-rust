# PLAN.md — V2.5 Targeted Architectural Implementation

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 8bfe6f7192c5cdd1258d1cffb7fc1b5dddfeab5e

## Objective and verified facts
Deliver *production* V2.5 performance improvements via bounded, measured architectural prototypes, not another documentation-only survey. Current production allocator and collections still use the V2.4 contracts. B10 has a global-mutex/atomic hotspot (~5x native on two vCPU); A4 ordinary deque ~3.3x while borrow-scoped `CompactVecDeque::with_view` is much faster; B6 repeated indexed vector updates benefit from `CompactVec::as_mut_slice`; hash insertion/probing is prominent in A5/B8/B5. Prior seven-bit fingerprint experiment was rejected. `allocator_model.rs` is sequential and does not establish concurrent safety. `ALLOCATOR_CONTRACT_V2_5.md` proposes owner-affine bounded chunks, local reuse and remote-release routing; no production chunk allocator exists. B3/B5 are already faster than native.

## Policy
Changes to contracts, metadata and layouts are permitted if repeatable performance gains justify complexity and memory. Every workstream must produce a concrete hypothesis, isolated prototype where safe, tests, comparative performance/memory measurements and explicit Accepted / Rejected / Blocked result. Blocked requires a specific demonstrable safety/technical prerequisite. Do not commit rejected unsafe prototypes. Preserve safe Rust borrow, initialization, destructor, concurrent ownership, collision-resistant randomized hashing and correct accounting.

## Memory and compatibility
Compared with pinned baseline: default retained bytes ≤+2%, peak RSS ≤+5%, no unbounded TLS footprint, no meaningful fragmentation regression, reclaim idle reservations, 4-byte compact owner and 12-byte deque preferred. Measure absolute and percentage deltas, virtual reservation separately from resident/committed memory, and RSS after thread exit. Exceptions require explicit approval; identify public API/ABI/persistence impacts.

## Ownership and parallelism
- PLAN_ALLOCATOR.md owns `crates/compact_backend_std/src/cage.rs`, `allocator_model.rs` and allocator-specific tests; owns backend resolution primitive implementation.
- PLAN_COLLECTION_ACCESS.md owns `crates/compact_collections/src/{deque.rs,vec.rs}` and dedicated tests; proposes backend APIs but never edits cage.rs.
- PLAN_HASH.md owns `crates/compact_collections/src/{hash_map.rs,hash_control.rs}` and hash tests.
- PLAN_VALIDATION.md owns independent profiling reports/scripts and memory/performance evidence; shared benchmark scenario/entrypoint edits and final docs are orchestrator-owned.
Run allocator, collection and hash experiments in isolated worktrees. Shared interface and symbol decisions must be approved centrally; no overlapping writers. Hash and collection prototypes can integrate separately; backend contract consumer changes wait for allocator-owner interface.

## Execution / integration
1. Verify latest intended main; reconcile relevant source/CI changes. Read this file then specific workstream.
2. Pin clean V2.5 baseline, hashed telemetry-free release timing artifacts, parity and memory before experiments.
3. Prototype collection ordinary operations and hash changes independently; test and measure, reject unsupported results.
4. Extend allocator model to proposed owner-affine semantics, resolve all concurrency safety gates, review architecture centrally, then implement isolated bounded chunk prototype with legacy fallback. Do not enable unsafe changes before gates pass.
5. Integrate accepted collection/hash improvements, then allocator; reprofile shifted hotspots and full 16 scenarios. Protect B3/B5 and already-fast cases.
6. Validate single-thread/two-thread B10 on present two-vCPU machine. 4/8-worker oversubscription is stress only; genuine 4+-physical-core host optional, **not** acceptance gate.
7. Run fmt/check/test/strict Clippy/full Miri and harness workflows, Apple cross-target if available, deterministic stress/differential tests, repeated accounting-free A/B captures, memory budgets and final diff review.
8. Central orchestrator resolves contradictions, documents rejected/blocked experiments, removes **all PLAN*.md** and commits implementation without them.

## Handoff
Each executor reports changed files/symbols, commit SHA, commands/results, baseline/candidate timings, memory, API changes, deviations and unresolved assumptions. Expand search only for changed symbols, compile/test failures, compatibility or safety necessities. An unsafe/regressive change is never forced through simply to produce a production commit.
