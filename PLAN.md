# PLAN.md — V2.4 Profile-Guided Optimization, Round 3

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified baseline: 12469610348009bc038215e6b97a5f7f9342c1e5

## Objective
Investigate and implement safe, measured V2.4 improvements for A4 deque, A5 hash, B6 order book, B8 cache, B10 concurrency and A2 allocation/header validation. Repair failing benchmark CI first. Only ship changes with repeatable gains and passing correctness tests.

## Verified facts
- Miri passes on pinned head.
- Benchmark CI run 37725406606 fails during `cargo run --locked ... --self-check`: Cargo.lock would need updating. No checksum failure was established.
- CompactHashMap::get uses insertion-oriented find_slot_in, which tracks tombstones despite lookup not needing an insertion target.
- Control bytes represent EMPTY, FULL, TOMBSTONE without short hash fingerprints.
- CompactVecDeque::{push_back,pop_front} independently resolve and validate backing cage storage.
- read_header is inlined but still performs bounds and metadata validation.
- Shared allocator mutex and B10 sync/atomic/futex-heavy samples; B8 combines hash and allocator costs.
- Benchmark measure mode's counting allocator distorts native allocations; production timing uses accounting-free profile driver, not measured allocation-stat mode.

## Frozen invariants
Maintain 4-byte CageAllocation<T>, Option<CageAllocation<T>>, CompactBox<T>, CompactVec<T>; 12-byte CompactVecDeque<T>; 16-byte AllocationHeader; 8-byte frozen descriptors. One cage, u32 offset ownership, no retained native pointers, safe borrowing/aliasing/FFI, exact allocation accounting, panic/recovery/cross-thread correctness and randomized collision-resistant default hash. No software prefetch, inline asm, V3 target redesign or unchecked public interfaces.

## Workstreams and ownership
1. PLAN_BENCHMARKS.md: owns .github/workflows/harness.yml, Cargo.lock policy, benchmark_compare and benchmark_profile harness; establishes baseline and green CI.
2. PLAN_HASH.md: owns crates/compact_collections/src/{hash_map.rs,hash_control.rs}, dedicated hash tests; separate lookup, probe, fingerprint experiments.
3. PLAN_DEQUE.md: owns crates/compact_collections/src/deque.rs and deque tests; batched ring access.
4. PLAN_ALLOCATOR.md: owns crates/compact_backend_std/src/cage.rs and allocator tests; B10 sync, B8 release, header optimization proposals.
5. PLAN_VALIDATION.md: owns crates/compact_collections/src/vec.rs and vector tests, round-three profile evidence. Reads cage.rs; cage edits by allocator owner.

The orchestrator owns shared docs, integration benchmarks and shared test files. Executors must coordinate any overlap. Hash/deque/vector can independently experiment in isolated worktrees after CI/baseline; allocator changes require central approval of synchronization invariants. No simultaneous writes to shared harness or cage symbols.

## Integration order and gates
1. Verify latest main and reconcile architecture.
2. Repair lockfile/CI contract and establish clean release accounting-free baselines and checksum parity.
3. Reprofile A2/A4/A5/B6/B8/B10 and B3/B5 sentinel workloads with symbolized CPU samples; use hashed no-telemetry artifacts.
4. Run isolated hash lookup and deque/vector batching experiments; independently validate before merging.
5. Analyze B10 contention and create explicit safe accounting/reclamation design before implementing thread-local chunks.
6. Integrate hash, deque, vector, then allocator; profile shifted hotspots after each significant change.
7. Run full 16-scenario suites twice plus repeated noisy concurrency cases; retain only improvements with stable medians/p95, absolute time, invariant preservation and no significant regressions.
8. Run cargo fmt, all-feature workspace checks/tests/strict Clippy, configured Apple cross-target check and full .github/workflows/miri.yml tests; benchmark CI must pass.
9. Independently review final diff, remove all PLAN*.md files before final implementation commit.

## Validation commands
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Use benchmark_profile for accounting-free production timings; document baseline/candidate hashes, host/toolchain, medians/p95, native/compact checksums, retained live bytes, cage cursor and RSS distinctly. Instrumented allocation-stat mode is not a valid production timing comparator.

## Final review checklist
- [ ] Benchmark harness CI green and lockfile policy documented
- [ ] Frozen layouts, hash security, borrow/aliasing/invalidation invariants preserved
- [ ] Panic/drop/remote free/recovery/exhaustion/concurrency tested
- [ ] Memory comparisons and native B3/B5 improvements protected
- [ ] All accepted performance changes individually benchmarked and reproducible
- [ ] Rejected experiments documented, new profiles captured
- [ ] Full tests, Miri, Clippy, portability and benchmarks pass
- [ ] No PLAN*.md in final implementation commit

Executor handoff: Read PLAN.md first, verify latest main, run isolated bounded workstreams, report changed files, commit SHA, tests, measurements, deviations and assumptions. Resolve conflicts centrally; remove planning files before production commit.
