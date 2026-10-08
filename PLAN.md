# PLAN.md — V2.4 performance completion, round two

Repository: Biddlebaddleboo/compact-std-library-rust
Target branch: main
Verified baseline: 8d11b1c1f8a205168139e4221e91a549e2b8730f

## Objective
Run fresh post-optimization profiles and fix any evidence-supported performance and profiling-methodology defects. Do not force speculative performance changes. Baseline compact/native ratios: A4 4.40–4.42x, A5 2.77–2.78x, B6 2.24–2.25x, B8 2.01–2.02x, B10 2.12–2.18x, A2 1.04x. Preserve B3/B5 wins. Aspirational, non-binding targets: A4 <2.5x, A5 <2x, B6 <1.5x, B8 <1.5x, B10 <1.5x.

## Verified repository facts
- Recent production changes inline hot cage/header accessors, avoid redundant resolution in CompactVec::push and resolve hash table control/entry storage once in CompactHashMap::{insert,get_mut,remove_entry}.
- Earlier profiles predate these optimizations; percentages are no longer authoritative.
- Benchmark harness in crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs} supports separate child processes, phases, checksums and allocator telemetry.
- Current head 8d11b1c Miri workflow passed, including integration coverage for get_mut and remove_entry.
- Old profiles showed CountingAllocator atomic overhead in native samples, missing native B10 samples, broken lock/transaction pairing and severe uprobe perturbation.

## Frozen architecture
Retain 4-byte CageAllocation<T>, Option<CageAllocation<T>>, CompactBox<T>, CompactVec<T>; 12-byte CompactVecDeque<T>; 16-byte AllocationHeader; 8-byte frozen descriptors; one cage, u32 relative owners, no retained native pointers; unchanged borrow/aliasing/init/FFI guarantees, randomized collision-resistant default hash, exact allocator accounting and safe recovery. No software prefetch, new inline assembly or V3 build-target redesign.

## Workstreams / ownership / dependencies
1. PLAN_BENCHMARKS.md: owns shared benchmark harness (main.rs, measure.rs, scenarios.rs), and profiling mode/diagnostic harness consistency. Its correctness and comparability work is a prerequisite for interpreting results.
2. PLAN_PROFILING.md: owns scripts/profile_cpu.sh, scripts/sample_collections.sh, scripts/profiling/allocator_profile.sh and allocator_uprobes.sh (coordinate shared scripts with benchmark executor), new PROFILE_V2_4_ROUND2.md. Fresh sampling should follow benchmarking methodology fixes; it may begin baseline measurements before then but must recapture affected measurements.
3. PLAN_COLLECTIONS.md: owns crates/compact_collections/src/{deque.rs,vec.rs,hash_map.rs,hash_control.rs} and dedicated collection tests. May conduct independently isolated experiments, but retain only results verified against final profiling/bench methodology.
4. PLAN_ALLOCATOR.md: owns crates/compact_backend_std/src/cage.rs and allocator tests. Starts with measurement, especially B10; invasive allocator changes only after credible evidence. B8 residual should be reprofiled after collection integration.

Orchestrator owns BENCHMARKS.md, ARCHITECTURE.md, SAFETY.md, PROFILE_SUMMARY.md and cross-workstream contracts, and resolves any conflicts. No executor concurrently writes shared harness files. If collection and allocator need a new shared API, centrally assign ownership and agree signature/invariants before consumers change.

## Execution and integration order
1. Verify latest main; reconcile architecture and Miri status; read PLAN.md first.
2. Capture uninstrumented native/compact baseline and checksum equivalence, host/toolchain facts.
3. Integrate benchmark methodology fixes and establish comparable profile modes.
4. Reprofile A4/A5/B6/B8/B10 on production equivalent optimized builds, with A2 and B3/B5 regression sentinels.
5. Run collection and allocator experiments in isolated worktrees, with minimal diff and controlled A/B comparisons. No speculative fixes. Assign bounded executors; report changed files, commit SHA, benchmarks, tests, deviations, assumptions.
6. Integrate collections, rerun relevant tests and B8 residual profiling; integrate allocator only after confirmed attribution.
7. Rerun 16 native/compact scenarios twice with production release flags and no telemetry. Repeat noisy scenarios. Record absolute medians/p95, phase costs, RSS, retained bytes, cage high-water and matching checksums.
8. Run format, checks, all-feature tests, strict Clippy, available Apple cross-target checks, and complete Miri workflow. Inspect final diff independently; remove all PLAN*.md before final implementation commit.

## Acceptance and negative results
Keep only repeatably faster changes that preserve correctness, layout, memory, and realistic workload performance. Revert neutral/harmful experiments and record rejected hypotheses. Distinguish exclusive/self CPU share from overlapping inclusive samples; never equate instrumented timer aggregates with production CPU time; never compare incompatible allocation-accounting modes. Preserve concurrency, cancellation/unwind, destructor, allocation failure, fragmentation and recovery behavior.

## Commands
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-round2-1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-round2-2.tsv
```
Miri commands must match .github/workflows/miri.yml.

## Final-diff checklist
- [ ] New profiles use latest production code and correctly symbolize worker threads or state limitations
- [ ] Harness accounting and probe perturbation are controlled
- [ ] Frozen layouts and hash security unchanged
- [ ] No persistent pointer/unsafe aliasing/invalidated batch borrow
- [ ] Panic, drop, allocation/release, OOM and concurrency invariants tested
- [ ] All scenarios have matching checksums; B3/B5 wins retained
- [ ] Repeatable wins documented, rejected experiments described
- [ ] Miri, workspace tests, Clippy and cross-target checks pass
- [ ] No speculative optimization or PLAN*.md remains at final commit
