# V2.5 Whole-Framework Profiling Baseline

## Summary

The clean capture covers all A1–A6 and B1–B10 scenarios with matching native/compact checksums. It includes two full accounting-free suites, nine noisy paired runs for A2/A4/A5/B3/B5/B6/B8/B10, CPU profiles, and three-repeat hardware-counter captures for all sixteen scenarios.

Compact's clean median is faster than native in six scenarios (A1, A3, B3, B5, B7, B9) and slower in ten. The strongest remaining compact gaps are B10 (5.03x), A4 (3.28x), B8 (2.09x), B6 (2.04x), and A5 (2.60x). The B6 and B9 clean ratios are sensitive to the small two-run suite: V2.4's nine-run B6 sentinel and V2.5's nine-run B6 subset are both about 1.9x, while B9's compact p95 is a high outlier. Treat cross-round changes as follow-up targets, not causal source regressions.

CPU attribution found deque churn in A4, hash-table insertion in A5/B5/B8, JSON/TOML parsing in B1/B2, path checks in A6, and allocator atomics in compact B3/B10. Task-scoped perf still produced no B10/native symbol report. A 99 Hz PID-filtered bpftrace fallback confirms process activity and checksum parity, but its native stacks are mostly unresolved.

## Capture setup

The source commit was `ec68fb78f03b964e4c4c4ea8ba2146f1eee0c0e6`. At capture time, `scripts/profile_cpu.sh` had a local all-sixteen profiling change; the scenario sources were not changed. The host manifest is in the sibling `v25-baseline-20261009` run directory and records the same build target root and artifact hashes used by this capture.

The host was Linux AArch64, kernel `6.17.0-1020-oracle`, two Neoverse-N1 vCPUs. The toolchain was rustc 1.95.0 / LLVM 22.1.2, Cargo 1.95.0, perf 6.17.13, on `stable-aarch64-unknown-linux-gnu`. Builds used `--no-default-features --features json,toml`; allocator telemetry was disabled.

The plain release timing/checksum artifacts were:

- `benchmark_profile-574d1082e0027351`, SHA-256 `6a5893ec6fe934e24c54c5b2a26b779c6bcc06912fe49c7a921f273218392c1b`
- `benchmark_compare-8d33004f27ec8ddd`, SHA-256 `d4dabf6531cce9b3eeaaa0bd45d77dcb26a11a470f43ce0218d2ced1d6352fd0`
- Frame-pointer sampling binary `benchmark_profile-edde4fbd9f071a9f`, SHA-256 `a1866975b57cd3583c6b9e58f1f74645d6b3cdb5da922d1a79e906ad06551c8d`

The accounting-free `benchmark_profile` binary uses `System`, installs no counting allocator, and takes no allocator snapshots during workload phases. The post-window compact integrity check is excluded from `elapsed_ns`, but can appear in whole-process perf samples and counters. CPU sampling used `cpu-clock:u` at 499 Hz with frame-pointer call chains. Profile repetitions were A1/A3/A6=15, A2/A4=5,000, A5=3,000, B1/B2/B4=9, B3=250, B5=3,000, B6=60,000, B7/B9=7, B8=600, and B10=1,000.

The clean timing summary is based on two complete runs with alternating variant order. Times are milliseconds per scenario repetition. The nearest-rank p95 with two observations is the maximum, so it describes the observed pair rather than a stable tail estimate. The noisy subset has nine paired observations and its p95 is likewise the maximum of nine.

## Clean full-suite timings

`C/N` is the ratio of compact to native medians. V2.4 ratios are from the Round 4 two-suite results in `PROFILE_V2_4_ROUND4.md`.

| Scenario | Native median / p95 (ms) | Compact median / p95 (ms) | C/N V2.5 | C/N V2.4 |
| --- | ---: | ---: | ---: | ---: |
| A1 | 0.112 / 0.113 | 0.106 / 0.106 | 0.95x | 0.85x |
| A2 | 1.429 / 1.467 | 2.153 / 2.336 | 1.51x | 1.49x |
| A3 | 5.326 / 5.852 | 1.229 / 1.456 | 0.23x | 0.22x |
| A4 | 0.427 / 0.450 | 1.397 / 1.406 | 3.28x | 3.26x |
| A5 | 1.328 / 1.349 | 3.457 / 3.457 | 2.60x | 2.63x |
| A6 | 1.286 / 1.335 | 1.539 / 1.611 | 1.20x | 1.17x |
| B1 | 14.769 / 15.101 | 18.928 / 19.491 | 1.28x | 1.27x |
| B2 | 0.960 / 0.972 | 1.190 / 1.209 | 1.24x | 1.27x |
| B3 | 32.695 / 33.850 | 23.408 / 23.624 | 0.72x | 0.71x |
| B4 | 6.279 / 6.361 | 9.892 / 10.254 | 1.58x | 1.50x |
| B5 | 10.074 / 10.194 | 7.882 / 8.166 | 0.78x | 0.75x |
| B6 | 0.057 / 0.058 | 0.117 / 0.118 | 2.04x | 0.91x |
| B7 | 23.414 / 23.553 | 20.887 / 21.110 | 0.89x | 0.90x |
| B8 | 13.730 / 14.049 | 28.646 / 28.724 | 2.09x | 2.12x |
| B9 | 12.334 / 12.630 | 11.906 / 15.966 | 0.97x | 0.72x |
| B10 | 0.618 / 0.712 | 3.107 / 3.168 | 5.03x | 7.11x |

B6 changed from a compact win in Round 4 to a 2.04x compact/native ratio here: the compact median moved from 0.122 ms to 0.117 ms while the native median moved from 0.134 ms to 0.057 ms. B9's ratio moved from 0.72x to 0.97x, with compact p95 at 15.966 ms. B10's ratio narrowed from 7.11x to 5.03x because the compact median fell 6.8% while native rose 31.7%. The nine-run noisy subset supports the direction of the B6 and B10 gaps but also shows substantial native B10 tail variation. Treat these as profiling targets, not causal regressions.

## Noisy paired subset

| Scenario | Native median / p95 (ms) | Compact median / p95 (ms) | C/N |
| --- | ---: | ---: | ---: |
| A2 | 1.332 / 1.382 | 1.997 / 2.721 | 1.50x |
| A4 | 0.430 / 0.596 | 1.392 / 1.433 | 3.24x |
| A5 | 1.308 / 1.333 | 3.422 / 3.467 | 2.62x |
| B3 | 31.642 / 32.747 | 23.138 / 23.908 | 0.73x |
| B5 | 10.183 / 10.940 | 7.726 / 8.526 | 0.76x |
| B6 | 0.061 / 0.069 | 0.116 / 0.166 | 1.88x |
| B8 | 13.384 / 13.947 | 28.553 / 29.391 | 2.13x |
| B10 | 0.542 / 1.074 | 2.564 / 3.293 | 4.73x |

All nine noisy runs retained stable per-variant checksums, and native/compact checksums matched for each scenario.

## CPU samples and top self symbols

The table reports each task-scoped perf profile's leading self symbol from the `--no-children` report. The percentage is self overhead. The 31 symbol reports all recorded zero lost samples. Symbols shown as addresses were not resolved from this build's sample data.

| Scenario | Native top self | Compact top self |
| --- | --- | --- |
| A1 | 63.98% vector-build closure (1,452 samples) | 41.32% `CageAllocation::extend_from_iter` (1,457) |
| A2 | 16.36% `malloc` (2,476) | 16.26% `CompactBox::get` (4,587) |
| A3 | 25.33% unresolved `0xa1a48` (300) | 25.02% unresolved `0xa1a48` (1,463) |
| A4 | 99.14% deque-churn closure (2,213) | 58.82% `CompactVecDeque::push_back` (3,227) |
| A5 | 29.40% `HashMap::insert` (1,837) | 25.30% `CompactHashSet::insert` (4,783) |
| A6 | 31.89% `Path::_starts_with` (1,295) | 21.65% `Path::_starts_with` (1,464) |
| B1 | 11.41% serde JSON `parse_str` (1,428) | 9.41% serde JSON `parse_str` (1,562) |
| B2 | 5.30% TOML `parse_keyval` (1,471) | 5.42% `CompactString::len` (1,477) |
| B3 | 11.90% `malloc` (2,496) | 11.49% `__aarch64_cas4_acq` (2,194) |
| B4 | 16.04% `malloc` (1,496) | 12.25% `compact_event` (1,502) |
| B5 | 13.70% `cfree` (10,606) | 31.07% `CompactHashMap::insert` (7,851) |
| B6 | 71.58% order-book closure (2,294) | 25.16% order-book closure (2,707) |
| B7 | 10.33% `malloc` (1,462) | 8.04% file-catalog closure (1,505) |
| B8 | 36.19% cache-churn closure (3,205) | 18.99% `CompactHashMap::insert` (7,245) |
| B9 | 13.70% unresolved `0x1406f4` (1,336) | 35.42% `core::str::from_utf8` (1,245) |
| B10 | No task-scoped samples/report | 28.07% `__aarch64_cas4_acq` (2,244) |

The native B10 profile was collected with the existing PID-filtered bpftrace fallback at 99 Hz. It produced 432 weighted samples for checksum `214883317414038028`; 416 leaf samples were raw addresses and only 16 were symbolized. This confirms process activity and checksum parity, but does not support function-level native B10 attribution. Compact B10's top self symbol is the AArch64 acquire CAS instruction, consistent with allocator synchronization being visible in the profile.

## Hardware counters

All five requested events were available after de-duplicating the probe-status rows: cycles, instructions, branches, branch misses, and cache misses. Each native/compact scenario pair was run three times in `perf stat` profile mode with matching checksums. Values below are the reported mean event counts per run, shown as native / compact. Cycles, instructions, and branches are in millions; misses are in thousands. `perf stat` relative run-to-run spread is retained in the source CSVs; the table does not present these as exact deterministic counts.

These are whole-process counters from zero-second profile windows. They include process startup/exit, benchmark metadata, and the post-window compact integrity check. They are diagnostic attribution evidence, not phase-only counts or workload elapsed time.

| Scenario | Cycles (M, N / C) | Instructions (M, N / C) | Branches (M, N / C) | Branch misses (K, N / C) | Cache misses (K, N / C) |
| --- | ---: | ---: | ---: | ---: | ---: |
| A1 | 6.07 / 5.72 | 10.11 / 10.10 | 1.52 / 1.40 | 15.41 / 14.05 | 52.72 / 38.90 |
| A2 | 18,249.77 / 26,910.89 | 57,062.06 / 78,629.36 | 12,062.60 / 19,149.66 | 4,445.70 / 3,213.72 | 121,401.84 / 87,644.48 |
| A3 | 200.21 / 49.09 | 333.89 / 65.26 | 54.26 / 10.73 | 80.47 / 36.28 | 5,367.89 / 1,387.24 |
| A4 | 6,394.49 / 19,201.51 | 10,921.77 / 66,558.53 | 1,218.79 / 14,851.52 | 269.65 / 571.33 | 5,378.30 / 17,213.61 |
| A5 | 10,743.93 / 28,236.59 | 24,101.55 / 64,949.95 | 2,838.76 / 10,478.99 | 21,289.41 / 214,386.99 | 225,494.41 / 283,057.94 |
| A6 | 55.58 / 66.40 | 140.87 / 173.59 | 33.72 / 43.45 | 72.88 / 57.10 | 348.73 / 320.77 |
| B1 | 385.46 / 496.77 | 1,045.37 / 1,192.77 | 229.96 / 288.54 | 540.32 / 1,537.67 | 2,618.22 / 870.46 |
| B2 | 27.47 / 33.30 | 57.14 / 72.89 | 10.63 / 14.94 | 91.18 / 103.90 | 220.33 / 201.07 |
| B3 | 18,810.93 / 12,993.73 | 49,776.14 / 32,791.81 | 10,856.84 / 7,167.77 | 8,904.90 / 3,527.17 | 198,992.35 / 112,212.48 |
| B4 | 168.42 / 251.61 | 452.95 / 593.29 | 96.76 / 134.41 | 435.47 / 267.12 | 1,762.06 / 1,780.63 |
| B5 | 61,224.93 / 46,737.34 | 123,926.57 / 76,179.44 | 25,773.92 / 11,694.44 | 182,025.52 / 211,832.57 | 1,198,133.62 / 842,210.73 |
| B6 | 7,118.55 / 16,247.19 | 20,903.44 / 54,755.55 | 3,844.62 / 12,401.61 | 6,762.57 / 19,882.31 | 117,438.71 / 135,645.57 |
| B7 | 470.08 / 421.69 | 1,148.12 / 958.94 | 245.62 / 221.06 | 465.28 / 459.40 | 5,199.20 / 4,260.59 |
| B8 | 18,407.34 / 42,852.26 | 29,422.98 / 85,665.91 | 4,448.48 / 16,060.58 | 43,648.95 / 185,114.02 | 525,090.02 / 822,083.38 |
| B9 | 226.62 / 166.51 | 620.95 / 411.86 | 145.79 / 103.97 | 74.65 / 102.20 | 1,567.16 / 1,391.04 |
| B10 | 1,584.89 / 12,084.61 | 2,771.50 / 11,765.40 | 544.83 / 2,454.15 | 6,172.62 / 27,138.97 | 14,017.08 / 66,704.20 |

The cycle ratios broadly follow elapsed-time gaps, with compact/native at 7.62x for B10, 3.00x for A4, 2.63x for A5, 2.33x for B8, and 2.28x for B6. B3 and B5 remain compact wins at 0.69x and 0.76x cycles. Branch-miss counts do not directly predict elapsed time: for example A5's compact count is 10.07x native and B10's is 4.40x, while branch-miss rates should be interpreted relative to each workload's branch count.

## B10 worker scaling

The corrected scaling experiment uses `CSL_B10_WORKERS` to set build, churn, and traversal worker counts. Each worker processes 8,000 records. Runs use nine alternating native/compact pairs and measure per-repetition elapsed time; the hardware counters use three `perf stat` repetitions of 1,000 workload repetitions. The environment variable is explicitly passed through sudo in the counter path. Every run records the requested worker count and equal native/compact checksums.

| Workers per phase | Native median / p95 (ms) | Compact median / p95 (ms) | C/N median | Cycles C/N | Compact cage high-water | Peak RSS native / compact (KiB) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.300 / 0.451 | 0.616 / 0.766 | 2.06x | 2.09x | 128,024 B | 2,952 / 3,084 |
| 2 | 0.558 / 0.728 | 2.736 / 3.416 | 4.90x | 6.91x | 256,040 B | 3,332 / 3,348 |
| 4 | 0.948 / 1.799 | 5.375 / 5.768 | 5.67x | 5.67x | 512,072 B | 3,612 / 3,868 |
| 8 | 1.882 / 2.224 | 11.397 / 11.814 | 6.06x | 6.62x | 1,024,136 B | 5,080 / 4,952 |

The experiment ran on a two-vCPU host, so the four- and eight-worker cases are oversubscribed stress points rather than multicore throughput measurements. From one to eight workers per phase, input work grows eightfold; native time grows about 6.3x and compact time about 18.5x. Compact's cycle ratio is highest at two workers (6.91x) and stays between 5.67x and 6.62x at four/eight. The first counter pass was discarded because sudo stripped `CSL_B10_WORKERS`; the corrected pass explicitly preserves it. The knob and correctly sized worker-handle vectors are in source revision `3ea75c7`; the default two-worker shape in the clean full-suite baseline is unchanged.

## Opportunity matrix

Priorities reflect measured compact/native cost and profile evidence. These are hypotheses for isolated experiments, not approved source changes. Every candidate must preserve the current logical checksum and compare the same workload shape. Keep the full suite as the regression gate.

| Priority / scenario | Measured evidence and source mechanism | Candidate and falsifiable measurement | Main caveat |
| --- | --- | --- | --- |
| P1 — B10 allocator synchronization | 5.03x clean median and 7.62x cycles at the default two workers; compact self profile leads with `__aarch64_cas4_acq` (28.07%). `scenarios::concurrent_workers` spawns build/churn/read workers; each churn worker creates and drops 4,000 `CompactBox` records through the shared cage allocator. | Evaluate bounded local reservation or allocation/release batching only after the allocator design gate is closed. Repeat the 1/2/4/8 worker matrix on a multicore host; record paired medians/p95, cycles, lock/futex samples, checksums, RSS and per-thread slack. | Remote frees, publication ordering, thread exit/reaper, panic recovery and accounting remain safety-critical. The current sequential model is not a concurrent proof; cap reserved slack and test memory limits. |
| P1 — A4 deque operations | 3.28x clean median; `CompactVecDeque::push_back` is 58.82% of compact self samples. `scenarios::deque_churn` executes 80,000 push/pop pairs on a fixed 4,096-entry ring without growth. | For callers that naturally batch non-growing operations, compare the existing borrow-scoped `CompactVecDeque::with_view` against per-operation calls. Keep the shared A4 path and a native equally batched path separate; include any reserve/growth in both sides. | The Round 4 probe measured compact per-operation at 1,189,849 ns and compact view with timed reserve at 143,001 ns; pre-reserved compact view was 143,681 ns against a 141,721 ns native batched floor. This is an opt-in call pattern, not a replacement A4 ratio. Preserve wrap, growth, error, ZST and borrow-scope behavior. |
| P1 — A5/B8 hash-table work | A5 is 2.60x and B8 2.09x clean median. Compact self samples lead with `CompactHashSet::insert` in A5 (25.30%) and `CompactHashMap::insert` in B8 (18.99%). A5 mutates a 16K map/set; B8 churns a fixed 8K-entry cache. | Separate hashing/probe, key/value construction, and removal/reinsert costs. Measure build, lookup, mutation and drop independently with identical inputs and counts; use A2/B3/B5 and the full suite as sentinels. | Randomized hashing and collection behavior are compatibility contracts. Avoid adding per-slot metadata or changing retained layouts without a separate compatibility/memory review. |
| P2 — B6 order-book vector updates | 2.04x clean median and 2.28x cycles. Both variants' top symbol is the order-book update closure. `scenarios::order_book` repeatedly indexes bid/ask vectors, mutates levels, then retains and clones snapshots. | Test the existing `CompactVec::as_mut_slice`/batch-access path for each fixed update round against an equivalently batched native slice. The Round 4 probe measured compact per-index updates at 39,040 ns and compact slice updates at 9,440 ns; native per-index and slice were 9,320 and 9,520 ns. Then rerun the unchanged full scenario. | Keep retain, clone, snapshot and update work in the same phase comparison. Respect borrow exclusivity and the frozen four-byte vector layout; do not compare a compact-only batch API against native per-operation calls. |
| P2 — A2 box traversal and allocation | 1.51x clean median; `CompactBox::get` leads compact self at 16.26%, while native `malloc` leads at 16.36%. `scenarios::box_objects` builds and traverses 25,000 boxed records. | Isolate owner/header resolution from allocation and traversal, then test a safe lookup fast path against the current `CompactBox::get`; retain object count, layout, checksum and RSS measurements. | Preserve owner validation, stale-handle checks and pointer provenance. Prior allocator critical-section experiments regressed A2, so no A2 optimization is accepted on B10 gains alone. |
| P2 — B1/B2 deserialization | B1 is 1.28x and B2 1.24x. Self reports show serde JSON `parse_str` (11.41% native) and TOML `parse_keyval` (5.30% native); compact B1 also leads with the JSON parser, while compact B2 shows `CompactString::len` (5.42%). | Break parse/build from traversal and repeated reads; test borrow-aware or lower-copy ingestion only where the ownership contract permits it. Compare checksums, parse errors, allocations, retained bytes and end-to-end time. | Parsing is shared dependency work and may dominate both variants. Preserve serde/TOML semantics, string ownership, and error behavior; don't move work out of only the timed compact path. |
| P2 — B8 value churn and teardown | 2.09x clean median; compact self leads with `CompactHashMap::insert` (18.99%). `mutate_compact_cache` replaces one eighth of an 8K population through repeated payload construction, remove and insert. | Split map probing from `CompactBytes` construction and release cost, then test capacity-preserving replacement or safe batched-release variants. Capture allocator stats, high-water, free-space shape and RSS beside end-to-end timing. | Retain the fixed population and 64-cycle workload. Round 4 did not find a large fragmented-free tail; avoid attributing the whole gap to the allocator without phase evidence. |
| P3 — B4 event-history strings | 1.58x clean median. Compact self leads with `scenarios::compact_event` (12.25%); each churn entry constructs compact component/message/request-id strings before bounded deque insertion. | Measure string construction separately from deque churn and test reuse/precomputation only if it preserves identical bounded history and record ownership. | Prebuilt events may change allocation/lifetime shape. Compare retained/peak memory and run the same event checksum. |
| P3 — A6/B7 path access | A6 is 1.20x; both top self symbols are `Path::_starts_with`. B7 remains a compact win at 0.89x; compact path queries call `CompactPathBuf::to_path_buf()` on sampled records before equality checks. | Separate path construction from queries. Test a borrowed path/`OsStr` comparison for B7 and prefix scanning for A6 against the current APIs using the same path set. | Preserve platform path semantics, including non-UTF-8 paths. A6 and B7 currently win or nearly match on absolute time; avoid trading their advantage for a microbenchmark-only gain. |
| P3 — B9 frozen-catalog reads | Clean median is near native at 0.97x, but compact p95 is 15.966 ms. `core::str::from_utf8` is 35.42% of compact self; native's top address is unresolved. `compact_catalog_record` repeatedly resolves frozen strings and slices. | Profile sequential, random and parallel readers separately; test a borrow-scoped validated view or reduced repeated descriptor/string decoding, then rerun all three checksums and full B9. | Must reject malformed descriptors/UTF-8 as before. No frozen layout changes or unchecked references without a separate safety review; first establish whether p95 is repeatable. |
| Sentinel — A1/A3/B3/B5/B7/B9 | Compact remains faster in A1 (0.95x), A3 (0.23x), B3 (0.72x), B5 (0.78x), B7 (0.89x), and B9 (0.97x). B5's compact top self is `CompactHashMap::insert`; A3's leading addresses and B9 native's top address are unresolved. | Keep these scenarios in every allocator, hash, path and deserialization A/B suite. Report absolute time and checksum, not only ratios. | A3's top samples are unsymbolized and B9's margin/p95 is noisy; retain these as guards rather than assuming their hotspot is understood. |

### Rejected experiments and bounded API result

- A 7-bit hash fingerprint in the existing control byte was rejected. It changed no per-slot bytes and improved B8 lookup scan 4–6%, but A5 end-to-end moved from 3.152 to 3.160 ms and B8 end-to-end improved only 0.5–1.1%. It did not justify format/compatibility risk.
- Precomputing the common alignment-eight request size improved B10 churn median 2.57% but worsened its p95 1.39% and end-to-end 0.72%; it regressed A2 build/end-to-end and A4 p95 and slightly regressed B3/B5. Moving header initialization after unlocking improved B10 but regressed A2 build and end-to-end. Neither allocator change was retained.
- The A4 `with_view` result above demonstrates an opt-in caller batching opportunity. It does not change the ordinary per-operation A4 comparison, and no production collection code was changed in Round 4.
- No thread-local chunk reservation/cache or lock-free reclamation should proceed until the allocator design gate's real publication ordering, reaper/TLS teardown, panic behavior, public accounting and concurrent-model evidence are resolved.

## Artifacts and limitations

Clean suite/noisy data, profiles, reports, counters, and status files are under `/tmp/csl-v25-profile-capture/runs/v25-clean-20261009/`. The matching source/toolchain/artifact manifest is `/tmp/csl-v25-profile-capture/runs/v25-baseline-20261009/host.txt`.

Round 4's B10/native task-scoped capture had the same sampling gap. The bpftrace fallback attributes B10 native activity to the workload process but leaves most leaf frames unresolved. The p95 values from the two clean suites are maxima, and independent V2.4/V2.5 runs cannot establish that observed median changes came from source changes.
