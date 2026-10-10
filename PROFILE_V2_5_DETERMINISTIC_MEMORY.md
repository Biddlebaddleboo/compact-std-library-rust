# V2.5 Deterministic Memory Implementation Report

## Integrated result

- Remote `main` was fast-forwarded to `24260d565cb7b43652aa6576207fdb9c051c0e0a` before implementation.
- Pinned allocator baseline: `030531ae4f6cedc9f5830f8cd4c4c9f051472443`.
- Synchronous extent-reuse implementation: `29b14fa77ae8df87a97fe0954666a9096239cfe3`.
- The change adds bounded, thread-local reuse for exact compatible extents after an actual allocator-lock contention event. It also retains release descriptors in an in-cage retry queue when global publication fails.

This completes the safe synchronous extent-reuse slice. It does not complete the planned owner-affine region manager. The region identity, generation/ABA, remote-release pinning, and concurrent accounting proofs remain open in the allocator contract, so chunk reservation and region return were not integrated. The maintenance worker is also omitted: its plan depends on a complete synchronous region manager, and the current allocator has no independent page-release work for a worker to perform.

## Changed files and behavior

- `crates/compact_backend_std/src/deterministic_memory.rs` defines fixed-size local cache storage. A cage-wide byte budget is `min(capacity / 50, 4096)`, with at most 16 cache-owning threads and 16 extents per thread. Only size-class-compatible blocks are cached; reuse requires an exact block length and compatible alignment.
- `crates/compact_backend_std/src/cage.rs` activates reuse after `try_lock` observes contention. An uncontended `try_lock` is the allocation lock acquisition. A cache hit initializes the new allocation header before removing its descriptor. Cache ownership transfers to the thread that releases the block; another thread cannot reuse it from that cache. Thread exit flushes that thread's cache.
- Cached extents remain included in allocator `live_bytes` until reused or published to the global free lists. `used_bytes`, stats, and validation flush the calling thread's cache and process at most one pending-release batch. A resize that cannot grow first flushes the caller's cache and retries once.
- The intrusive pending-release queue uses a mutex to prevent ABA on reused cage offsets. Its queue links live inside the released cage blocks; allocator transactions drain at most 64 descriptors. Collector flush preserves descriptors across failed publication attempts. An invalid or unrecoverable allocator state faults the cage closed; production stats do not expose a separate quarantined-byte counter.
- `allocator_model.rs` adds local-reuse and quarantine transitions to the test-only sequential model. `tests/integration.rs` exercises same-range reuse, 32-thread churn, cross-thread drop/owner-thread exit, panicking destructors, exhaustion recovery, and allocator validation.
- The public API and frozen layouts are unchanged. `CageAllocation<T>` remains 4 bytes, `AllocationHeader` remains 16 bytes, and `CompactVecDeque` remains 12 bytes. `AllocatorStats.live_bytes` is documented to include authoritative pending releases.

## Validation

Passed against the integrated source:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- Every command in `.github/workflows/miri.yml` with `MIRIFLAGS=-Zmiri-disable-isolation` and `PROPTEST_CASES=16`: core (3), collection ownership (22), deque view (8), batch access (9), backend integration (1), backend library (47), and V2.4 behavior (7) tests passed.
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- Release benchmark harness self-check: measure/profile checksums matched for all 16 scenarios.

The runtime integration includes barrier-controlled contention and cross-thread release tests. Miri and the sequential model do not prove concurrent memory ordering; no Loom model or thread sanitizer run was performed. The final source diff was reviewed in this workstream, without a separate independent reviewer.

## Performance

Release profile runs used the pinned baseline binary and the integrated candidate, Rust 1.95.0, `json,toml`, Linux AArch64, and two available CPUs. The table compares compact `end_to_end` medians and p95s from 101 timed repetitions per scenario, with native then compact order. Allocator accounting was disabled for timing, and scenario checksums matched. Values are milliseconds; median delta compares the candidate median with the baseline median.

| Scenario | Baseline median / p95 | Candidate median / p95 | Median delta |
| --- | ---: | ---: | ---: |
| A1 | 0.071 / 0.082 | 0.071 / 0.101 | +0.2% |
| A2 | 1.624 / 1.650 | 1.692 / 1.770 | +4.2% |
| A3 | 0.582 / 0.651 | 0.595 / 0.693 | +2.3% |
| A4 | 1.066 / 1.120 | 1.070 / 1.101 | +0.4% |
| A5 | 2.803 / 4.052 | 2.751 / 3.165 | −1.8% |
| A6 | 1.232 / 1.281 | 1.268 / 1.303 | +2.9% |
| B1 | 15.995 / 16.706 | 16.148 / 17.158 | +1.0% |
| B2 | 0.986 / 1.077 | 0.993 / 1.251 | +0.7% |
| B3 | 17.103 / 17.569 | 17.815 / 18.550 | +4.2% |
| B4 | 6.816 / 7.057 | 7.200 / 7.372 | +5.6% |
| B5 | 4.797 / 5.265 | 4.815 / 5.341 | +0.4% |
| B6 | 0.078 / 0.091 | 0.078 / 0.095 | +0.1% |
| B7 | 13.384 / 14.042 | 14.202 / 14.857 | +6.1% |
| B8 | 21.824 / 22.466 | 21.480 / 22.573 | −1.6% |
| B9 | 4.466 / 5.920 | 4.638 / 6.669 | +3.8% |
| B10 (2 workers) | 2.757 / 2.904 | 1.214 / 1.358 | −56.0% |

The 1-worker B10 profile measured 0.441 ms baseline and 0.465 ms candidate end-to-end (+5.4%); it did not contend and therefore did not activate local reuse. The 2-worker profile measured 2.732 ms baseline and 1.215 ms candidate (−55.5%) end-to-end; its allocation/drop churn phase improved from 2.532 ms to 1.027 ms (−59.4%).

Separate allocator-telemetry runs showed 8,000 allocation/release lock acquisitions for 1-worker B10 both before and after. For 2 workers, acquisitions fell from 16,000 to 4 and cursor fallbacks from 7,891 to 2. Cage high-water cursor stayed at 128,024 bytes for 1 worker and 256,040 bytes for 2 workers.

An all-16 measure-mode comparison showed identical cage high-water cursors for baseline and candidate in every scenario. Peak process RSS was one sample per scenario: candidate deltas ranged from −0.64% to +3.77%, with the maximum in B10; this is within the plan's +5% peak-RSS budget. It is not a repeated retained-RSS measurement. The measured high-water cursors, in bytes, were A1 800,024; A2 900,024; A3 13,368,824; A4 98,344; A5 720,968; A6 528,024; B1 3,277,000; B2 46,720; B3 11,903,944; B4 1,076,000; B5 3,440,680; B6 65,576; B7 15,186,960; B8 4,424,784; B9 3,840,080; and B10 256,040.

Repeated ordering showed some variability in B9. A2 was +4.2% in the fixed-order all-16 run, +3.3% in an isolated native-first run, and +3.0% in the alternate-order all-16 run. B9 was +3.8% in the fixed-order all-16 run and +9.8% in the alternate-order all-16 run, while an isolated native-first run was within 0.3%. These differences are not attributed conclusively; treat non-B10 deltas as directional. The report includes p95 only; 101 repetitions are insufficient for useful p99 or p99.9 estimates.

## Deferred gates and risks

- `CageState` still owns the shared allocator mutex and global cursor/free structures. Ordinary allocation still uses that path when no exact local extent is available; no cage-region reservation or return manager was added.
- The chunk/region work remains gated on checked offset provenance, fresh generation identity, ABA protection, remote registry pin/reap ordering, complete concurrent byte accounting, and failure/quarantine reporting. The executable hypothetical chunk model is sequential and does not discharge these proofs.
- The in-cage retry queue is bounded by finite cage storage rather than a separate descriptor limit. If allocator metadata is invalid, the cage faults closed and retains known descriptors; recovery and an explicit production quarantine-byte report remain unimplemented.
- The maintenance worker was not implemented or benchmarked in disabled, idle, active, or unscheduled states. The maintenance plan makes it dependent on synchronous region-manager success and optional if benefit is small.
- This evidence covers one two-vCPU AArch64 host. No 4/8-worker oversubscription, sanitizer run, stable idle-RSS series, or p99/p99.9 profile was collected.
