# PLAN_BENCHMARKS.md — CI Repair and Performance Baselines

## Scope
Start with .github/workflows/harness.yml, Cargo.toml, Cargo.lock (if present), crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs}, crates/compact_std/examples/benchmark_profile.rs, scripts/profile_cpu.sh.

## Confirmed CI failure
GitHub Actions run 37725406606 stopped at `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check`. Cargo reports that it cannot update Cargo.lock because --locked was supplied; this is not a verified checksum mismatch.

## Repair
Establish whether Cargo.lock is intentionally tracked. If tracked, regenerate with intended Rust toolchain, inspect dependency changes, commit consistent file. If omitted by library-workspace policy, use a documented CI resolution strategy rather than blindly dropping --locked; avoid unreviewed dependency version changes. Reproduce exact workflow command from clean checkout and obtain passing CI.

## Reliable baselines
Use accounting-free benchmark_profile with System allocator and no per-phase snapshots for production CPU timing; retain benchmark_compare --mode measure for allocation stats. Assert logical checksum parity across modes. Pin hashed release artifacts and production no-telemetry feature set to avoid concurrent build hardlink swaps. Record compiler, CPU, kernel, features, warmups, native/compact interleaving, host load and run counts; repeat noisy A4/B10. Track medians/p95, CPU symbols, retained bytes, high water and RSS separately.

## Tests and handoff
Run harness --self-check, full 16-scenario native/compact parity, GitHub harness workflow, workspace fmt/tests/Clippy and Miri if new unsafe tests require it. Report root cause, files, SHA, dependency-lock policy, baseline files and exact validation.
