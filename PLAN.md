# PLAN.md — V2.4 performance completion

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 4a57dc713f1158347b2c912b6d374ea4dd7213f7

## Objective and baseline
Close the remaining measured A2/A4/A5/B6/B8 CPU gaps against native Rust without reducing retained-memory density. Baseline compact/native ratios: A2 1.47–1.50x, A4 5.65–6.16x, A5 4.25–4.32x, B6 3.68–3.70x, B8 2.73–2.76x. These are investigation targets, not guaranteed improvements.

Current main already implements pending exact-release recycling before locking, a default-disabled global size-class cache, no-growth deque pushes, direct contiguous SIMD hash control classification, CompactVec::retain, and try_clone_copy. Preserve these wins.

## Frozen invariants
- CageAllocation<T>, Option<CageAllocation<T>>, CompactBox<T>, CompactVec<T>: exactly 4 bytes.
- CompactVecDeque<T>: 12 bytes; AllocationHeader: 16 bytes; frozen descriptors: 8 bytes.
- One process-wide cage, u32 retained offsets, no retained native pointers, safe Rust borrow API and native ABI/FFI.
- Every extent is uniquely live, pending, or globally free; exact live-byte accounting, alignment, no double release; no user destructor under allocator mutex.
- Panic safety, failure recovery, exhaustions and concurrency correctness remain mandatory.
- Do not add software prefetch/PRFM/PREFETCH*, new inline asm, speculative hints, lock-free reclamation, new pointer model or per-object metadata.

## Verified implementation surfaces
- crates/compact_backend_std/src/cage.rs: CageAllocation<T>::allocate, take_pending_reuse, ReleaseCollector::take_compatible/push/flush, ACTIVE_RELEASE_COLLECTOR_COUNT, allocate_block, release_many_locked, AllocatorTransaction, AllocatorStats.
- crates/compact_collections/src/deque.rs: CompactVecDeque::{push_back,push_front,pop_front,pop_back,reserve,physical_index}; existing no-growth path resolves each operation.
- crates/compact_collections/src/vec.rs: CompactVec::{retain,try_clone,try_clone_copy}, RetainNoDropGuard, RetainDropGuard. No-drop retain compacts in-place; drop-bearing retain uses native staging.
- crates/compact_collections/src/hash_map.rs: CompactHashMap::{hash,find_slot,find_slot_in,ensure_insert_capacity,rehash}, classify_control_group, first_empty_slot; randomized SipHash-2-4; rehash destination Vec<usize>.
- crates/compact_collections/src/hash_control.rs: scalar oracle, NEON, SSE2 control classification.

## Workstreams and ownership
1. PLAN_ALLOCATOR.md: A2/B8, owns cage.rs and allocator-specific tests. Do not modify collection production source.
2. PLAN_DEQUE_VECTOR.md: A4/B6, owns deque.rs, vec.rs and related tests. Allocator internals read-only.
3. PLAN_HASH.md: A5, owns hash_map.rs, hash_control.rs and related tests. Allocator internals read-only.

Parallel-safe in isolated worktrees. Central orchestrator owns shared benchmark files (crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}, as needed), BENCHMARKS.md, ARCHITECTURE.md and SAFETY.md. Executors should provide optional separate local harnesses and avoid conflicting changes. Interface changes must be approved centrally before integration.

## Execution/integration order
1. Verify latest main and reconcile code changes before starting.
2. Capture pinned baseline and native per-phase measurements.
3. Run three bounded workstreams in isolated worktrees.
4. Each executor reports changed files, commit SHA, tests, measurements, deviations, assumptions and risks.
5. Integrate allocator, then deque/vector, then hash; run targeted tests after each integration.
6. Run complete 16-scenario release benchmark suite twice; compare to native and pinned 4a57dc7 baseline.
7. Check medians, p95, individual phases, retained bytes and logical checksums; reject unmeasured complexity or harmful regressions.
8. Independently verify final diff, exact size assertions and Miri correctness. Resolve plan contradictions centrally.
9. Update shared docs; delete ALL PLAN*.md files from implementation tree and commit implementation without plans.

## Validation
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-performance-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-performance-run2.tsv
```
Run repository's complete Miri workflow including allocator and collection property tests, in an appropriate supported environment. Verify release assembly on AArch64 and x86-64 without inserting prefetch or hand-written assembly.

## Final-diff checklist
- [ ] Frozen sizes and offset-only retained ownership unchanged.
- [ ] No prefetch, new asm, lock-free scheme or weaker randomized hash default.
- [ ] Exact ownership, accounting, panic and cross-thread safety verified.
- [ ] A2/A4/A5/B6/B8 targeted results and B10 concurrency regression recorded.
- [ ] Full checksums and retained-memory measurements match expected logic.
- [ ] Workspace checks, test, Clippy, Miri, portability and two full suites pass.
- [ ] Only evidence-backed implementations kept.
- [ ] PLAN*.md deleted prior to implementation commit.

Execution handoff: "Implement PLAN.md exactly. Verify latest main first. Stay within scope unless code or tests require expansion. Assign each named plan to a bounded executor in separate worktrees; integrate in documented order. Run tests and twice-run release suite, review diff, delete PLAN*.md, and commit."
