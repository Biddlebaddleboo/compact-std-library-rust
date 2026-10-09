# PLAN_VALIDATION.md — V2.5 Implementation Acceptance

## Scope / baseline
Read `scripts/profile_cpu.sh`, `scripts/sample_collections.sh`, `scripts/profiling/allocator_profile.sh`, `crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}`, `crates/compact_std/examples/benchmark_profile.rs`, `PROFILE_V2_5_FINAL.md`, `PROFILE_V2_5_MEMORY.md`, `BENCHMARKS.md`. Own profiling reports and standalone probes; orchestrator owns shared harness and final integration. Baseline pinned `8bfe6f7192c5cdd1258d1cffb7fc1b5dddfeab5e`, immutable toolchain/feature/CPU metadata and hashed telemetry-free artifacts.

## Correct comparisons
Use accounting-free timing, avoid counting allocator/lock snapshots in windows, alternate order and repeat noisy A2/A4/A5/B6/B8/B10/B3/B5. Same checksums. Equal workloads native/compact. Compare distinct borrow-scoped/batch API pathways separately. Record absolute latency, median/p95, CPU samples, instructions/branches/misses where supported, per-thread throughput, locks, retained/live bytes, cage high-water, fragmentation, side metadata, virtual/committed pages, peak RSS and current idle RSS after teardown. Don't mistake 4/8 oversubscribed workers on two vCPU for multicore scaling; 4+ physical cores are optional, not a gate. Compare same inputs and worker counts.

## Verification
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Run every step in `.github/workflows/miri.yml` and `harness.yml`; Apple cross-target check if configured/available, deterministic allocator remote-free/exit/reentrancy/panic/failure/recovery stress, randomized collection std differential tests, duplicate-allocation checks, collision-security tests, all sixteen scenarios and B3/B5 regressions.

## Acceptance and handoff
Default ≤+2% retained and ≤+5% peak RSS vs pinned baseline, bounded TLS caches, stable idle RSS, no data races, unsafe aliasing, double drops, lost release descriptors, unapproved ABI/layout changes or benchmark-only shortcuts. Classify each candidate Accepted/Rejected/Blocked (specific safety prerequisite). Produce `PROFILE_V2_5_IMPLEMENTATION.md` with commits, diff scope, accepted/rejected designs, timing+memory tables, tests and residual hotspots. Independently review final diff against PLAN.md and all workstreams, remove all temporary PLAN*.md files in the final **implementation** commit.
