# PLAN_CPU_PROFILING.md — CPU sampling and generated-code investigation

## Objective
Identify inclusive and exclusive CPU hotspots in A2/A4/A5/B6/B8 for both native Rust and compact Rust, with B3/B5/B10 as sentinels. This is profiling only.

## Implementation scope and verified facts
Start with `crates/compact_std/examples/benchmark_compare/main.rs`, `measure.rs`, `scenarios.rs`; read `BENCHMARKS.md`. The benchmark harness already runs variants in child processes, accepts `--scenario`/`--runs`/`--output`, records phase timing, and installs a CountingAllocator. Do not change production code.

## Procedure
1. Record system information (CPU, architecture, OS, Rust toolchain, optimization flags and features). Pin commit `fc0a58bf08aa3bffc577ec2da8151d262b6a5f53`.
2. Build uninstrumented release benchmarks and reproduce scenario medians/p95 and matching checksums. Warm before measurement and repeat runs.
3. Build separately with debug symbols and frame pointers as supported. Record any benchmark timing delta introduced by profiling flags.
4. Use suitable native sampling tool (Linux perf, macOS Instruments or equivalent). If hardware counters are unavailable, say so and use supported sampling methods without fabricating hardware data.
5. Profile native and compact child executions, not primarily the parent process. Use sufficient workload repetition to resolve sub-millisecond B6 phases.
6. Produce per-scenario top functions, inclusive/exclusive times, call graphs, and absolute cost estimates; identify shared overhead versus scenario-specific cost.
7. Inspect optimized AArch64 and x86-64 assembly for repeated cage-base/header resolutions, bounds checks, divisions/modulo, branch cost, missed inlining or vectorization. Record evidence rather than presupposing any cause. No new prefetch/assembly.

## Correctness and bias controls
Time comparisons must exclude profiling instrumentation and tracing overhead. Do not sum overlapping inclusive costs as independent totals; do not treat a single sample as conclusive. Preserve native/compact work parity and checksum outputs.

## Write scope
Create `PROFILE_CPU.md` and small reproducible scripts or command transcripts if useful. Treat shared benchmark harness source as centrally owned; requests to edit it must go through orchestrator. No production code changes.

## Handoff
Report commands, host/toolchain, sampling method, symbolized data, graphs/artifact locations, top hotspots (absolute time and fraction), uncertainty/noisy results, and ranked next steps. Include changed-file list, SHA if applicable, tests run and deviations.
