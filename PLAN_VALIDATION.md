# PLAN_VALIDATION.md — Deterministic Reclamation, Leak and Latency Gates

## Scope and pinned baseline
Baseline `030531ae4f6cedc9f5830f8cd4c4c9f051472443`. Start `PROFILE_V2_5_NEXT.md`, `PROFILE_V2_5_MEMORY.md`, `crates/compact_std/examples/{benchmark_profile.rs,benchmark_compare/main.rs,memory_profile.rs}`, `scripts/profile_cpu.sh`, `scripts/profiling/memory_profile.sh`, `crates/compact_backend_std/src/allocator_model.rs`. Own independent timing/memory reports and standalone profiling scripts; shared scenario/entrypoint changes orchestrator-owned. Compare identical compiler/toolchain/features/inputs and hashed telemetry-free baseline/candidate binaries. Alternate A/B order, repeat noisy cases and validate checksums; count allocation telemetry only in separate non-timing runs.

## Correctness / leak proof
Unique owner; no overlapping live cage ranges; exact accounted GlobalFree/Reserved/Live/Pending/LocallyReusable/Reclaimable/Quarantine states, including failed release and collector flush. No lost descriptors, no reuse with live/pending suballocations, safe remote drop after original thread exit, no reliance on worker for progress. Test repeated allocate/drop with stable cage high-water and actual capacity reuse (zero live alone insufficient), heterogeneous sizes/fragmentation, partial region occupation, repeated thread churn, exhaustion then recovery, stale/duplicate frees, malicious malformed offsets, failed reservation, panicking destructor, nested collectors, TLS exit, interrupted reclaim and quarantine reporting. Use controlled barriers and real-concurrency model checking/sanitizers where supported; test with worker permanently disabled and never scheduled.

## Latency and resource capture
Measure read/write, local alloc/recycle, remote release, region refill/return, pressure fallback median/p95/p99/p99.9 with sufficient samples, and CPU/lock time. Foreground work must not include unbounded maintenance scan or stop-the-world pause; separate scheduler noise from collector-induced latency. Compare A2/B8/B10 primary, A4/A5/B6 secondary, B3/B5 sentinels and **all 16** after integration. B10 1/2 workers mandatory on available two-vCPU machine; 4/8 oversubscribed optional only. Compare worker off, sleeping, active, unscheduled. Retained ≤+2%, peak RSS ≤+5% default; bounded metadata/slack, idle RSS, high-water, free extents, fragmentation, VmSize/resident memory separately. Explicit exception approval.

## Commands and gates
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```
Execute complete `.github/workflows/miri.yml` and harness CI; Apple cross-target check when available. No lost releases, wrong accounting, unbounded queues, stranded chunks, data races, STW, worker-dependent allocation progress or unapproved memory regressions. Miri alone does not prove thread synchronization. Independent review of final diff.

## Deliverable
`PROFILE_V2_5_DETERMINISTIC_MEMORY.md`: baseline/integrated SHAs, changed interfaces and files, safety transitions, leak stress/threads/interleavings, memory/latency percentiles, worker off/idle/active, all-16 ratios and accepted/rejected designs with remaining risks. Central orchestrator removes **all PLAN*.md** before final implementation commit.
