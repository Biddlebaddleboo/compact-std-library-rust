# PLAN_PERFORMANCE_VALIDATION.md — Round 4 profiling and regression validation

## Scope
Own standalone profiling scripts (scripts/profile_cpu.sh, scripts/sample_collections.sh, scripts/profiling/allocator_profile.sh) and PROFILE_V2_4_ROUND4.md. Inspect crates/compact_std/examples/benchmark_compare/main.rs, benchmark_profile.rs, BENCHMARKS.md and PROFILE_V2_4_ROUND3.md read-only; orchestrator owns shared benchmark scenarios, global docs and conflicting test changes.

## Baseline
Pin actual main SHA and exact compiler, LLVM, flags/features, host, kernel and CPU model. Run clean checksum parity and green Miri/harness CI. Capture two full 16-scenario accounting-free native/compact suites on explicit telemetry-off hashed binaries, repeated noisy cases; B3/B5 sentinels preserved. Counting allocator mode is for allocation stats, never production latency ratios.

## Expanded profiling
A2/A4/A5/B6/B8/B10: inclusive/exclusive samples, top symbols, instruction/cycle/cache/branch counters only if supported, allocation counts, lock wait/hold, worker throughput, CPU scaling, retained/live bytes, high-water and RSS. B10 must sample worker threads (all-thread/system-wide capture with workload attribution), controlling scheduler and other-system noise. Alternate variants, log contamination and lost samples. Separate native/compact comparable algorithms; label opt-in batch versus per-op measurements separately.

## A/B protocol
One isolated hypothesis per change; reproduce exact baseline and candidate with same harness; match checksum, sample enough trials for distribution, record medians/p95 and absolute costs, generated-code observations, memory and unrelated scenarios; reject insignificant or unstable wins and explain limitations. Reprofile after integration to detect shifted hotspots.

## Final validation
Run:
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Run entire .github/workflows/miri.yml, .github/workflows/harness.yml and configured cross-target checks, two full release suites and additional deterministic allocator stress tests. Record unsupported counters rather than guessing.

## Deliverable
PROFILE_V2_4_ROUND4.md must state verified baseline and integrated SHA, before/after hotspot tables, accepted/rejected optimizations, safety and CI outcomes, allocator accounting/fragmentation, hash collision behavior, API semantics, remaining bottlenecks and whether further V2.4 effort is justified. Independently review final diff and remove all PLAN*.md before production commit.
