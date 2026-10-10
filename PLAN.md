# PLAN.md — V2.5 Performance Gap Reduction

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 4f1c1aa222bf6a62ba6a72b4585ab24799135c42

## Objective
Reduce native-Rust performance gaps by changing justified architectural contracts, while optimizing already-fast scenarios if shared improvements help. Priorities: B10 allocator synchronization; A5/B8 hashing, probing, storage; A4 ordinary deque; A2/B6 shared access and overhead. Implement bounded experiments, not another documentation-only survey.

## Verified repository facts
- V2.5 accepted `read_owner_header<T>` for allocator-issued private owner access while typed raw-offset validation remains.
- Four-byte owners, twelve-byte deque, sixteen-byte allocation headers and public APIs were not enlarged.
- Global allocator mutex dominates portions of B10; `allocator_model.rs` is sequential, not a concurrent correctness proof. `ALLOCATOR_PHASE1_V2_5.md` lists open safety gates for owner-affine chunks.
- `CompactVecDeque::with_view` and `CompactVec::as_mut_slice` already support efficient batching, but ordinary A4 is slower.
- Previous seven-bit fingerprints, EMPTY-only SIMD classifier and branchless deque index experiments were rejected based on full-workload measurements.
- B3/B5 retain faster-than-native results. The harness separates accounting-free timing from accounting instrumentation.

## Architectural and memory policy
Existing contracts may change with measured benefits; prefer common mechanisms only when multiple consumers benefit. For every candidate document hotspot, mechanism, exact symbols, safety, performance/memory A/B, and Accepted/Rejected/Blocked result. "Blocked" needs a precise safety prerequisite. Default vs pinned baseline: retained bytes ≤+2%, peak RSS ≤+5%, idle memory near baseline, bounded TLS slack, no material fragmentation regression. Preserve compact owner layouts absent separately approved benefit; no weakened collision resistance or raw-offset validation.

## Ownership and dependencies
- `PLAN_ALLOCATOR.md`: exclusive write ownership of `crates/compact_backend_std/src/cage.rs`, `allocator_model.rs`, allocator tests.
- `PLAN_HASH.md`: `crates/compact_collections/src/hash_map.rs`, `hash_control.rs`, hash tests.
- `PLAN_COLLECTIONS.md`: `crates/compact_collections/src/deque.rs`, `vec.rs`, collection tests. Backend access API changes proposed here but implemented only by allocator owner.
- `PLAN_VALIDATION.md`: standalone profiler scripts and reports; orchestrator owns shared benchmark scenario and entrypoint edits, shared documentation and interface integration.
Hash and collections are parallel-safe in isolated worktrees. Allocator safety modeling can proceed concurrently, but its unsafe implementation awaits central gate approval. Keep file/symbol ownership non-overlapping.

## Integration order
1. Verify latest main and reconcile source/test changes; read PLAN.md and relevant workstream.
2. Capture pinned telemetry-free baseline and memory/checksum parity on current two-vCPU machine.
3. Prototype hash and collection changes independently, measure each, integrate only accepted improvements.
4. Close actual allocator provenance, concurrent registry/pin ordering, bounded remote release, TLS/reaper, stats and memory-cap gates; centrally review, then implement isolated owner-affine chunk prototype with global fallback.
5. Integrate accepted hash then collection changes, finally allocator after full safety/memory validation. Reprofile shifted hotspots.
6. Validate 1- and 2-worker B10 as primary evidence; 4/8 oversubscription optional stress, not multicore scaling claims. A 4+-physical-core host is **not** a requirement.
7. Run complete all-16 accounting-free suite, repeat noisy cases, memory and B3/B5 sentinels; fmt/check/tests/strict Clippy/Miri/CI/cross-target where available, independent final diff review.
8. Resolve contradictions centrally, record decisions, delete all PLAN*.md before final implementation commit.

## Non-goals and execution handoff
No wholesale profiling survey, repeat of failed designs without new evidence, unsafe release/reclaim shortcuts, unbounded TLS cache, V3 compiler changes or benchmark-only shortcuts. Executors report exact changed files/symbols, commit SHA, tests/commands, timing and memory changes, deviations and assumptions. Do not force a failing or unsafe change to satisfy a production-delivery goal.
