# PLAN.md — V2.5 System-Wide Deterministic Allocation Reuse

Repository: `Biddlebaddleboo/compact-std-library-rust`
Target: `main`
Reviewed baseline HEAD: `eddf97f7d88547030ffa8d65fef5c00fbaac3e5a`
Pinned earlier allocator comparison: `030531ae4f6cedc9f5830f8cd4c4c9f051472443`
Current optimized implementation: `29b14fa77ae8df87a97fe0954666a9096239cfe3`

## Goal

Broaden the **existing** deterministic extent-reuse optimization to as many supported cage-backed allocation patterns as measurement and safety permit: uncontended/single-threaded allocation, repeated compatible sizes, mixed sizes, varying alignment, allocation churn and collection growth/shrink. Optimize the shared `CageAllocation<T>` infrastructure, not individual benchmarks. Retain **one** cage allocator and bounded local reuse; introduce **no additional allocator hierarchy, owner-affine regions or background GC thread**. Preserve the 2-worker B10 gain and address one-worker/B4/B7/A2 regressions while improving the full workload distribution.

This is a planning-only handoff. The orchestrator must inspect the latest main and implement after separate execution authorization; PLAN*.md are temporary and must be removed in the final implementation commit.

## Verified repository facts and measured evidence

- `crates/compact_backend_std/src/cage.rs` contains process-wide `CageState`, `Mutex<Allocator>`, global cursor, size classes `[32,40,112,528]`, intrusive free list, release batching, pending-release queue, owner headers and local-cache integration.
- `crates/compact_backend_std/src/deterministic_memory.rs` contains `LocalCacheState::{take_compatible,push,bytes,clear}`, `ReleaseExtent`, `RecycledExtent`, `MAX_ACTIVE_LOCAL_CACHE_OWNERS=16`, `LOCAL_CACHE_CAPACITY=16`; cache reuse currently insists on **exact extent length**, alignment compatibility and a global cage-wide local-cache budget.
- `cage.rs::lock_for_allocation` calls uncontended `try_lock` until an actual `WouldBlock` sets `local_reuse_activated`; `local_reuse_enabled` gates caching on that flag. Ordinary allocations without contention keep using the shared allocator. `finish_lock` drains at most one bounded pending-release batch under the allocator lock.
- `cage.rs::flush_local_cache_state` publishes cached extents or queues them for retry; `drain_pending_releases_locked` handles at most 64 release descriptors. A corrupted/unrecoverable state faults the cage closed. `ReleaseCollector::flush_with` has publication recovery logic.
- `cage.rs::CageAllocation<T>::allocate` and `try_resize` are central integration points; normal `CompactVec`, `CompactHashMap`, `CompactVecDeque` and owned cage values call into this infrastructure.
- The test-only sequential `allocator_model.rs` is **not** proof of concurrent atomic correctness; region generation/ABA/remote pin/retirement requirements remain unresolved. Do not accidentally implement the earlier owner-affine chunk proposal in this pass.
- `PROFILE_V2_5_DETERMINISTIC_MEMORY.md` reports B10 2-worker median 2.757→1.214 ms (−56.0%), lock acquisitions 16000→4, but 1-worker B10 0.441→0.465 ms (+5.4%); A2 +4.2%, B4 +5.6%, B7 +6.1%, B3 +4.2%, B8 −1.6%. These are workload-specific observations from a two-vCPU AArch64 host; non-B10 deltas need paired repeat confirmation.
- Current report measured 101 repetitions and p95, not statistically useful p99.9; only one RSS sample per scenario. Harness and Miri workflows passed for reported latest HEAD.

## Implementation scope — inspect these symbols first

Primary write scope, one owner:
- `crates/compact_backend_std/src/cage.rs`: `CageState`, `Allocator`, `ReleaseCollector::{take_compatible,flush_with}`, `LocalReuseCacheSlot::drop`, `lock_for_allocation`, `finish_lock`, `flush_local_cache_state`, `flush_current_local_cache`, `drain_pending_releases_locked`, `enqueue_pending_release`, `local_reuse_enabled`, `local_reuse_eligible`, `with_local_reuse_cache`, `take_pending_reuse`, `block_layout`, `allocate_block`, `allocate_from_cursor`, `size_class_index`, `CageAllocation<T>::allocate`, `CageAllocation<T>::try_resize`, `release`, `release_many`, `release_many_locked`, `CompactRuntime::{allocator_stats,validate_allocator_state,used_bytes,remaining_bytes}`.
- `crates/compact_backend_std/src/deterministic_memory.rs`: `LocalCacheState::{take_compatible,push}`, budget/eviction accounting, compatibility checks. Preserve the one existing local extent cache; no second allocator layer.

Tests and telemetry, same integrated owner unless separately isolated:
- `crates/compact_backend_std/tests/integration.rs`
- `crates/compact_backend_std/src/allocator_model.rs` (state-transition tests only; avoid claiming sequential model proves concurrency).
- Existing backend unit tests inside `cage.rs` and `deterministic_memory.rs`.
- `crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}`, `crates/compact_std/examples/benchmark_profile.rs`, `crates/compact_std/examples/memory_profile.rs`, `scripts/profile_cpu.sh`, `scripts/profiling/memory_profile.sh`.
- `PROFILE_V2_5_DETERMINISTIC_MEMORY.md` is the existing baseline/results reference; record new results in proposed `PROFILE_V2_5_SYSTEM_WIDE_REUSE.md`.

Read-only collection entrypoints initially:
- `crates/compact_collections/src/vec.rs`: `CompactVec::{reserve,push,shrink_to_fit,as_slice,as_mut_slice}`.
- `crates/compact_collections/src/deque.rs`: `CompactVecDeque::{reserve,push_back,pop_front,with_view}`.
- `crates/compact_collections/src/hash_map.rs`: `CompactHashMap::{insert,rehash,shrink_to_fit}`.
Do **not** rewrite collections unless a measured allocation compatibility bug or failing test establishes necessity.

## Phase 0 — Baseline and hypothesis matrix

1. Reverify remote main HEAD, compare since reviewed baseline, reconcile relevant edits before touching files. Pin candidate and baseline binaries and toolchain/flags. Record all 16 scenario checksums and A/B order.
2. Collect separate non-timing allocator telemetry: number of local cache attempts/hits, reasons for misses (disabled, not cached, incompatible size/alignment, owner limit, budget, evictions), global lock acquisitions/wait, pending queue length/drain, cursor/high-water, free list search and in-place resize outcomes. Add only bounded gated telemetry; no timing-run instrumentation.
3. Quantify performance of uncontended `try_lock`, TLS cache access, exact-extent linear scan, conditional atomic loads and release path. Profile A2/B3/B4/B7/B8/B10 rather than assume every regression is allocator-related.
4. Form explicit proposed candidates with expected affected workload and memory risk; set evaluation thresholds before experiments.

## Phase 1 — Universal/adaptive deterministic reuse

1. Prototype **always-attempt-compatible-local-reuse** without prior contention, using the existing `LocalCacheState`. Contrast with current contention-gated behavior and a cheap, predictable adaptive policy. Do not simply add TLS access on every allocation without measuring the miss cost.
2. Make cache activation/deactivation policy explicit and benchmark deterministic cases: never allocated before, repeated same-size, varied sizes, refill after cache eviction, 1/2 workers and thread churn. Existing runtime behavior and fallback must remain correct irrespective of activation history.
3. Evaluate release fast paths and cache-owner cap; do not silently retain released extents beyond a bounded budget. Release descriptors must be reliably published if cache full/disabled, including after thread exit.
4. On misses, the existing allocator remains authoritative; no unproven lock-free global allocator bypass.

## Phase 2 — More compatible sizes/alignment and resizing

1. Investigate *representable* extent compatibility beyond the four fixed exact classes, using `block_layout` to verify required payload, alignment and exact block origin/header. Consider bounded size-class expansion or compatible larger extent reuse with safe splitting and handling of unusable small tails; prototype each separately. A larger extent may only be reused without splitting if actual usable capacity and header `block_len` are represented correctly and the resulting physical accounting remains consistent. Never misrepresent live capacity, create overlapping ranges or accept alignment by size alone.
2. Keep cache bytes/entry count capped and ensure a mixed-size workload does not lock useful memory into unpopular classes. Compare exact match, best-fit bounded scan and safe fallback under fragmentation.
3. Probe `CageAllocation<T>::try_resize`, `allocate_block`, release + fresh allocation sequences for opportunistic reuse with normal ownership and initialization guarantees. Never move elements with invalid initialized-length metadata or call destructors under allocator locks.
4. Preserve documented behavior of reserved vs live vs pending vs reusable bytes. Runtime snapshots must not give false leak-free proof merely by flushing current TLS while other threads retain cached blocks.

## Phase 3 — Correctness and reclamation guarantees

For every candidate establish:
- Unique allocator-issued owner, disjoint extents, valid alignment, checked offset arithmetic and 16-byte allocation header preservation.
- Destruction exactly once, no read/use after release, no double publication or ABA via reused offsets, no exposure of stale data as initialized.
- Remote drop after home-thread exit, simultaneous cache eviction/publication, nested release collectors, concurrent pending-release drain, poison/fault-closed behavior and panicking destructor recovery.
- Cache accounting and pool counts remain bounded; data remains globally reclaimable after completed thread exits; repeated drop/reallocate cycles demonstrate actual reusable capacity.
- No worker thread needed. No global stop-the-world or unbounded foreground scan. Fail safely and visibly if recovery cannot be guaranteed.

Use deterministic barriers, model-based invariants and where feasible Loom or sanitizer testing; clearly label proof limits. Keep hypothetical chunk model separate from actual implemented extent cache.

## Phase 4 — Performance selection and acceptance

Use isolated reversible experiments; only integrate candidates that have:
- Deterministically passing memory/ownership/remote-release tests and Miri.
- Broad net benefit across **all 16** scenarios, not just B10. Prioritize removal of 1-worker B10, A2, B4 and B7 regressions; explicitly monitor B3/B5 and already-fast cases.
- Two-worker B10 benefit preserved within noise (comparison to current implementation), substantial reduction in lock acquisitions where repeated reuse is possible, and no worse p95/p99 tails where statistically supported.
- Default **≤ +2% retained heap** and **≤ +5% peak RSS** compared with the pinned current optimized baseline; measure repeat idle RSS/thread churn, live bytes, cached extents, cursor high-water and fragmentation. Separate high-RAM experiments may use larger limits but must not silently become defaults.
- No added allocator, region hierarchy, background reclamation thread, public API changes or altered four-byte `CageAllocation<T>`, 16-byte `AllocationHeader`, four-byte `CompactVec` and twelve-byte `CompactVecDeque` layouts.

Use ≥ enough timed samples for p99 (and p99.9 only when sample count and noise make meaningful); report precise distributions and paired repetitions. B10 1-worker/2-worker on existing two-vCPU Neoverse-N1 host mandatory; 4/8-worker oversubscription optional, not a prerequisite. If a proposed broad fast path loses to adaptive or incumbent, retain the fastest **safe system-wide policy**, including the original gated policy as fallback. Do not force all pattern categories into a single policy if that regresses overall performance.

## Validation commands

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Run complete `.github/workflows/miri.yml` and benchmark harness as defined in repo, supported Apple cross-target check, and independent final diff audit; no real sleeps for deterministic concurrency tests.

## Workflow and ownership

This is a **single tightly coupled workstream**, not split into artificially parallel executors. One owner edits cage/local reuse symbols, tests and measurements. Baseline profiling and independent result audit can be done without concurrent writes. Orchestrator resolves any interface contradictions and keeps changes within the named implementation surface unless compilation/tests, moved symbols, another call site or correctness require expansion.

Before implementation verify latest main; report experimental candidates, exact files and symbols changed, commit SHA(s), benchmark and RSS tables, deviations, tests and remaining caveats. Integrate results in `PROFILE_V2_5_SYSTEM_WIDE_REUSE.md`; inspect final diff; **delete all PLAN*.md before final implementation commit**. This plan commit itself contains only PLAN.md, no production changes.
