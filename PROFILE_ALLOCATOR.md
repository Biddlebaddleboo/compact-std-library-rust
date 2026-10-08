# Allocator diagnosis: A2, B8, and B10

## Scope and reproducible setup

This report is based on `fc0a58bf08aa3bffc577ec2da8151d262b6a5f53` and makes no allocator or benchmark-harness changes. Production allocator policy remains the default policy B: pending exact reuse is enabled and global size-class lookups are disabled. The captures cover A2 allocation/build, B8 fixed-population cache churn and release, and B10 concurrent allocation/drop.

The checked-in scripts are:

- [`allocator_profile.sh`](scripts/profiling/allocator_profile.sh) and [`allocator_sample.bt`](scripts/profiling/allocator_sample.bt): 99 Hz user-stack samples from the no-telemetry release binary.
- [`allocator_uprobes.sh`](scripts/profiling/allocator_uprobes.sh): uprobes for allocator lock-call latency, allocation payload/alignment, release-batch length, and release function duration.
- [`allocator_telemetry.sh`](scripts/profiling/allocator_telemetry.sh): existing `allocator-telemetry` counters, phase timers, B8 cache-cycle state, and `VmHWM`.
- [`summarize_allocator_profile.py`](scripts/profiling/summarize_allocator_profile.py): read-only parser for saved bpftrace and telemetry output. It does not run workloads.
- [`allocator_near_full_probe.rs`](crates/compact_std/examples/allocator_near_full_probe.rs) and [`allocator_near_full_probe.sh`](scripts/profiling/allocator_near_full_probe.sh): a one-shot 16 MiB exhaustion/recovery check, separate from `benchmark_compare`.

Each capture script requires `ALLOCATOR_PROFILE_WINDOW=coordinated`; set it only during an agreed shared-host measurement window. The sampler and uprobe scripts need root or equivalent BPF capability on this host. The output defaults to `/tmp/compact-allocator-profile-<UTC timestamp>/`; `ALLOCATOR_PROFILE_OUTPUT_DIR` can pin a path.

The separate release binaries used for A2/B8/B10 were copied from an existing compatible Cargo cache, so the profiler capture window did not run Cargo builds. Their features and SHA-256 values were:

- Production profile/uprobe binary: `default,json,serde,toml`, without telemetry or allocator-policy features; `dfa198479f3fc0201639562a9bb79b1aff886c48f978d5cba68b8b03c0b8bee0`.
- Telemetry binary: `allocator-telemetry,default,json,serde,toml`, without allocator-policy features; `582a4001b209300bb75124336f7a07020c2f4281f496a9f37fe971bd5b05db34`.

Both binaries were built from source identical to the pinned baseline; the main checkout differed at capture time only by plan documents. The later near-full probe was compiled in its own coordinated slot. To reproduce the profiling captures, first build separate binaries as described below, then run these commands from the repository root during an approved window:

```sh
ALLOCATOR_PROFILE_WINDOW=coordinated \
PROFILE_BINARY=/tmp/benchmark_compare-production \
ALLOCATOR_PROFILE_OUTPUT_DIR=/tmp/compact-allocator-profile-window1 \
  bash scripts/profiling/allocator_profile.sh A2
ALLOCATOR_PROFILE_WINDOW=coordinated \
PROFILE_BINARY=/tmp/benchmark_compare-production \
ALLOCATOR_PROFILE_OUTPUT_DIR=/tmp/compact-allocator-profile-window1 \
  bash scripts/profiling/allocator_uprobes.sh A2
ALLOCATOR_PROFILE_WINDOW=coordinated \
TELEMETRY_BINARY=/tmp/benchmark_compare-telemetry \
ALLOCATOR_PROFILE_OUTPUT_DIR=/tmp/compact-allocator-profile-window1 \
  bash scripts/profiling/allocator_telemetry.sh A2
```

Repeat with `B8` and `B10`. The fixed repetition counts are 800/45/500 for stack sampling, 120/12/80 for uprobes, and 15/9/7 for telemetry (A2/B8/B10). The corresponding binary build commands, when needed, are:

```sh
cargo build --release --locked -p compact_std --example benchmark_compare \
  --features json,toml
cp target/release/examples/benchmark_compare /tmp/benchmark_compare-production

cargo build --release --locked -p compact_std --example benchmark_compare \
  --features json,toml,allocator-telemetry
cp target/release/examples/benchmark_compare /tmp/benchmark_compare-telemetry
```

Neither build enables `benchmark-allocator-a`, `benchmark-allocator-b`, or `benchmark-allocator-c`; no policy feature means the production B default. Summarize saved output without rerunning a workload with:

```sh
python3 scripts/profiling/summarize_allocator_profile.py \
  /tmp/compact-allocator-profile-v2.4
```

## Host, tools, and capture provenance

The host was Linux AArch64, kernel `6.17.0-1020-oracle`, with two vCPUs; Rust/Cargo `1.95.0`; bpftrace `0.20.2`. `/usr/bin/perf` 6.17 was present, but `kernel.perf_event_paranoid=4` and missing perf capabilities blocked unprivileged sampling. Root-authorized bpftrace uprobes and user-stack sampling worked. `trace-cmd` was installed; `strace` and `gprof` were present but did not provide the needed uninstrumented Rust allocator CPU stacks. `samply`, Valgrind, and `llvm-profdata` were not found.

The saved environment files record the pinned commit, kernel, toolchain, binary hash, and capture UTC time. The sampler/uprobes/telemetry timestamps span `2026-10-08T01:47:53Z` through `2026-10-08T01:49:27Z`; the isolated near-full run was captured at `2026-10-08T01:59:44Z`. All artifacts are under `/tmp/compact-allocator-profile-v2.4/`:

- For each `A2`, `B8`, and `B10`: `<S>.environment.txt`, `<S>.user-stacks.txt`, and `<S>.profile-run.log` are the production sampler output; `<S>.uprobes-environment.txt`, `<S>.allocator-uprobes.bt`, `<S>.allocator-uprobes.txt`, and `<S>.uprobes-run.log` are the uprobe output; `<S>.telemetry-environment.txt` and `<S>.telemetry.log` are the telemetry output.
- `near-full-probe.log` contains the isolated allocator build/run environment and its checks. The release executable is `/tmp/compact-std-library-rust-allocator-profile/target/release/examples/allocator_near_full_probe`, SHA-256 `880caffeebcc02eb71ca71f6f5745d2cbb4ff265bdb305358ab937e4e72ddab8`; the log SHA-256 is `15aa2c7df3ab21b3da564a1685037b01d5687c9143bbfe82d3ada0ae32cd3371`.
- The production sample commands ran the child as `benchmark_compare --child <scenario> compact <repetitions>` with the no-telemetry binary. Uprobes used the same child and the same no-telemetry binary. Telemetry used the separate telemetry binary.

The bounded near-full command was:

```sh
ALLOCATOR_PROFILE_WINDOW=coordinated \
ALLOCATOR_NEAR_FULL_OUTPUT=/tmp/compact-allocator-profile-v2.4/near-full-probe.log \
  bash scripts/profiling/allocator_near_full_probe.sh
```

That wrapper runs `cargo run --release --locked -p compact_std --example allocator_near_full_probe --no-default-features`. The example reserves one 16 MiB cage and allocates uninitialized `u8` blocks; it does not touch/fault in the full payload. It checks allocator exhaustion and accounting, not resident-memory behavior.

## Production user-stack samples and phase timings

The sampler records user stacks at 99 Hz, with a 32-frame stack depth, and prints/clears aggregates every 500 ms. The weighted sample totals are small; top leaves are useful clues, not a stable percentage profile. A leaf count is exclusive to that sampled instruction; inclusive caller counts overlap and must not be summed.

| Scenario | Repetitions | Weighted samples | Resolved leaf samples | Unresolved leaf samples | Profile child `VmHWM` |
| --- | ---: | ---: | ---: | ---: | ---: |
| A2 | 800 | 200 | 196 (98.0%) | 4 (2.0%) | 4,452 KiB |
| B8 | 45 | 144 | 93 (64.6%) | 51 (35.4%) | 35,204 KiB |
| B10 | 500 | 246 | 0 | 246 (100%) | 3,940 KiB |

Top sampled leaf functions:

| Scenario | Leaf function | Samples | Share |
| --- | --- | ---: | ---: |
| A2 | `cage::read_header` | 62 | 31.0% |
| A2 | `CageAllocation::as_slice` | 36 | 18.0% |
| A2 | `__aarch64_cas4_acq` | 12 | 6.0% |
| A2 | `ReleaseGuard::drop` | 11 | 5.5% |
| A2 | `CageAllocation::push` | 11 | 5.5% |
| A2 | `CompactRuntime::alloc_owned_value` | 11 | 5.5% |
| B8 | `CompactHashMap::find_slot_in` | 17 | 11.8% |
| B8 | `cage::read_header` | 17 | 11.8% |
| B8 | unresolved raw address | 51 | 35.4% |
| B8 | `cage::release_many_locked` | 8 | 5.6% |
| B8 | `SipHasher24::finish` | 7 | 4.9% |
| B8 | `hash_map::classify_control_group` | 7 | 4.9% |
| B10 | unresolved raw address | 246 | 100% |

In B8, `CompactRuntime::with_batched_releases` occurs in 91/144 sampled stacks and `mutate_compact_cache_batch` in 79/144. Those are inclusive frame counts: they show samples inside the cache mutation/release path, but not time exclusively spent in either function. B10's worker-thread stacks were all raw addresses, so no function-level CPU attribution is available for that capture.

The no-telemetry sampled child reported these median/p95 phase times. The no-telemetry historical full-suite medians are included for context; the sampler run is a diagnostic child, not an independent matched-control experiment.

| Scenario | Phase | Sampled median / p95 (ms) | Historical compact median (ms) |
| --- | --- | ---: | ---: |
| A2 | traverse | 0.470 / 0.490 | — |
| A2 | build | 1.472 / 1.504 | 1.469 / 1.478 |
| A2 | drop | 0.570 / 0.590 | — |
| A2 | end-to-end | 2.517 / 2.569 | 2.511 / 2.520 |
| B8 | fixed-population churn | 26.057 / 27.587 | 25.838 / 25.916 |
| B8 | cache lookup and scan | 0.349 / 0.389 | — |
| B8 | build | 1.790 / 1.932 | 1.742 / 1.759 |
| B8 | drop | 2.425 / 2.903 | 2.422 / 2.413 |
| B8 | end-to-end | 30.699 / 32.294 | 30.358 / 30.392 |
| B10 | parallel allocate/drop churn | 2.828 / 3.047 | 2.738 / 2.782 |
| B10 | parallel traversal | 0.088 / 0.181 | — |
| B10 | build | 0.096 / 0.165 | 0.096 / 0.100 |
| B10 | drop | 0.001 / 0.002 | 0.001 / 0.001 |
| B10 | end-to-end | 3.015 / 3.428 | 2.913 / 2.969 |

Checksums matched the benchmark child outputs: A2 `6387817404620238636`, B8 `10141254246637991735`, and B10 `214883317414038028`. The sampled medians are near the earlier no-telemetry runs, but there was no matched same-minute unprofiled control. Treat these as workload timing context, not a measured profiler-overhead correction.

## Uprobe distributions and limitations

The uprobes use `hist((nsecs - start) / 1000)`, so the duration bins are integer microseconds and powers-of-two buckets; they are not exact percentiles. `lock_latency_us` spans entry to return from `cage::lock`, combining mutex acquisition and function/probe overhead. It is not a pure scheduler wait measurement. BPF maps are printed and cleared every 500 ms; the values below sum those intervals.

| Scenario | `lock()` calls | Latency histogram samples | `0 us` | `1 us` | `2–4 us` | `4–8 us` | `8–16 us` | `16–32 us` | `32+ us` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| A2 | 3,024,887 | 3,024,770 | 0 | 1,072,213 | 1,923,135 | 20,246 | 6,921 | 1,651 | 604 |
| B8 | 144,262 | 144,257 | 0 | 33,642 | 108,364 | 1,525 | 533 | 135 | 58 |
| B10 | 648,093 | 648,058 | 375 | 116,302 | 196,364 | 2,097 | 243,823 | 85,558 | 3,539 |

B10's histogram has 51.4% of observations in bins at or above 8 us; A2 has 0.30% and B8 0.50%. This direction is consistent with additional contention in B10, but the uprobe pass adds severe overhead (below), so these are not production lock-latency percentiles.

The `allocate_block` argument maps and release batch lengths were:

| Scenario | `allocate_block` payload sizes (count) | Alignments (count) | `release_many_locked` batch sizes (count) |
| --- | --- | --- | --- |
| A2 | 16 B: 3,024,893; 100,000 B: 121 | 8 B: 3,024,898; 4 B: 121 | 64 extents: 47,190; 41 extents: 121 |
| B8 | 24 B: 36,051; 96 B: 36,052; 512 B: 36,051; 1,024 B: 36,051; 16,384 B: 32; 786,432 B: 32 | 4 B: 144,240; 8 B: 32 | 1: 52,428; 2: 5; 4: 13; 28: 14; 64: 1,428 |
| B10 | 16 B: 647,967; 128,000 B: 162 | 8 B: 648,140 | 1: 648,141 |

These allocation argument counts observe calls to `allocate_block`, which runs after pending exact-reuse misses; they do not count all logical allocation requests when an extent is reused before reaching that function. Small count differences between paired maps can arise from the process exiting between 500 ms print/clear intervals.

The measured `release_many_locked` duration histograms were also highly instrumented. In compact notation (bin: count): A2 `0:46,850; 1:141; 2–4:121; 4–8:145; 8–16:35; 16–32:14; 32–64:3; 64–128:1; 1K–2K:1`; B8 `0:13,061; 1:3,399; 2–4:35,419; 4–8:501; 8–16:427; 16–32:1,026; 32–64:52; 64–128:2; 128–256:1`; B10 `0:92; 1:14,305; 2–4:589,620; 4–8:41,333; 8–16:1,884; 16–32:543; 32–64:170; 64–128:51; 128–256:44; 256–512:20; 512–1K:25; 1K–2K:4`. These bins include probe overhead and cannot be read as production release cost.

The BPF critical-section histogram is invalid. The probe pairs each lock return with a wildcard transaction-drop probe using one timestamp per thread, without a reliable one-to-one lock/transaction pairing. It yielded only 121 observations for A2, 89 for B8, and none for B10, versus millions or hundreds of thousands of lock calls. Do not use it to estimate lock hold time. No `merge_us` samples were emitted, so this capture does not provide a merge-function duration distribution.

The uprobe run itself is strongly intrusive. Its benchmark child medians were A2 build 118.130 ms/end-to-end 119.580 ms, B8 build 34.643 ms/end-to-end 64.826 ms, and B10 parallel churn 77.091 ms/end-to-end 77.367 ms. Compared with the production sampled medians, these are about 80x for A2 build, 19x for B8 build, and 27x for B10 churn. Use the event counts and broad contention direction as diagnostic evidence only; discard uprobe phase timings as production performance measurements.

## Allocator telemetry and B8 cursor contractions

Telemetry runs use the `allocator-telemetry` feature and include setup/warm-up as well as measured work in the process-wide summary. The phase rows are medians from 15/9/7 repetitions. Phase timers include `Instant` and atomic-update costs, and some timers are nested or inclusive; do not add them into an elapsed-time breakdown.

Selected cumulative timer totals from `META\tallocator_phase_profile` (milliseconds):

| Scenario | Pending lookup | Layout | Mutex wait | Free-list search | Bump allocation | Header initialization |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A2 | 13.751 | 13.620 | 16.767 | 13.303 | 44.760 | 12.636 |
| B8 | 181.361 | 63.235 | 7.966 | 11.492 | 8.053 | 20.770 |
| B10 | 4.075 | 2.590 | 35.681 | 3.355 | 8.345 | 2.350 |

The counter totals and phase-level context were:

- **A2:** 406,366 lock acquisitions; 400,016 pending misses, of which 400,000 had no active collector; 400,000 cursor fallbacks; 0 size-class hits. The size histogram recorded 400,000 32-byte blocks and 16 blocks in the 1,024-byte-plus bucket. Instrumented build median was 12.942 ms versus 1.472 ms in the separate no-telemetry sample. Telemetry disables the no-telemetry empty-free-structures cursor fast path, so these timings and timer rankings do not describe production A2.
- **B8:** 178,375 lock acquisitions and 3,839,352 free-list visits; 524,340 pending exact-reuse hits; 124,594 misses, including 124,544 with no active collector and 50 with no exact compatible extent; 0 size-class hits. There were 1,704,460 pending candidates across 524,390 active lookups (3.25 candidates/lookup). Depth histogram: `0:10, 1:10, 2:24, 3:393141, 4:131078, 5:95, 6:32`. Candidate-size histogram: `40:393274, 112:393266, 528:393250, 1024+:524670`. Telemetry's fixed-churn median was 50.967 ms versus 26.057 ms in the separate no-telemetry sample; its end-to-end median was 72.175 ms versus 30.699 ms. These are not production timings.
- **B10:** 128,092 lock acquisitions and 21,561 free-list visits; 64,016 pending misses, all with no active collector; 64,000 32-byte allocations and 16 blocks at 1,024 bytes or above; no size-class hits. Cumulative mutex-wait time was 35.681 ms across process activity, the largest of the listed telemetry timer totals. The parallel churn median was 6.752 ms versus 2.828 ms in the separate no-telemetry sample. The wait total is summed across threads, includes telemetry overhead, and cannot be converted directly to per-operation production wait.

B8 emitted 64 cache-cycle snapshots. The cursor rose to 5,211,232 bytes at cycles 23–33, then contracted to 3,639,392 bytes at cycle 34 and 3,638,336 bytes at cycle 57. Free bytes ranged from 16,552 to 19,136 B through cycles 34–45, and from 16,552 to 18,080 B through cycles 57–64; the largest free block in those ranges was about 17.1 KiB. This directly confirms cursor-tail contraction in the diagnostic replay; the snapshots do not establish its CPU cost in the production churn phase.

The isolated near-full example exercised the public allocation API at a fixed 16 MiB cage size:

| Step | Requested payload | Expected check | Observed |
| --- | ---: | --- | --- |
| Empty cage | — | `used_bytes() = 0 B` | 0 B |
| Near-full allocation | 15,728,640 B (15 MiB) | `used_bytes() = 15,728,656 B` | 15,728,656 B |
| Remaining after near-full allocation | — | `remaining_bytes() = 1,048,560 B` | 1,048,560 B |
| Deliberate allocation failure | 2,097,152 B (2 MiB) | `AllocationExhausted`; live bytes unchanged | `AllocationExhausted`; 15,728,656 B |
| Drop near-full allocation | — | `used_bytes() = 0 B` | 0 B |
| Recovery allocation | 1,048,576 B (1 MiB) | `used_bytes() = 1,048,592 B` | 1,048,592 B |
| Final drop | — | `used_bytes() = 0 B` | 0 B |

Each live-byte check also matched `AllocatorStats.live_bytes`; `validate_allocator_state()` passed after recovery and at exit. This fills 93.75% of the configured cage by requested payload, demonstrates failure without accounting drift, and confirms recovery after dropping the large block. The block is uninitialized, so this bounded probe does not measure page residency or RSS.

## Live bytes, cage high-water cursor, and process RSS

The benchmark validates that compact `used_bytes()` returns zero after each scenario, and it checks that native tracked live bytes are zero after drop. Thus the high-water values below are not retained live allocations. The TSV column `cage_high_water_cursor` is the maximum cursor position observed during a measured phase; it is a peak arena-use measure, not bytes still live after scenario completion.

| Scenario | Compact cage high-water cursor in build phase | Live cage bytes after scenario |
| --- | ---: | ---: |
| A2 | 900,016 B | 0 B |
| B8 | 3,620,248 B | 0 B |
| B10 | 256,032 B | 0 B |

Historical no-telemetry full-suite medians and RSS come from
`/tmp/v24-performance-run1.tsv` and `/tmp/v24-performance-run2.tsv`; earlier
allocator telemetry is in `/tmp/v24-allocator-profile.tsv`. Historical
no-telemetry full-suite peak RSS (`VmHWM`, KiB; run 1 / run 2):

| Scenario | Native RSS | Compact RSS |
| --- | ---: | ---: |
| A2 | 3,340 / 3,340 | 3,516 / 3,512 |
| B8 | 35,656 / 35,644 | 35,144 / 35,144 |
| B10 | 3,156 / 3,156 | 3,288 / 3,272 |

Fresh diagnostic child `VmHWM` (KiB; production sampler / uprobes / telemetry): A2 `4,452 / 3,600 / 3,416`; B8 `35,204 / 35,148 / 35,112`; B10 `3,940 / 3,356 / 3,316`. These captures use different repetition counts and instrumentation. RSS includes runtime, fixtures, allocator metadata, and temporaries; it is not comparable to the cage cursor or live allocation bytes. The small run-to-run movement also means RSS is not a precise proxy for allocator retention.

## Supported and rejected hypotheses

1. **A2 has material time in owner/header access and allocator paths.** The uninstrumented sample places 31% of A2 leaf samples in `read_header`, 18% in `CageAllocation::as_slice`, and additional samples in push, allocation, and release paths. Build median is 1.472 ms, versus 0.470 ms traverse and 0.570 ms drop. This supports inspecting header validation and owner access in a future focused experiment. It does not prove their exclusive CPU share: only 200 weighted samples were collected, and the allocator uprobe/telemetry timings are heavily distorted.
2. **B8 work is split across collection logic and allocator release/reuse.** Symbolized leaves include hash-table slot search, control-group classification, hashing, `read_header`, and `release_many_locked`; inclusive stacks frequently pass through batched releases. Telemetry confirms many pending hits but a shallow average scan of 3.25 candidates and only 50 active-collector lookups with no exact extent. That rejects a hypothesis that long pending scans are the obvious dominant cost. It does not rank map mutation against allocator release because the stack sample is small/partly unresolved and inclusive frames overlap.
3. **B10 contention deserves a lower-overhead follow-up.** Telemetry's aggregate mutex-wait counter and the traced lock histogram both move in the contention direction; under uprobes, 51.4% of lock observations land at 8 us or more, compared with under 1% for A2/B8. B10 stack symbols are entirely unresolved, and the uprobes slow the workload by roughly 25–27x. The result supports contention as a candidate, not an absolute production wait estimate or a ranked CPU cost.
4. **Global size-class reuse is not a measured explanation for these policy-B runs.** Telemetry reported zero size-class hits in A2, B8, and B10. B8 cache reuse is primarily reported through pending exact-reuse counters in these captures.
5. **Peak RSS is not retained cage data.** The workloads finish with zero live cage bytes, while the peak RSS includes process and fixture memory. The high-water cursor and RSS answer different questions and should stay separate in future comparisons.

## Ranked follow-up measurements

These are measurements to run before considering code changes:

1. **B8:** separate hash-table slot search/hash/control classification from allocation, pending reuse, and release work with a lower-overhead per-phase capture. B8 has the largest measured compact phase, while the current sample sees both collection and allocator frames but cannot rank their exclusive costs.
2. **A2:** attribute `read_header`, owner access, cursor allocation, and uncontended lock wrapper cost with a low-overhead profiler or call-count instrumentation. Its sample is fully symbolized and concentrated in header/access paths, but only 200 weighted samples were collected.
3. **B10:** repeat lock-wait and worker-stack attribution with a profiler that preserves thread symbols and has substantially lower overhead than these uprobes. Current counters indicate contention, but the B10 CPU sample is unresolved and uprobe timings are not production values.

## Uncertainty and deviations

The sampling pass produced only 144–246 weighted samples per scenario, with B8 partial and B10 unusable symbolization. Histograms use coarse integer-microsecond bins. Uprobes materially change runtimes and fail to pair critical-section begin/end events; those measurements cannot satisfy a production lock-hold-time distribution. Telemetry timers overlap and the feature changes A2's path. The near-full probe verifies allocator accounting and recovery but does not touch the reserved payload; there was no native CPU profile or same-minute uninstrumented control. No allocator behavior was changed and no optimization is proposed in this profiling pass.
