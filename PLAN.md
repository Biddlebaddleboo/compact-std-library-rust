# PLAN.md — V2.4 Round 4 Profiling and Optimization

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 59540b84cd97c0eb5bfda3f4db3ce45c5a604d76
CI at baseline: Miri and benchmark harness passing.

## Objective
Profile and resolve remaining evidence-supported V2.4 bottlenecks without sacrificing memory footprint, safety, or compatibility. Priorities: B10 allocator concurrency; A5 hash probing; B8 combined map/allocator overhead; A4/B6 batch access; A2 overhead. Do not repeat rejected experiments without a new hypothesis.

## Verified facts
Round 3 introduced CompactHashMap::find_index_in for lookup, and CompactVecDeque::with_view for opt-in borrow-scoped no-growth batch operations; regular deque operations remain unchanged. Hash control metadata has no partial-hash fingerprints. B10 has substantial lock/futex/atomic self samples, with large variability on two-vCPU host. Thread-local allocation chunks have not been implemented. A2 is ~1.5x native in accounting-free capture. B3/B5 remain faster than native in measured cases.

## Frozen invariants
Four-byte cage-relative owners/vectors; 12-byte deque; 16-byte AllocationHeader; eight-byte frozen descriptors; one process-wide cage; no persistent native pointers; correct ownership/aliasing/initialization/drop/FFI; accurate live-byte accounting, concurrent/remote free semantics; randomized hash-flood-resistant default hashing. No software prefetch, new inline assembly, V3 compiler-target changes or unapproved lock-free reclamation.

## Workstreams and file ownership
- PLAN_ALLOCATOR_CONCURRENCY.md owns crates/compact_backend_std/src/cage.rs and dedicated allocator tests. Must design allocator state transitions, duplicate-allocation prevention, cross-thread frees and reclamation *before* implementing any thread-local chunk scheme.
- PLAN_HASH_PROBING.md owns crates/compact_collections/src/hash_map.rs, hash_control.rs, and dedicated hash tests.
- PLAN_COLLECTION_ACCESS.md owns crates/compact_collections/src/deque.rs, vec.rs and dedicated collection tests.
- PLAN_PERFORMANCE_VALIDATION.md owns standalone scripts (scripts/profile_cpu.sh, scripts/sample_collections.sh, scripts/profiling/allocator_profile.sh), PROFILE_V2_4_ROUND4.md and profiling evidence; common benchmark_compare/main.rs, benchmark_profile.rs, scenarios and BENCHMARKS.md are orchestrator-owned.

Hash and collection experiments are parallel-safe in isolated worktrees; allocator measurements may run concurrently but invasive allocator redesign requires central review of the complete state/accounting contract. Overlapping test/harness edits are integrated centrally; assign each shared symbol one owner.

## Integration sequence
1. Verify latest main and reconcile any material architecture changes.
2. Record clean accounting-free native/compact baseline on exact commit; run checksums and both CI workflows.
3. Profile A2/A4/A5/B6/B8/B10, retain B3/B5 sentinels. Include multiple worker counts/hardware where available.
4. Experiment independently with hash probing/fingerprints and collection batching. Report before/after micro and real scenario timings.
5. Prepare allocator state-transition model, tests and centrally reviewed design. Only then experiment with bounded thread-local reservation or caching, keeping simpler mutex changes as controls.
6. Integrate hash, collection and allocator work in that order; reprofile shifted costs and validate regressions.
7. Repeat full 16-scenario release suite twice plus noisy-case runs. Separate accounting-enabled measure mode from accounting-free timing, pin telemetry-free hashed artifact and record medians, p95, sample attribution, retained bytes, high-water and RSS.
8. Run cargo fmt --all -- --check; cargo check --workspace --all-features; cargo test --workspace --all-features; cargo clippy --workspace --all-targets --all-features -- -D warnings; available Apple cross-target check; full Miri workflow; harness CI parity.
9. Independently verify final diff and safety/performance evidence, resolve plan contradictions centrally; delete all PLAN*.md before final implementation commit.

## Acceptance and handoff
Only keep stable performance improvements without significant B3/B5 or other scenario regressions; maintain frozen layouts, matching checksums, collision security and complete allocator correctness. Document rejected hypotheses and profiler limitations. Every executor reports exact changed files/symbols, commit SHA, tests, measurements, deviations and unresolved assumptions. PLAN.md is a temporary handoff artifact, not a permanent source file.
