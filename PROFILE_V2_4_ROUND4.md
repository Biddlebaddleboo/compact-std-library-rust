# V2.4 Round 4 profiling and validation

## Outcome

No production optimization cleared the Round 4 acceptance bar. The hash fingerprint and two allocator critical-section experiments were measured and rejected. The collection work added correctness coverage and a standalone probe for the existing borrow-scoped APIs. A test-only allocator state model now exercises a hypothetical chunk protocol, while its implementation safety gate remains open.

The baseline source commit was `cd4c0d1ddde6d2277dd9b5d1d3b889e2bea8995c`. The working-tree changes are uncommitted, so there is no integrated commit SHA. No retained owner layout, allocator policy, hash metadata, or public collection API changed.

## Baseline and method

The initial Round 4 timing pass was superseded after finding that profile windows included the B8 fragmentation diagnostic and a per-scenario cage-lock check. The reported runs below use the corrected path: profile mode skips both inside each scenario, then validates compact state once after each timed window. That final check is excluded from elapsed time but can appear in whole-process samples.

The host was Linux AArch64, kernel 6.17.0-1020-oracle, two Neoverse-N1 vCPUs. Toolchain: rustc 1.95.0, LLVM 22.1.2, Cargo 1.95.0, perf 6.17.13. Features were `--no-default-features --features json,toml`; allocator telemetry was disabled in timing and sampling binaries.

Timing used `benchmark_profile`, which installs no counting global allocator and takes no allocator snapshot during a workload window. A compact allocator integrity check runs after each window; it is excluded from `elapsed_ns` but can appear in whole-process samples. The B8 fragmentation diagnostic and per-scenario live-byte check run only in measure mode, outside accounting-free profile windows.

The plain timing binary was `benchmark_profile-574d1082e0027351` (SHA-256 `6a5893ec6fe934e24c54c5b2a26b779c6bcc06912fe49c7a921f273218392c1b`). The checksum binary was `benchmark_compare-8d33004f27ec8ddd` (SHA-256 `d4dabf6531cce9b3eeaaa0bd45d77dcb26a11a470f43ce0218d2ced1d6352fd0`). The frame-pointer sample binary was `benchmark_profile-edde4fbd9f071a9f` (SHA-256 `602f6d0177ff5c8b91c31d2a86712d8872e4190532b7e40656c467b4bf286ca0`), built with `-C debuginfo=1 -C force-frame-pointers=yes`.

The checksum self-check passed all 16 scenarios across measure/profile modes and native/compact variants. Two full timing suites used alternating variant order; eight key scenarios received nine additional paired runs. All timing pairs and repeated checksums matched. Nearest-rank p95 with two samples is the maximum; with nine samples it is also the maximum, so the tails are descriptive rather than a strong percentile estimate.

Full-suite values are milliseconds per scenario repetition; each cell is median/p95 from two suite runs. Compact/native is the median ratio.

| Scenario | Native ms | Compact ms | Compact/native |
| --- | ---: | ---: | ---: |
| A1 | 0.126 / 0.135 | 0.107 / 0.117 | 0.85x |
| A2 | 1.323 / 1.332 | 1.968 / 1.974 | 1.49x |
| A3 | 4.859 / 4.882 | 1.083 / 1.139 | 0.22x |
| A4 | 0.429 / 0.461 | 1.398 / 1.418 | 3.26x |
| A5 | 1.289 / 1.290 | 3.392 / 3.395 | 2.63x |
| A6 | 1.253 / 1.265 | 1.468 / 1.490 | 1.17x |
| B1 | 14.811 / 14.998 | 18.794 / 18.927 | 1.27x |
| B2 | 0.946 / 0.950 | 1.200 / 1.218 | 1.27x |
| B3 | 32.755 / 33.940 | 23.227 / 23.375 | 0.71x |
| B4 | 6.237 / 6.273 | 9.355 / 9.424 | 1.50x |
| B5 | 10.194 / 10.248 | 7.605 / 7.619 | 0.75x |
| B6 | 0.134 / 0.203 | 0.122 / 0.127 | 0.91x |
| B7 | 23.345 / 23.564 | 21.017 / 21.471 | 0.90x |
| B8 | 13.369 / 13.425 | 28.412 / 28.437 | 2.12x |
| B9 | 10.686 / 10.707 | 7.691 / 7.934 | 0.72x |
| B10 | 0.469 / 0.515 | 3.335 / 3.346 | 7.11x |

The repeated key-scenario results are more useful for noisy cases:

| Scenario | Native ms, median/p95 | Compact ms, median/p95 | Compact/native |
| --- | ---: | ---: | ---: |
| A2 | 1.335 / 1.425 | 1.981 / 2.021 | 1.48x |
| A4 | 0.416 / 0.466 | 1.390 / 1.457 | 3.34x |
| A5 | 1.314 / 1.391 | 3.404 / 3.524 | 2.59x |
| B6 | 0.061 / 0.065 | 0.115 / 0.130 | 1.90x |
| B8 | 13.603 / 14.126 | 28.628 / 29.855 | 2.10x |
| B10 | 0.533 / 0.989 | 3.182 / 3.315 | 5.97x |
| B3 sentinel | 32.356 / 32.962 | 23.336 / 23.545 | 0.72x |
| B5 sentinel | 10.266 / 11.250 | 7.699 / 8.260 | 0.75x |

Artifacts for the two full suites, noisy samples, measure statistics, and host metadata are under `/tmp/csl-v24-round4-baseline-final/runs/20261009T003519Z-1931430/`. CPU samples and symbol reports are under `/tmp/csl-v24-round4-baseline-final/runs/20261009T003625Z-1933324/`.

## Allocation, memory, and sampled costs

The B10 workload used two build, two churn, and two traversal threads on this two-vCPU host; no 4/8-worker scaling run was available.

The separate measure-mode run is for allocation and memory statistics only; its timings are not used as production ratios. Compact retained bytes after build are shown with the allocator high-water cursor and measure-mode peak RSS.

| Scenario | Compact live bytes after build | High-water cursor | Peak RSS |
| --- | ---: | ---: | ---: |
| A2 | 900,016 | 900,024 | 3,704 KiB |
| A4 | 32,784 | 32,792 | 2,864 KiB |
| A5 | 720,960 | 720,968 | 3,600 KiB |
| B3 | 11,903,936 | 11,903,944 | 34,888 KiB |
| B5 | 3,440,672 | 3,440,680 | 11,224 KiB |
| B6 | 49,184 | 49,192 | 3,020 KiB |
| B8 | 3,620,248 | 3,620,256 | 35,272 KiB |
| B10 | 256,032 | 256,040 | 3,444 KiB |

The telemetry-only allocator binary reported these diagnostic totals:

Its diagnostic repetitions were 15 for A2, 9 for B8, and 7 for B10.

| Scenario | Lock acquisitions | Free-list visits | Release batches/extents | Pending reuse hits / misses |
| --- | ---: | ---: | ---: | ---: |
| A2 | 406,366 | 0 | 6,256 / 400,016 | 0 / 400,016 |
| B8 | 178,375 | 3,829,227 | 53,576 / 124,594 | 524,340 / 124,594 |
| B10 | 128,092 | 18,891 | 64,016 / 64,016 | 0 / 64,016 |

Telemetry lock-phase timers include instrumentation and are not production lock-latency measurements. B8 cache-cycle snapshots reached a 4,424,784-byte high-water cursor; the largest observed free total was 804,528 bytes in one extent, with at most two free extents. This capture did not show a large fragmented-free-space tail.

CPU profiling used `cpu-clock:u` at 499 Hz. Every perf capture with samples reported zero lost samples. The largest compact exclusive symbols were:

| Scenario | Samples | Top compact self symbol |
| --- | ---: | --- |
| A2 | 4,603 | `CompactBox::get` (15.69%) |
| A4 | 3,220 | `CompactVecDeque::push_back` (59.29%) |
| A5 | 4,784 | `CompactHashMap::insert` (25.52%) |
| B8 | 7,256 | `CompactHashMap::insert` (18.63%) |
| B10 | 2,528 | `__aarch64_cas4_acq` (28.40%) |

Task-scoped perf produced no B10/native samples. The PID-filtered bpftrace fallback sampled all threads in the workload process at 99 Hz and matched checksum `214883317414038028`: 642 native and 865 compact weighted samples. Most leaf addresses could not be symbolized (626/642 native, 859/865 compact), so this confirms workload attribution and activity, not function-level native B10 attribution.

All five hardware events tested for A4/A5/B6 were supported: cycles, instructions, branches, branch misses, and cache misses. The five-run counter captures are in `/tmp/csl-r4-sample-collections-clean/20261009T003913Z-1993975/`. For example, A4 used about 36.2M native versus 122.2M compact cycles, and B6 about 5.5M native versus 10.4M compact cycles; these include process-level post-window validation and are diagnostic counters.

## Workstream results

### Hash probing

A prototype stored a 7-bit fragment of the randomized hash in the existing one-byte control slot. It added no per-slot bytes and preserved the randomized default hasher. Hash-map, hash-control, and collection differential tests passed; A5/B8 checksums matched. A5 end-to-end was 3.152 ms baseline and 3.160 ms candidate. B8 lookup scan improved 4–6%, while B8 end-to-end improved only 0.5–1.1%. Retained memory was unchanged at 720,960 bytes for A5 and 3,620,248 bytes for B8. The whole-scenario gain did not justify the control-byte format change, so the prototype was rejected and no hash source change remains.

### Collection access

The new `collection_batch_access` test checks wrapped reserve, growth, unwind/error behavior, ZSTs, and native Vec/VecDeque references. It passed 9/9 under normal tests and Miri. The workflow now runs it in the Miri job.

The standalone probe kept per-operation and borrowed-batch measurements distinct. For the shared A4 full-ring shape, the probe included `reserve(1)` inside the compact batch timing so both compact paths included growth:

| A4 shared-shape path | Median / p95, ns | Checksum |
| --- | ---: | ---: |
| Native per-operation | 321,483 / 343,003 | 3,199,960,000 |
| Compact per-operation | 1,189,849 / 1,210,609 | 3,199,960,000 |
| Compact view with timed reserve | 143,001 / 147,722 | 3,199,960,000 |

The separate pre-reserved API study at capacity 84,104 measured compact per-operation at 1,198,409 / 1,379,210 ns and compact view at 143,681 / 155,762 ns; its native storage-batch floor was 141,721 / 159,641 ns. Those numbers describe an opt-in caller pattern, not the shared A4 per-operation ratio.

For B6, the probe used the shared 2,048-level input but timed updates only; retain and clone were excluded. Checksums matched across all variants:

| B6 update path | Median / p95, ns |
| --- | ---: |
| Native per-index | 9,320 / 9,960 |
| Native slice | 9,520 / 9,680 |
| Compact per-index | 39,040 / 99,361 |
| Compact slice | 9,440 / 9,720 |

There was no production collection code change. The existing borrow-scoped deque view and `CompactVec::as_mut_slice` remain opt-in patterns; frozen 12-byte deque and four-byte vector layouts are unchanged.

### Allocator concurrency

The test-only allocator model now tracks chunk/allocation identities, live/pending/reserved/free/reclaimable bytes, remote release queues, transfer, thread exit, panic recovery, exhaustion/replenishment, stale IDs, and coalescing. A follow-up review caught and fixed thread exit after an interrupted reclaiming transition, and corrected the cage integration test to allocate from a real free extent. The model and allocator tests passed.

Two safe Phase 2A experiments were rejected. Precomputing the common alignment-eight request length improved the B10 churn median by 2.57%, but worsened its p95 by 1.39% and B10 end-to-end by 0.72%. It regressed A2 build by 3.78%, A2 end-to-end by 2.23%, and raised A4 p95 by about 32%; B3/B5 medians also regressed by 1.53%/0.69%.

Moving header initialization after unlocking improved B10 churn median/p95 by 8.84%/9.35% and B10 end-to-end by 9.87%/8.84%. It regressed A2 build median/p95 by 11.42%/19.58% and A2 end-to-end by 4.26%/9.41%; B3 rose 1.10%. Both candidates passed their focused allocator tests (32 unit + 1 integration by default; 35 + 1 with the C allocator policy feature), but neither met the no-regression acceptance bar. Neither source change was retained.

The layout-precompute binaries had SHA-256 values `2af05c4c5db250014a043194b4389be63e858242bb6911ce14befd533d8a839d` (baseline) and `6716e74539f13fe6c87d641c1f30b93d0058f463c77fdfdd032c8a3ea0f48f20` (candidate). The header-unlock candidate was `fe857940231543b035e3964602bb4730ad6e59af8c901b806fe2311dc082c6e2`, compared with the same baseline binary. Raw TSVs and logs remain under `/tmp/r4-allocator/phase2a-artifacts/`.

The state model is explicitly hypothetical and sequential. It does not establish real publication ordering, TLS destructor/reaper behavior, nested collector reentrancy, production accounting compatibility, or concurrent memory safety. The remaining design questions are:

- Which synchronization primitive and memory ordering publishes remote frees?
- How do thread-local teardown and the reaper handle in-flight operations?
- What happens with nested collectors, destructor reentrancy, or flush errors?
- How do panic, allocation failure, and abort interact with real chunk metadata?
- How can reserved/reclaimable bytes be represented in public statistics without changing frozen layouts?
- What concurrent stress and model-checking evidence is required before reviewing an unsafe implementation?

The six open chunk-design gaps cover remote-publication memory ordering, real TLS/reaper teardown, nested collectors and reentrancy, panic/allocation-failure behavior, public accounting compatibility, and concurrent stress/model-checking evidence. No thread-local chunk reservation/cache or lock-free reclamation change was added.

## Validation and handoff

Passed:

- `cargo fmt --all -- --check` and `git diff --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- All steps in `.github/workflows/miri.yml`, including the new collection batch test
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- Measure/profile checksum self-check across all 16 scenarios

The Miri counts were: compact_core 3; cage_collections 22; deque_view 8; collection_batch_access 9; backend integration 1; backend library 32; V2.4 tests 7.

No production optimization met the overall acceptance criteria. B3/B5 remain faster than native; A4, A5, B8, and especially B10 remain the main measured gaps. Further allocator work should wait for concrete answers to the remaining chunk safety questions and a multicore A/B host. The temporary `PLAN*.md` files remain as handoff material because the unsafe allocator design gate is still open and no final implementation commit was made.
