# PLAN_MAINTENANCE.md — Optional Dedicated Low-Priority Worker

## Prerequisites and ownership
Implement **after** deterministic memory manager and minimal cage manager pass synchronous safety, leak and memory tests. Own proposed new `crates/compact_backend_std/src/memory_maintenance.rs` and worker-specific tests. Read-only approved deterministic/region APIs and `crates/compact_std/examples/memory_profile.rs`. Region owner handles cage.rs edits; orchestrator handles lib.rs and shared configs.

## Contract
One **dedicated OS thread**, not an application executor/thread pool and not a dedicated CPU core. Best-effort lower scheduling priority where supported; failure to lower it cannot impact correctness. Event-driven wake/sleep, no busy polling; strictly bounded maintenance batch and lock hold. Worker performs only opportunistic coalescing of already-free extents, idle region-metadata cleanup, and eligible OS page release without invalidating stable cage offset mapping. No tracing, live-object discovery, moving objects, app callbacks/Drop, stop-the-world or globally blocking pages-release operation. All released objects must be reusable with worker indefinitely disabled; foreground fallback handles pressure without waiting for this worker.

## Implementation
Create opt-in disabled/enabled internal configuration, wake, quiescence/drain for tests and graceful shutdown or documented process-lifetime ownership. Worker obtains an independent bounded batch of already-proven-free work, releases shared locks before OS page calls, yields appropriately, and never holds release descriptors needed for correctness. Guard races with region reuse, shutdown, pending remote release, late worker wake and unsupported OS APIs. Linux is initial measured target, other targets compile with safe no-op/disabled fallback if priority/decommit APIs not supported.

## Evidence
Compare deterministic-only, worker enabled idle, worker active, worker permanently unscheduled. Test pause under allocation pressure, repeated thread churn, OS page release safety, wake/sleep/shutdown/reentrancy, no overlapping reuse, no worker-required progress, bounded lock holds. Measure foreground median/p95/p99/p99.9, worker CPU, RSS/idle, fragmentation and useful maintenance throughput. Reject worker entirely if benefit is small or it causes meaningful tail-latency impact; do not make worker implementation an unconditional V2.5 launch gate. Report exact file/symbol, SHA, tests, memory, latency and platform gaps.
