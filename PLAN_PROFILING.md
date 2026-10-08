# PLAN_PROFILING.md — Post-optimization CPU and memory profiling

## Scope / starting points
Read PROFILE_CPU.md, PROFILE_COLLECTIONS.md, PROFILE_ALLOCATOR.md, PROFILE_SUMMARY.md and BENCHMARKS.md. Own standalone profiling scripts including scripts/profile_cpu.sh, scripts/sample_collections.sh, scripts/profiling/allocator_profile.sh, scripts/profiling/allocator_uprobes.sh; coordinate any shared diagnostic tool edits with PLAN_BENCHMARKS.md owner. Output PROFILE_V2_4_ROUND2.md. Production source and benchmark harness are read-only to this workstream.

## Verified facts
Previous CPU sampling predates cross-crate inline, one-header CompactVec push and CompactHashMap::{insert,get_mut,remove_entry} single-resolution changes. Old B10 worker symbols were missing and uprobe runtime distortion was severe. Original profiler CPU shares are hypotheses, not new baseline truths.

## Procedure
1. Pin SHA, CPU architecture/model, host load, kernel, Rust/LLVM, features, release flags and profiler permissions. Benchmark all 16 scenarios twice and repeat A4/A5/B6/B8/B10 enough for medians/p95 on same host.
2. Profile native and compact child processes using low-overhead sampling, capturing all threads. Record sample counts, lost samples, inclusive/exclusive call graphs, event provenance and profiling-build effect. Use cycles/instructions/cache misses only if supported and report unavailable metrics honestly.
3. Check A4 deque mutations and build, A5 hash hit/miss/insert/remove and probe, B6 indexed updates/build/retain/snapshot, B8 hash/release/allocator separation, B10 worker-thread lock/atomic/scheduler paths. Keep A2 near-native and B3/B5 as sentinels.
4. Compare optimized AArch64 and x86-64 generated code for actual inlining, cage base/header reloads, bounds checks, register spills, branch/loop effects and missed vectorization.
5. For each hotspot create an isolated reversible experiment and A/B test. Use production no-telemetry build for speed; never add overlapping inclusive samples or instrumented phase totals.

## Deliverable / validation
PROFILE_V2_4_ROUND2.md: current absolute/relative costs, top symbols per scenario, artifact paths/commands, memory distinctions (retained live bytes vs cage high-water vs process RSS), evidence quality, suspected vs confirmed bottlenecks, accepted/rejected experiments, and priority/risk ranking. Verify checksums and avoid undocumented changes to production implementation. Report SHA, changed files, test runs, open limitations.
