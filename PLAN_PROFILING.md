# PLAN_PROFILING.md — Whole-Framework Hotspot Survey

## Scope and verified starting points
Read `scripts/profile_cpu.sh`, `scripts/sample_collections.sh`, `scripts/profiling/allocator_profile.sh`, `crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}`, `crates/compact_std/examples/benchmark_profile.rs`, `crates/compact_std/examples/collection_batch_probe.rs`, `PROFILE_V2_4_ROUND4.md`, `BENCHMARKS.md`. Own standalone profiling scripts and reports; common benchmark entrypoints/scenarios orchestrator-owned; production code read-only.

## Required baseline
Verify source SHA, rustc/LLVM, flags, target CPU, kernel, feature set, allocator policy and artifact hashes. Avoid concurrent mutable Cargo build artifacts, use pinned hashed binaries/isolated target dirs. Use accounting-free `benchmark_profile` for production timings and instrumented `benchmark_compare --mode measure` strictly for accounting; checksum all 16 native/compact scenarios. Alternate order and repeat noisy samples, include multiple worker counts and multicore hardware when available.

## Comprehensive survey
Profile all A1–A6 and B1–B10 without excluding compact wins (A3/B3/B5/B7/B9). Break apart construction, mutation, lookup, iteration, clone, drop, serialization, string/path, hashing, allocator and realistic request/dispatch phases. Capture top inclusive/exclusive symbols, cycles, instructions, branches/misses, cache misses where supported, memory traffic, copies, header validations, hash cost, lock/futex samples. Distinguish unsupported counter from zero.

Memory: live/retained bytes, allocation counts, high-water, peak/idle RSS, virtual cage reservation versus committed pages, fragmentation, size classes, bounded per-thread slack. Trace attribution carefully for B10 child/worker threads, note post-window integrity-check samples separately.

## Opportunities and experiments
Rank candidates by absolute CPU cost, usage frequency, cross-library consumers, projected savings, complexity, correctness risk, compatibility and memory. For every hotspot name the exact symbol, observed source mechanism, existing contract, proposed alternative and falsifiable test. Compare existing API versus existing API; new batch versus equivalently batched native; report cross-API advantage separately. Isolated A/B experiments only; collect baseline/candidate medians, dispersion/p95, counters, RSS, checksums, code size where relevant and regressions before integration.

## Deliverables
`PROFILE_V2_5_BASELINE.md` and an opportunity matrix of every meaningful hotspot, prioritized architecture contracts, rejected experiments and source-specific starting points. Reprofile on final integrated state; no production code changes here.
