# PLAN_VALIDATION.md — Header/Vector Profiling and Integration

## Scope
Own crates/compact_collections/src/vec.rs and vector-specific tests; create PROFILE_V2_4_ROUND3.md with centrally integrated results. Read cage.rs read_header, validate_typed_header, CageAllocation::{as_mut_slice,uninit_capacity_mut} but allocator workstream owns any cage changes. Do not edit shared harness files.

## B6 experiment
Benchmark existing CompactVec::as_mut_slice borrow once per order-book update round against per-index IndexMut, identical update stream and checksum, avoiding borrow across retain/growth/replacement. Investigate separate B6 construction and vector push/retain/snapshot costs. Prefer existing API documentation over new API if sufficient.

## Header validation
Inspect optimized AArch64 and x86-64 codegen for inlined range checks, redundant calculations and register usage. Distinguish external/unsafe offset ingress from already-owned, valid borrow scope. Propose precise safe eliminations with proof; any cage.rs change belongs to allocator executor. Never globally bypass typed-header or lifetime validation.

## Fresh profiling
After integrating experiments, collect accounting-free native/compact CPU profiles, absolute medians/p95, shifted top self symbols, instructions/cycles when available and B10 all-worker samples. Use hashed telemetry-free binaries, separate measurements for allocations and RSS. Track frozen layouts and B3/B5 sentinel wins.

## Required validation and final report
Run workspace fmt/check/test/clippy, all GitHub Miri commands, benchmark CI checksum parity, x86-64 Apple target check if toolchain installed, at least two full 16-scenario suites and repeated noisy workloads. PROFILE_V2_4_ROUND3.md must distinguish confirmed hypotheses from guesses, accepted/rejected experiments, correctness results, memory tradeoffs and remaining bottlenecks. Handoff changed files, SHA, test outcomes, limitations and anomalies.
