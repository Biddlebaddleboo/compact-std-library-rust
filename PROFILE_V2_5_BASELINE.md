# V2.5 Targeted Profiling Baseline (b3ca878)

## Summary

This report is pinned to source b3ca8786448c5fa0b7e503e915414170eaa31e31. Its measurements come from /tmp/csl-v25-targeted-baseline/runs/v25-targeted-baseline only. All A1–A6 and B1–B10 checksum self-checks passed; two clean accounting-free suites and nine noisy paired runs for A2/A4/A5/B3/B5/B6/B8/B10 are included.

Compact is faster in six clean scenarios (A1, A3, B3, B5, B7, B9) and slower in ten. Largest clean compact/native gaps are B10 (7.77x), A4 (3.50x), A5 (2.62x), B8 (2.16x), and B6 (2.10x). Compact B9 is a win at 0.73x in this pinned run, with p95 7.985 ms.

All current timing, memory, symbol, counter, and B10 fallback results below are from the b3ca878 bundle. The ec68 column is explicitly historical context from the earlier report, not a controlled comparison. No source attribution is inferred from cross-revision timing differences.

## Pinned capture and method

The host manifest records Linux AArch64, kernel 6.17.0-1020-oracle, two Neoverse-N1 vCPUs, rustc 1.95.0 / LLVM 22.1.2, Cargo 1.95.0, perf 6.17.13, and stable-aarch64-unknown-linux-gnu. Builds used --no-default-features --features json,toml, with allocator telemetry disabled.

The recorded artifacts are:

- Plain benchmark_profile-574d1082e0027351, SHA-256 d9d596aa07bd47ded1707d087dcd8c4f6973697b54fbb6c38a634ccb7863d612
- Checksum benchmark_compare-8d33004f27ec8ddd, SHA-256 5182e93f5afcd9e31d91bb85c460383c3a4b66fc170cc37db04a85c4643937c5
- Frame-pointer sampling benchmark_profile-edde4fbd9f071a9f, SHA-256 f73c24f475ae6638db3a279e68a61330784e825378b7f69d868a9c840e46157e

The plain timing binary uses System, installs no counting allocator, and takes no allocator snapshots during workload phases. Its post-window compact integrity check is excluded from elapsed_ns but can appear in whole-process samples and counters. CPU sampling used cpu-clock:u at 499 Hz with frame-pointer call chains. Profile repetitions were A1/A3/A6=15, A2/A4=5,000, A5=3,000, B1/B2/B4=9, B3=250, B5=3,000, B6=60,000, B7/B9=7, B8=600, and B10=1,000.

Clean timings use two full suites in alternating variant order. Values are milliseconds per scenario repetition. Nearest-rank p95 with two observations is the maximum, so it describes that pair rather than a stable tail. Noisy results use nine pairs; their p95 is also the maximum of nine.

## Clean full-suite timings

C/N is the ratio of compact to native medians. The ec68 ratio column is copied from the prior V2.5 report solely for historical context; all other columns below are from the pinned b3ca878 capture.

| Scenario | Native median / p95 (ms) | Compact median / p95 (ms) | C/N b3ca878 | C/N ec68 (historical) |
| --- | ---: | ---: | ---: | ---: |
| A1 | 0.110 / 0.116 | 0.103 / 0.106 | 0.94x | 0.95x |
| A2 | 1.344 / 1.366 | 2.017 / 2.040 | 1.50x | 1.51x |
| A3 | 4.626 / 4.834 | 1.050 / 1.109 | 0.23x | 0.23x |
| A4 | 0.400 / 0.406 | 1.400 / 1.415 | 3.50x | 3.28x |
| A5 | 1.283 / 1.287 | 3.361 / 3.363 | 2.62x | 2.60x |
| A6 | 1.197 / 1.202 | 1.486 / 1.507 | 1.24x | 1.20x |
| B1 | 14.699 / 14.709 | 18.529 / 18.575 | 1.26x | 1.28x |
| B2 | 0.962 / 0.973 | 1.194 / 1.219 | 1.24x | 1.24x |
| B3 | 32.233 / 33.428 | 23.016 / 23.131 | 0.71x | 0.72x |
| B4 | 6.280 / 6.321 | 9.364 / 9.420 | 1.49x | 1.58x |
| B5 | 10.004 / 10.010 | 7.829 / 7.929 | 0.78x | 0.78x |
| B6 | 0.059 / 0.060 | 0.124 / 0.128 | 2.10x | 2.04x |
| B7 | 23.185 / 23.219 | 21.042 / 21.142 | 0.91x | 0.89x |
| B8 | 13.146 / 13.152 | 28.394 / 28.457 | 2.16x | 2.09x |
| B9 | 10.536 / 10.556 | 7.701 / 7.985 | 0.73x | 0.97x |
| B10 | 0.422 / 0.447 | 3.277 / 3.343 | 7.77x | 5.03x |

Against the historical ec68 ratios, most gaps are similar. B9's ratio moved from 0.97x to 0.73x, with compact median falling from 11.906 to 7.701 ms. B10 moved from 5.03x to 7.77x; native median fell from 0.618 to 0.422 ms while compact rose from 3.107 to 3.277 ms. A4 widened from 3.28x to 3.50x. These are cross-revision comparisons, not controlled source experiments.

## Noisy paired subset

| Scenario | Native median / p95 (ms) | Compact median / p95 (ms) | C/N |
| --- | ---: | ---: | ---: |
| A2 | 1.330 / 1.388 | 1.981 / 2.304 | 1.49x |
| A4 | 0.431 / 0.489 | 1.404 / 1.497 | 3.26x |
| A5 | 1.301 / 1.335 | 3.465 / 3.744 | 2.66x |
| B3 | 31.409 / 31.825 | 23.034 / 23.524 | 0.73x |
| B5 | 10.128 / 10.373 | 7.711 / 8.148 | 0.76x |
| B6 | 0.061 / 0.097 | 0.118 / 0.157 | 1.95x |
| B8 | 13.405 / 13.971 | 28.398 / 29.694 | 2.12x |
| B10 | 0.428 / 1.361 | 3.049 / 3.254 | 7.12x |

The nine noisy runs had stable per-variant checksums, with native and compact matching for every pair. Native B10 remains the noisiest time series: its p95 is over three times its median.

## Measure-mode memory diagnostics

These values come from measure-stats.tsv. Measure-mode timings are diagnostic and are not used as production ratios. Compact live bytes and high-water cursor are from the build phase; peak RSS is the per-process measure-mode maximum.

| Scenario | Compact live bytes after build | Compact high-water cursor | Peak RSS native / compact (KiB) |
| --- | ---: | ---: | ---: |
| A1 | 400,016 | 400,024 | 3,100 / 3,288 |
| A2 | 900,016 | 900,024 | 3,316 / 3,576 |
| A3 | 13,368,816 | 13,368,824 | 21,908 / 22,336 |
| A4 | 32,784 | 32,792 | 2,628 / 2,724 |
| A5 | 720,960 | 720,968 | 3,344 / 3,472 |
| A6 | 528,016 | 528,024 | 3,652 / 3,980 |
| B1 | 2,692,880 | 3,277,000 | 10,416 / 7,368 |
| B2 | 45,424 | 46,720 | 3,576 / 3,652 |
| B3 | 11,903,936 | 11,903,944 | 42,468 / 34,760 |
| B4 | 486,112 | 486,120 | 12,676 / 12,244 |
| B5 | 3,440,672 | 3,440,680 | 13,704 / 11,088 |
| B6 | 49,184 | 49,192 | 2,764 / 2,888 |
| B7 | 15,186,952 | 15,186,960 | 41,464 / 38,620 |
| B8 | 3,620,248 | 3,620,256 | 35,712 / 35,204 |
| B9 | 3,840,072 | 3,840,080 | 17,092 / 19,028 |
| B10 | 256,032 | 256,040 | 3,216 / 3,348 |

B8's 64-cycle allocator diagnostic reached a 5,211,232-byte high-water cursor, 1,590,976 free bytes across at most four extents, and a largest free extent of 1,588,248 bytes. This is allocator state from the separate measure-mode diagnostic, not the accounting-free timing pass.

## CPU samples and top self symbols

The table reports each task-scoped perf profile's leading self symbol from the no-children report. The percentage is self overhead. All 31 available symbol reports recorded zero lost samples. A3's leading addresses and B9 native's leading address were unresolved.

| Scenario | Native top self | Compact top self |
| --- | --- | --- |
| A1 | 63.88% vector-build closure (1,445 samples) | 39.35% CageAllocation extend_from_iter (1,469) |
| A2 | 17.44% malloc (2,494) | 16.20% CompactBox get (4,580) |
| A3 | 23.19% unresolved address 0xa1a50 (332) | 24.34% unresolved address 0xa1a50 (1,438) |
| A4 | 99.26% deque-churn closure (2,028) | 59.42% CompactVecDeque push_back (3,226) |
| A5 | 29.08% HashMap insert (1,840) | 25.36% CompactHashSet insert (4,739) |
| A6 | 28.91% Path starts_with (1,311) | 21.83% Path starts_with (1,452) |
| B1 | 10.30% serde JSON parse_str (1,476) | 9.62% core::str::from_utf8 (1,559) |
| B2 | 5.71% TOML parse_keyval (1,472) | 5.99% CompactString len (1,468) |
| B3 | 10.87% malloc (2,586) | 11.69% AArch64 acquire CAS (2,225) |
| B4 | 14.35% malloc (1,477) | 10.32% compact_event (1,492) |
| B5 | 13.34% cfree (10,646) | 30.29% CompactHashMap insert (7,837) |
| B6 | 73.56% order-book closure (2,402) | 26.09% order-book closure (2,698) |
| B7 | 9.77% malloc (1,494) | 7.44% AArch64 acquire CAS (1,545) |
| B8 | 34.89% cache-churn closure (3,279) | 18.94% CompactHashMap insert (7,407) |
| B9 | 17.34% unresolved address 0x1406f4 (1,205) | 27.33% core::str::from_utf8 (1,182) |
| B10 | No task-scoped sample report | 27.91% AArch64 acquire CAS (2,458) |

B10 native perf produced no task-scoped symbol report. The PID-filtered bpftrace fallback sampled the B10 process at 99 Hz and matched checksum 214883317414038028. Its stack histogram contained 411 weighted samples, with 398 raw-address leaf samples and 13 symbolized leaf samples. This confirms process activity and checksum parity, not function-level native attribution.

## Hardware counters

All five requested events were available: cycles, instructions, branches, branch misses, and cache misses. Each native/compact pair has three perf-stat repetitions; all 32 workload logs contain three stable checksums, and every native/compact checksum pair matches. Values below are the reported mean counts per workload run, shown native / compact. Cycles, instructions, and branches are in millions; misses are in thousands. Relative run-to-run spread is in the source CSVs.

These are whole-process counts from zero-second profile windows. They include process startup/exit, benchmark metadata, and the post-window compact integrity check. They are diagnostic attribution evidence, not phase-only counts or workload elapsed time.

| Scenario | Cycles (M N/C) | Instructions (M N/C) | Branches (M N/C) | Branch misses (K N/C) | Cache misses (K N/C) |
| --- | ---: | ---: | ---: | ---: | ---: |
| A1 | 6.13 / 5.68 | 10.06 / 10.09 | 1.52 / 1.40 | 15.06 / 13.75 | 51.89 / 40.20 |
| A2 | 18,217.25 / 26,902.30 | 57,066.73 / 78,619.32 | 12,064.88 / 19,148.10 | 4,948.64 / 3,573.92 | 120,382.79 / 87,299.78 |
| A3 | 200.48 / 47.78 | 331.20 / 64.96 | 53.46 / 10.64 | 84.05 / 36.82 | 5,374.89 / 1,388.00 |
| A4 | 6,519.91 / 19,344.81 | 10,921.12 / 66,562.75 | 1,218.69 / 14,852.14 | 275.16 / 597.66 | 5,649.81 / 17,464.27 |
| A5 | 10,561.69 / 28,033.14 | 24,104.97 / 64,943.96 | 2,839.27 / 10,478.06 | 21,117.37 / 211,951.77 | 225,589.77 / 282,856.28 |
| A6 | 54.46 / 66.59 | 140.73 / 173.60 | 33.69 / 43.45 | 68.58 / 59.71 | 356.23 / 321.91 |
| B1 | 390.01 / 492.52 | 1,045.30 / 1,192.39 | 229.89 / 288.47 | 790.72 / 1,486.01 | 2,578.21 / 858.54 |
| B2 | 27.01 / 32.70 | 57.04 / 72.78 | 10.61 / 14.92 | 86.46 / 96.67 | 218.15 / 197.57 |
| B3 | 18,923.61 / 13,002.50 | 49,779.55 / 32,797.08 | 10,857.68 / 7,168.60 | 8,134.76 / 4,909.11 | 197,721.43 / 112,674.76 |
| B4 | 169.83 / 251.04 | 452.78 / 593.50 | 96.74 / 134.44 | 450.25 / 251.03 | 1,746.70 / 1,759.42 |
| B5 | 62,374.80 / 46,840.54 | 123,974.83 / 76,183.16 | 25,778.91 / 11,695.01 | 183,958.69 / 212,619.92 | 1,187,026.40 / 837,505.56 |
| B6 | 7,047.08 / 16,252.04 | 20,907.58 / 54,752.60 | 3,845.25 / 12,401.15 | 10,165.56 / 20,014.05 | 118,524.04 / 136,122.18 |
| B7 | 473.88 / 430.88 | 1,147.83 / 959.62 | 245.52 / 221.19 | 469.42 / 465.71 | 5,189.01 / 4,219.57 |
| B8 | 18,472.89 / 42,730.65 | 29,422.52 / 85,655.81 | 4,448.80 / 16,059.09 | 44,226.44 / 184,253.18 | 524,232.49 / 821,098.77 |
| B9 | 223.91 / 167.97 | 621.14 / 411.57 | 145.82 / 103.91 | 74.90 / 99.97 | 1,601.30 / 1,391.04 |
| B10 | 1,564.68 / 11,795.49 | 2,773.92 / 11,567.58 | 547.03 / 2,421.51 | 6,015.27 / 27,128.73 | 13,759.03 / 65,719.32 |

Cycle ratios are 7.54x for B10, 2.97x for A4, 2.65x for A5, 2.31x for B8, and 2.31x for B6. B3 and B5 remain compact wins at 0.69x and 0.75x cycles. Branch-miss counts are not elapsed-time predictions; normalize them by each workload's branch count.

## B10 worker scaling

This pinned b3ca878 bundle contains only the default two-worker B10 workload, confirmed by its worker_threads metadata. It does not include a worker-count sweep; do not attach earlier ec68 worker-scaling results to this source revision.

## Opportunity matrix

Measured evidence and symbols in this section are from b3ca878. Source mechanism descriptions and candidate ideas are carried forward from the ec68 source review and have not been revalidated against b3ca878 source files. Treat them as hypotheses to verify before implementation. Keep the full suite and checksum comparisons as regression gates.

| Priority / scenario | Measured evidence and source mechanism | Candidate and falsifiable measurement | Main caveat |
| --- | --- | --- | --- |
| P1 — B10 allocator synchronization | 7.77x clean median and 7.54x cycles; compact self profile leads with AArch64 acquire CAS (27.91%). B10 metadata shows two build, churn, and traversal workers. | Evaluate bounded local reservation or allocation/release batching only after the allocator design gate is closed. Capture paired worker-count sweeps, cycles, lock/futex samples, checksums, RSS and per-thread slack. | High implementation complexity: remote frees, publication ordering, thread exit/reaper, panic recovery and accounting remain safety-critical. The sequential model is not a concurrent proof; cap reserved slack and test memory limits. |
| P1 — A4 deque operations | 3.50x clean median; CompactVecDeque push_back is 59.42% of compact self samples. The profile points to fixed-ring deque churn. | For callers that naturally batch non-growing operations, compare the existing borrow-scoped with_view against per-operation calls. Keep shared A4 and an equally batched native path separate; include reserve/growth on both sides. | The large with_view gain in the ec68/Round 4 probe was an opt-in call pattern, not the ordinary A4 ratio. Preserve wrap, growth, error, ZST and borrow-scope behavior. |
| P1 — A5/B8 hash-table work | A5 is 2.62x and B8 2.16x clean median. Compact self leads with CompactHashSet insert in A5 (25.36%) and CompactHashMap insert in B8 (18.94%). | Separate hashing/probe, key/value construction and removal/reinsert costs. Measure build, lookup, mutation and drop with identical inputs; keep A2/B3/B5 sentinels. | Preserve randomized hashing and collection semantics. Avoid per-slot metadata/layout changes without a separate compatibility and memory review. |
| P2 — B6 order-book vector updates | 2.10x clean median and 2.31x cycles. Both variants' top self symbol is the order-book update closure. | Compare existing CompactVec mutable-slice/batch access per update round against an equally batched native slice, then rerun the unchanged full workload. | Round 4's slice probe is historical context. Keep retain, clone, snapshot and update work in the same comparison; respect slice borrow rules and frozen layout. |
| P2 — A2 box traversal and allocation | 1.50x clean median; CompactBox get leads compact self at 16.20%, while malloc leads native at 17.44%. | Isolate owner/header resolution from allocation and traversal, then test a safe lookup fast path with fixed object count, layout, checksum and RSS. | Preserve owner validation, stale-handle checks and pointer provenance. Prior allocator critical-section changes regressed A2. |
| P2 — B1/B2 deserialization | B1 is 1.26x and B2 1.24x. Native self leaders are serde JSON parse_str (10.30%) and TOML parse_keyval (5.71%); compact leaders are UTF-8 validation (9.62%) and CompactString len (5.99%). | Break parse/build from traversal/repeated reads and test lower-copy ingestion only where ownership allows. Compare checksums, parse errors, allocations, retained bytes and end-to-end time. | Parser semantics, ownership and error behavior are compatibility contracts; do not move work out of only one timed path. |
| P2 — B8 cache churn / allocator state | Compact B8 is 2.16x; map insertion leads compact self at 18.94%. Measure-mode cache-cycle diagnostics show a 5.21 MB high-water cursor and up to four free extents. | Separate map probes, payload construction and releases; compare any replacement or batching path against the fixed 8K population and 64-cycle workload. | Current diagnostics show fragmentation to measure, not a cause by themselves. Track RSS and retained bytes; the probe is not accounting-free timing. |
| P3 — B4 event-history strings | Compact B4 is 1.49x; compact_event is 10.32% of compact self. Native malloc leads at 14.35%. | Measure string construction separately from deque churn; test reuse only if it preserves the same bounded history and checksum. | Reuse may alter allocation lifetime/retention. Record peak/retained bytes and error behavior. |
| P3 — A6/B7 path access | A6 is 1.24x; both profiles lead with Path starts_with. B7 remains a compact win at 0.91x; compact self leads with CAS at 7.44% while native malloc leads at 9.77%. | Separate path construction from queries. Test borrowed OsStr/path comparisons for B7 and prefix checks for A6 on the same path set. | Preserve platform path semantics, including non-UTF-8 paths; B7 is a fast scenario sentinel. |
| P3 — B9 frozen-catalog reads | Compact B9 is 0.73x and from_utf8 is 27.33% of compact self; native's leading address remains unresolved. | Profile sequential, random and parallel readers separately; test reduced repeated descriptor/string decoding and rerun all three checksums. | Preserve malformed descriptor/UTF-8 rejection; no frozen layout change or unchecked references without safety review. |
| Sentinel — A1/A3/B3/B5/B7/B9 | Compact wins: A1 0.94x, A3 0.23x, B3 0.71x, B5 0.78x, B7 0.91x, B9 0.73x. | Keep these in every allocator, hash, path and parser A/B suite. Report absolute times and checksums, not only ratios. | A3 and B9 native symbols are unresolved; keep their wins as guards without pretending their attribution is complete. |

## Historical ec68 / V2.4 context

This section is not part of the b3ca878 run. The earlier ec68 report recorded a 7-bit hash fingerprint experiment: A5 end-to-end moved from 3.152 to 3.160 ms; B8 lookup scan improved 4–6% but B8 end-to-end improved only 0.5–1.1%, so the experiment was rejected. It also recorded two rejected allocator critical-section changes: alignment-size precomputation helped B10 churn but regressed A2/A4 and sentinels, while moving header initialization after unlock helped B10 but regressed A2.

The ec68/Round 4 A4 batch probe measured compact per-operation at 1,189,849 ns and compact with timed reserve at 143,001 ns; pre-reserved compact view was 143,681 ns versus 141,721 ns native batched. This is an opt-in call pattern, not the b3ca878 shared A4 ratio. The hypothetical allocator model is not a production safety proof; the remote-free/reaper/panic/accounting gate remains separate.

## Artifacts and limitations

All b3ca878 values above were derived from /tmp/csl-v25-targeted-baseline/runs/v25-targeted-baseline/. The report's historical ratio column is from the earlier ec68 report and is labeled separately. B10 profile samples still have no task-scoped native symbol report; the fallback only establishes process activity. Clean p95 is the maximum of two suite runs; noisy p95 is the maximum of nine.
