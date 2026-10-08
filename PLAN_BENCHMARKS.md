# PLAN_BENCHMARKS.md — Profiling and benchmark methodology repair

## Scope
Own crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}, and coordinate access to crates/compact_std/examples/collection_profile.rs and script instrumentation with PLAN_PROFILING.md. No production allocator/collection code changes. Baseline 8d11b1c.

## Verified methodological issues
- CountingAllocator atomics dominate some native sampled CPU profiles; native CPU profiles with tracking are not pure System allocator profiles.
- Native B10 yielded no useful CPU samples in old capture.
- Allocator uprobes failed to pair lock and transaction lifetime and distorted runtime substantially.
- A4 and B10 showed large shared-host timing variability.
- Focused diagnostics and authoritative benchmark_compare loops are not always equivalent.
- Miri now passes, but newly optimized unsafe paths need continuing coverage.

## Fix 1 — separate measurement modes
Provide explicit versioned mode or separate binary for low-overhead CPU sampling without per-allocation tracking, while retaining counting runs for allocation statistics. Do not silently compare different modes. Preserve harness checksums and ensure tracking state reset per child. Confirm disabling counts does not change logical workload.

## Fix 2 — B10 sampling
Ensure samples include all spawned worker threads, sufficient repetition and symbol information; document limits if unavailable. Do not claim a hotspot without resolved samples.

## Fix 3 — valid lock instrumentation
Replace invalid wildcard lock/transaction pairing with reliable per-thread nesting-aware pairing or alternate low-overhead sampling. Test synthetic nested and early-exit/unwind traces. Record lost events and instrumentation overhead; never use heavily probed timings as production lock latency.

## Fix 4 — repeatability
Record host CPU/kernel/compiler/features/load, use paired alternating native/compact timing where feasible, consistent warmup, explicit exclusion criteria for contaminated runs, and enough independent captures for noisy A4/B10. Preserve uninstrumented release benchmark as authoritative.

## Fix 5 — diagnostic parity
Document differences in black_box placement, loop iteration counts, initial capacity, growth, mutation order, checksum logic and allocator tracking between diagnostic collection_profile and benchmark_compare. Keep distinct diagnostic and timing outputs; confirm baseline checksums.

## Fix 6 — CI regression safeguards
Maintain Miri integration coverage for CompactHashMap::get_mut/remove_entry, ensure new unsafe changes get deterministic regression tests. Never disable failing safety checks.

## Validation
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
Run .github/workflows/miri.yml, available cross target checks and two full release suites. Report changed files, commit SHA, profiler cost, comparison-mode proof, raw artifact locations and unresolved platform limitations.
