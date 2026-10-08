# PLAN.md — V2.4 performance investigation

Repository: Biddlebaddleboo/compact-std-library-rust
Target branch: main
Verified baseline: fc0a58bf08aa3bffc577ec2da8151d262b6a5f53

## Objective

Profile, do not optimize, the remaining V2.4 CPU gaps versus native Rust. Establish evidence-supported hotspots before any further production algorithm changes.

Current end-to-end compact/native benchmark ratios:
- A2 allocations: 1.41–1.44x slower
- A4 deque: 5.05–5.55x slower
- A5 hash: 3.96–4.00x slower
- B6 order book: 3.51–3.52x slower
- B8 cache churn: 2.51–2.55x slower

Also measure B3, B5, and B10 as regression sentinels.

## Verified repository facts

- Benchmark harness lives in `crates/compact_std/examples/benchmark_compare/` and includes `main.rs`, `measure.rs`, `scenarios.rs`, `models.rs`, `datasets.rs`.
- `main.rs` runs native and compact variants in separate child processes and supports `--scenario`, `--runs`, `--output`.
- `measure.rs` contains phase timing and a `CountingAllocator` wrapping `System`.
- Cage allocator has optional telemetry for locking, free-list visits, pending reuse, and batched release; instrumentation affects costs.
- Repository Miri workflow validates allocator and collections ownership.

## Invariants and non-goals

This is a profiling-only investigation, not an optimization pass. Do not change production algorithms, allocator policies, hashing defaults, ownership representations, or public APIs.

Preserve: 4-byte compact owners, 12-byte deque, 16-byte allocation header, 8-byte frozen descriptors, single cage, and u32 offset ownership. No software prefetching, new inline assembly, new retained pointers, or allocator redesign.

Temporary benchmark and instrumentation changes require correctness checks. Timed comparisons must use uninstrumented builds. Do not commit large machine-specific raw profiles.

## Workstreams

1. `PLAN_CPU_PROFILING.md`: sampling, call graphs, assembly; starts in benchmark harness.
2. `PLAN_ALLOCATOR_PROFILING.md`: allocation and release breakdown; starts in `crates/compact_backend_std/src/cage.rs`.
3. `PLAN_COLLECTION_PROFILING.md`: A4/A5/B6 collection breakdown; starts in `crates/compact_collections/src/{deque.rs,vec.rs,hash_map.rs,hash_control.rs}`.

Workstreams may run in isolated worktrees. CPU and collection profiling may proceed independently; allocator profiling also parallel-safe, provided ownership is respected. The orchestrator owns modifications to shared benchmark harness files and to summary documents. Allocator and collection executors should create separate temporary diagnostic probes/harnesses or propose shared instrumentation for central integration; never concurrently edit shared benchmark sources.

## Measurement protocol

- Verify latest main and compiler/toolchain/host before measurement.
- Pin host architecture, CPU, OS, Rust version, build profile, features, and benchmark command arguments.
- Collect clean release baseline using native and compact child processes.
- Build CPU samples, allocator telemetry, and targeted diagnostic counters separately; do not interpret instrumented timings as production timings.
- Repeat scenarios enough to establish stable medians/p95. Report time in absolute units and fractions of workload.
- Avoid summing overlapping CPU inclusive times or overlapping instrumented phase totals.
- Match logical checksums, work volume and live data characteristics between native and compact.
- Collect retained live bytes, peak cage usage, and peak process RSS distinctly; virtual reservation is not resident memory.

## Integration and execution

1. Confirm latest `main` and reconcile changed architecture.
2. Run baseline benchmarks; establish native/compact comparable conditions.
3. Assign three named workstreams to bounded executors in isolated worktrees.
4. Collect `PROFILE_CPU.md`, `PROFILE_ALLOCATOR.md`, `PROFILE_COLLECTIONS.md` and supporting artifact paths/command provenance.
5. Orchestrator compiles `PROFILE_SUMMARY.md`: ranked hotspots, measured absolute and relative cost, native comparisons, representation versus removable cost, RAM implications, optimization candidate/risk ranking and rejected hypotheses.
6. Verify all diagnostic changes and results without shipping accidental production optimizations.
7. Independently review the final diff, remove all `PLAN*.md` handoff files and commit accepted reports/diagnostic tooling only as explicitly required; no performance implementation.

## Validation

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --scenario A2 --scenario A4 --scenario A5 --scenario B6 --scenario B8 --output /tmp/profile-targeted.tsv
```

Run complete 16-scenario suite at least twice. Where unsafe code or ownership-related tests change, run Miri using the repository `.github/workflows/miri.yml` workflow. Preserve B3/B5/B10 sentinel coverage.

## Final-diff checklist

- [ ] No collection or allocator behavior modification shipped.
- [ ] Frozen sizes and offset contract unchanged.
- [ ] All representative benchmark checksums match.
- [ ] Profiler overhead excluded from production measurements.
- [ ] Profiles identify exact functions/operations, with evidence.
- [ ] Native baseline is compared on same machine/toolchain.
- [ ] RAM comparisons distinguish retained bytes and RSS.
- [ ] Findings rank next experiments without speculative claims.
- [ ] Required test/Clippy/Miri validation passed.
- [ ] PLAN*.md deleted before final implementation/report commit.

## Handoff

Verify latest main; read PLAN.md first; assign the three named profiling workstreams in isolated worktrees; collect reproducible evidence; integrate observations centrally; resolve contradictory measurements with repeat runs; produce PROFILE_SUMMARY.md; review the final diff; remove PLAN*.md; commit only authorized profiling deliverables. Do not implement optimizations.
