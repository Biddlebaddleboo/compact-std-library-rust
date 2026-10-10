# PLAN.md — V2.5 Allocation-Pattern-Aware Deterministic Memory Management

Repository: `Biddlebaddleboo/compact-std-library-rust`; target: `main`; verified baseline: `184ee63b9b196d5c5ecb858fb7cbf6080326dd9f`.

## Objective
Keep **one authoritative shared cage allocator** with several inexpensive, deterministic policies selected by allocation properties, not collection names. Improve broad allocation patterns without tracing GC, another allocator hierarchy, owner-affine regions or a background collector. Preserve ownership, leak-free reclamation, compact layouts, the two-worker B10 improvement, and memory budgets. Verify latest remote main and reconcile relevant changes before implementation.

## Verified repository facts
- `cage.rs` contains `CageState` / `Mutex<Allocator>`, bounded release batching, pending-release recovery, `lock_for_allocation`, and `CageAllocation<T>` alloc/resize/release paths.
- `deterministic_memory.rs` implements a bounded thread-local exact-extent cache via `LocalCacheState::{take_compatible,push}`. Local classes `[32,40,112,528]`, global budget up to 4 KiB, at most 16 owning threads and 16 entries per owner. Cache currently activates after real lock contention.
- `PROFILE_V2_5_SYSTEM_WIDE_REUSE.md`: unconditional reuse sped single-worker B10 ~5.1–5.3% but slowed B4 ~89%; broader-size matching had no repeatable B10 benefit; replacing try_lock with lock regressed single-worker B10. Existing contention-gated policy retained; opt-in telemetry was added in commit `184ee63b9`.
- Earlier two-worker B10 improved ~56% vs `030531ae4`; B4, B7, A2 and one-worker B10 remain concerns. Sequential `allocator_model.rs` does not prove concurrency safety.

## Implementation scope and symbols
**Exclusive implementation owner** for `crates/compact_backend_std/src/cage.rs`: `CageState`, `Allocator`, `lock_for_allocation`, `finish_lock`, `local_reuse_enabled`, `local_reuse_eligible`, `with_local_reuse_cache`, `take_pending_reuse`, `allocate_block`, `allocate_from_cursor`, `block_layout`, `size_class_index`, `release`, `release_many`, `release_many_locked`, `CageAllocation<T>::allocate`, `CageAllocation<T>::try_resize`, `CompactRuntime::{allocator_stats,validate_allocator_state}`.
Also `crates/compact_backend_std/src/deterministic_memory.rs`: `LocalCacheState::{take_compatible,push}`, `ReleaseExtent`, `RecycledExtent`, cache eligibility, bounded accounting.
Tests: `crates/compact_backend_std/tests/integration.rs`, existing unit tests in those two files, `crates/compact_backend_std/src/allocator_model.rs` for model-only checks.
Harness/report: `crates/compact_std/examples/{benchmark_profile.rs,memory_profile.rs}`, `crates/compact_std/examples/benchmark_compare/main.rs`, `PROFILE_V2_5_SYSTEM_WIDE_REUSE.md`; proposed output `PROFILE_V2_5_ALLOCATION_POLICIES.md`.
Read-only collection callsites unless needed by correctness or evidence: `crates/compact_collections/src/vec.rs` `CompactVec::{reserve,push,shrink_to_fit}`, `deque.rs` `CompactVecDeque::{reserve,push_back,pop_front}`, `hash_map.rs` `CompactHashMap::{insert,rehash,shrink_to_fit}`. Avoid broad search absent moved symbols, changed architecture, compile/test failures or safety needs.

## Policy design (candidate additions)
A private lightweight policy selector may use conceptual `AllocationPolicy::{LocalExactReuse,SharedFreeReuse,DirectCageAllocation}`; this enum is **proposed**, not existing, and a simpler representation is acceptable if faster. Selection uses cheap size, alignment, size-class eligibility, existing cache availability, contention state, and *only if worthwhile* bounded per-class hit/miss indicators; no collection-name dispatch and no extra allocation for policy evaluation.

- **Small repeatable allocations:** selectively attempt existing exact local cache; capture uncontended B10 opportunity without always-scanning B4.
- **Small low-reuse/mixed-size:** bypass unproductive local scan and use current shared free-space path.
- **Medium compatible extents:** investigate bounded compatible reuse and fragmentation tradeoffs. Validate true `block_layout`, alignment, header, physical size, split tails and accounting; do not pass off a larger extent as a smaller one without correct representation.
- **Large buffers:** prefer direct shared allocation and verified in-place resizing. Do not spend tiny-cache overhead on unsupported sizes.
- **Contended allocations:** retain incumbent activation and bounded local reuse fallback, preserving two-worker B10 behavior.
- **Cross-thread release:** always use authoritative, safe release publication independent of allocating-thread lifetime; policy selection cannot weaken this contract.

## Phase 0 — Baseline and telemetry
Verify HEAD; pin baseline binaries/flags/toolchain. Run all 16 workloads with checksums and alternate-order comparisons. In **separate telemetry builds**, measure class distribution, alignment, local lookups/hits/miss reasons/evictions, ownership caps and budget, lock acquisitions/wait, pending-queue activity, global free-list visits, cursor fallback/high-water and in-place resize results. Avoid telemetry during timing. Attribute regressions empirically; do not assume they are allocator-caused.

## Phase 1 — Static deterministic policy experiments
Prototype constant-cost size/alignment and existing cached-class-aware eligibility. Keep changes reversible, compare first to incumbent under B4 and B10 1/2-worker then full suite. Prevent missing fast paths from paying TLS scan/miss cost. Check same-size, mixed-size, cold, cache-eviction, thread-churn and large-buffer workloads. Do not change the shared allocator's authority.

## Phase 2 — Optional adaptive policy experiments
Only if static routing leaves measurable opportunity, test bounded per-class deterministic counters/cooldowns. Document saturation/reset, per-thread vs shared state, atomic order, switch transitions and finite metadata. Do not introduce per-request expensive synchronization. Compare policy-selection cost itself. Drop the adaptive candidate if it causes B4-like miss overhead or harms broad results.

## Phase 3 — Integrate useful common improvements
Apply proven policy to shared `CageAllocation<T>` alloc/reuse/release/resize. Maintain shared fallback for invalid alignment, unsupported class, exhaustion, unavailable cache owner and failed local attempts. Explore compatible free extents and resize optimizations **only** with robust accounting and independent safety gates. Keep ordinary object access independent of policy/allocator synchronization; no per-collection allocator rewrites.

## Safety and recovery invariants
Every live extent has exactly one allocator-issued ownership path; intervals are disjoint; header alignment and capacity valid; no double publication/ABA/data races or stale initialized data after reuse. Normal `Drop` runs exactly once. Cached and pending extents are bounded and tracked; failed publication never silently drops its only descriptor. Remote frees after owner thread exit remain reclaimable. Invalid metadata faults closed; do not incorrectly claim released memory. Repeated allocate/drop and thread churn must demonstrate **actual capacity reuse**, not only counters reaching zero. No unbounded foreground scan or dependence on background scheduling. Verify nested collectors, panics, mutex poison and exhaustion-reclaim. Model-based tests plus barrier-controlled real concurrency; Miri/sequential model alone cannot prove memory ordering.

## Benchmarks, budgets and acceptance
All 16 scenarios mandatory; primary A2, B3/B5, B4, B7, B8 and B10 1/2-worker. Preserve B10 two-worker speedup against current incumbent within measurement noise, eliminate significant B4-style regressions, seek reproducible benefit in at least one additional pattern and optimize aggregate results. Record paired repeated medians and p95; p99/p99.9 only if enough samples. Current **two-vCPU AArch64** machine sufficient; 4/8 oversubscribed workers optional. Check telemetry-free timing, checksum parity, lock counts, high-water, repeated retained RSS, peak RSS, fragmentation and thread exit.
Default vs verified optimized baseline: **≤+2% retained memory and ≤+5% peak RSS**, bounded cache/metadata and no growing thread-local reservations; request approval for exceptions. Preserve four-byte `CageAllocation<T>`, 16-byte header, four-byte `CompactVec<T>`, twelve-byte `CompactVecDeque<T>`, public APIs, alignment and exhaustion semantics. Accept no policy solely to satisfy plan if incumbent is faster/safer.

## Validation
```sh
cargo fmt --all -- --check
cargo check --workspace --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check
```
Run full repository Miri and benchmark CI plus supported Apple cross-target check, and independent final diff review. Prefer barriers, injected counters and deterministic schedules over real sleeps. Test policy boundaries, repeated reuse, mixed sizes, alignment, cache limits/eviction, exhaustion, remote release, exit, panic, stale descriptor and concurrent activation.

## Workstream coordination and execution handoff
One **tightly coupled workstream** owns allocation-policy design, cage integration and local cache changes. No artificial workstream decomposition; baseline measurement and independent review may run separately without shared writes. Orchestrator resolves contradictions centrally, allows scope expansion only for moved code, compilation/tests, compatibility or correctness.

1. Verify latest main; read this plan; capture baseline.
2. Prototype static then optional adaptive policies; test each independently.
3. Integrate only accepted candidates; run all correctness and performance gates.
4. Record changed symbols, SHAs, tested/rejected candidates, performance tables, RSS and unresolved assumptions in `PROFILE_V2_5_ALLOCATION_POLICIES.md`.
5. Independently verify final source diff against this plan.
6. **Remove all PLAN*.md files before the final production implementation commit**; commit source/results without temporary planning artifacts.

**This planning commit does not authorize production code implementation by this assistant.**
