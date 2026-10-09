# PLAN_VALIDATION.md — V2.5 Integration and Acceptance

## Scope
Independent final review of PLAN.md, six workstream plans, profile/memory reports, implementation diffs and CI. Orchestrator owns shared benchmark changes; this workstream owns validation checklist and summary, not independent architecture redesign.

## Baseline and comparability
Verify exact V2.4 baseline 9ff38cdfc6daffc75eb39fcc8047d12cd9d180f8 and current remote main, reconcile relevant changes. Record toolchain/LLVM/target/features/harness and compare same algorithm, data and work to native for ordinary API. Separate new batch/caller-level comparison; keep allocation-counting mode out of production timing. Run all sixteen A/B workloads including already-fast B3/B5, matching checksums.

## Regression gates
Capture at least two full accounting-free release suites plus repeated interleaved runs for noisy cases, worker scaling B10, medians/p95 and absolute time, throughput, counters and CPU hotspots. Reprofile after integration. Validate retained memory ≤+2%, peak RSS ≤+5%, idle close to V2.4, bounded thread slack, no fragmentation surprises or unapproved footprint/compatibility changes. Protect strong baseline wins and reject severe hidden regressions.

## Verification commands
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Execute complete `.github/workflows/miri.yml` and `harness.yml`, configured cross-target checks (including Apple where available), native/compact differential and randomized tests, concurrency safety, reentrancy, panic/unwind, remote free, reclamation, allocation exhaustion, collision security, serialization ABI/migration cases. Do not claim tests actually run unless verified.

## Final handoff
Write `PROFILE_V2_5_FINAL.md` recording commit SHAs, accepted/rejected designs, phase and scenario results, memory deltas, safety evidence, outstanding risks and explicit approved budget exceptions. Independently review final diff against all plans and resolve contradictions centrally; delete all temporary `PLAN*.md` files in final implementation commit. Executors report changed files, SHA, tests, deviations and unresolved assumptions.
