# PLAN_VALIDATION.md — V2.5 Performance and Safety Verification

## Scope/baseline
Read `scripts/profile_cpu.sh`, `scripts/sample_collections.sh`, `scripts/profiling/allocator_profile.sh`, `crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}`, `crates/compact_std/examples/benchmark_profile.rs`, `PROFILE_V2_5_IMPLEMENTATION.md`, `PROFILE_V2_5_MEMORY.md`, `BENCHMARKS.md`. Own standalone profiling scripts and `PROFILE_V2_5_NEXT.md`; shared benchmark edits orchestrator-owned. Pinned runtime baseline `4f1c1aa222bf6a62ba6a72b4585ab24799135c42`.

## Comparability
Use immutable baseline/candidate release hashed binaries with identical compiler/feature/CPU/input, separate target directories; accounting-free timings without snapshot/counting allocator inside windows; allocation statistics separately. Alternate native/compact and baseline/candidate order, repeat noisy scenarios, validate all checksums. Record absolute/median/p95 time, throughput, CPU samples and available instructions/branches/cache counters, lock acquisitions and wait/hold, code size if relevant. Primary A2/A4/A5/B6/B8/B10, B3/B5 sentinels, final all-16 suite. Primary B10 1/2 workers on existing two-vCPU machine; 4/8 oversubscription only optional stress, physical multicore **not required**.

## Memory gates
Measure live/retained cage bytes, metadata and reserved slack, cursor high-water, free extents/fragmentation, peak RSS, idle RSS after thread exit, virtual reservation and committed pages where available. Default integrated ≤+2% retained and ≤+5% peak RSS, bounded TLS memory and no significant idle/fragmentation regression. Report absolute and percentage deltas, noise and any centrally approved exceptions.

## Validation commands
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Run full `.github/workflows/miri.yml` and `harness.yml`, Apple cross-target checks when available, collection differential tests and deterministic allocator concurrent interleavings, panic/recovery/exhaustion, race and duplicate-free checks, collision-security tests. No loss of raw-offset validation or initialized-prefix/drop invariants.

## Deliverable and final review
Create `PROFILE_V2_5_NEXT.md` with baseline/integrated SHAs, accepted/rejected/blocked prototypes and precise blockers, timings and ratios, counter limitations, memory/compatibility, CI, safety evidence and remaining hotspots. Independently review final diff against PLAN.md/workstreams; resolve contradictions centrally; delete all PLAN*.md in final implementation commit. Each executor reports touched files/symbols, SHA, tests, measurements and deviations.
