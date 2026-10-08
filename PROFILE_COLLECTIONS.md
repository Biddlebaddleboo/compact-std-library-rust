# Collection profiling report

## Scope and result

This report follows `PLAN_COLLECTION_PROFILING.md` at baseline `fc0a58bf08aa3bffc577ec2da8151d262b6a5f53`. The worktree is isolated on `codex/collection-profiling`. No collection implementation or shared benchmark harness code was changed. The only added code is the standalone diagnostic example and two runner scripts.

The unchanged `benchmark_compare` child is the timing authority. In its 30-run medians, compact collections take 4.3× native A4 end-to-end time, 4.0× native A5 time, and 3.5× native B6 time. The sampled stacks point to repeated cage-header access in the deque and vector paths, and map/set insertion and probing in A5. The focused probes narrow those paths without changing production behavior.

## Reproduction

Host and compiler used:

- Linux `aarch64`, Oracle Linux kernel `6.17.0-1020-oracle`, two Neoverse-N1 CPUs.
- `rustc 1.95.0 (59807616e 2026-04-14)`, LLVM 22.1.2; Cargo 1.95.0.
- `perf 6.17.13`; privileged access used through `sudo -n perf`, with no sysctl change.
- Release profile, features `json,toml`.

Build and run uninstrumented comparisons and focused timings:

```sh
cargo build --release -p compact_std --features json,toml \
  --example benchmark_compare --example collection_profile
PROFILE_OUT=/tmp/csl-collection-profile \
PROFILE_RUNS=30 BENCHMARK_RUNS=30 \
./scripts/profile_collections.sh
```

`profile_collections.sh` runs each scenario in order A4, A5, B6; for each, it runs native then compact, and runs the unchanged harness before the focused probe. Every child gets 30 repetitions in its own process. Output includes raw samples, median, nearest-rank p95, minimum, maximum, checksums, cage allocator phase deltas, and `/usr/bin/time -v` results.

The harness commands are equivalent to:

```sh
target/release/examples/benchmark_compare --child A4 native 30
target/release/examples/benchmark_compare --child A4 compact 30
target/release/examples/benchmark_compare --child A5 native 30
target/release/examples/benchmark_compare --child A5 compact 30
target/release/examples/benchmark_compare --child B6 native 30
target/release/examples/benchmark_compare --child B6 compact 30
```

For counters and sampled call stacks, run separately from the timing pass:

```sh
PROFILE_OUT=/tmp/csl-collection-profile \
PERF_STAT_RUNS=30 PERF_RECORD_RUNS=500 \
./scripts/sample_collections.sh
```

That script runs `perf stat --repeat 5` for each 30-repetition child. For each 500-repetition `perf record --freq 997 --call-graph dwarf,8192` capture, it also runs a matched unprofiled child and saves both `/usr/bin/time -v` records. All raw artifacts from this capture are under `/tmp/csl-collection-profile`; the report records the summarized values below. Do not use profiled wall times as variant timings.

## A4: deque

The unchanged path is `scenarios.rs::deque_churn`: create a `VecDeque<u64>` or `CompactVecDeque<u64>` with capacity 4,096, fill it, then run 80,000 `push_back`/`pop_front` pairs while keeping it full. The first push grows the full ring. It then traverses the final 4,096 values. Native and compact checksums both equal `2758306198405373103`.

| Harness phase | Native median / p95 (min–max), µs | Compact median / p95 (min–max), µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Build | 1.20 / 1.72 (0.76–8.08) | 53.60 / 56.68 (53.48–56.76) | 44.7× |
| Push/pop mutation | 459.76 / 582.80 (452.92–753.97) | 1,925.50 / 1,945.66 (1,917.30–1,954.02) | 4.19× |
| Traverse | 0.80 / 1.00 (0.80–1.16) | 5.04 / 5.08 (4.96–5.16) | 6.30× |
| End-to-end | 461.98 / 585.76 (454.80–757.01) | 1,984.44 / 2,004.50 (1,976.34–2,013.38) | 4.29× |

The standalone probe uses statically selected `VecDeque` or `CompactVecDeque` methods inside its timed loops, without an enum dispatch per operation. It times each directional operation independently and runs equal-count churn at contiguous, wrapped, and full-capacity starting states.

| Focused operation, 80,000 operations or pairs | Native median / p95, µs | Compact median / p95, µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Push back only | 263.80 / 362.72 | 1,028.77 / 1,229.93 | 3.90× |
| Push front only | 276.00 / 715.81 | 1,026.41 / 1,041.37 | 3.72× |
| Pop front only | 311.20 / 354.80 | 1,009.69 / 1,029.33 | 3.24× |
| Pop back only | 204.12 / 212.24 | 1,067.13 / 1,085.45 | 5.23× |
| FIFO, contiguous | 385.68 / 416.36 | 2,046.18 / 2,054.82 | 5.31× |
| FIFO, wrapped | 454.24 / 489.12 | 2,046.50 / 2,174.66 | 4.51× |
| FIFO, starts full and grows | 458.04 / 473.04 | 2,039.46 / 2,049.78 | 4.45× |

The focused initial-growth result closely matches the harness native mutation (458.04 vs 459.76 µs); compact is 2,039.46 vs 1,925.50 µs. The compact focused loop black-boxes each input and accumulates popped values as a checksum, so treat its 5.9% difference as probe overhead and keep the harness value authoritative.

`physical_index_shadow` takes 190.68 µs native and 216.92 µs compact; `metadata_updates_shadow` takes 294.32 and 294.32 µs. `header_resolution_only` takes 727.29 µs compact. These are instrumented lower bounds and must not be subtracted from full operation times. The storage-only write/read microprobe is not a deque comparison: it bypasses the collection API and uses an uninitialized compact cage slice.

The exact compact paths are `CompactVecDeque::{push_back,push_front,pop_front,pop_back}`, `physical_index`, `reserve`, and `move_ring` in [deque.rs](crates/compact_collections/src/deque.rs). `push_back` and `pop_front` each resolve writable cage capacity for slot access; only the first push needs to reserve and move the ring. The compact sampled profile has 1,039 samples and no lost samples. Its most visible costs are `CageAllocation::uninit_capacity_mut` and `read_header`, reached from repeated `CompactVecDeque::push_back` calls. Native has 197 samples, overwhelmingly in the inlined `deque_churn` closure; the standard-library methods do not remain as distinct sampled symbols.

## A5: hash map and set

The shared path is `scenarios.rs::hash_churn`: build a 16,000-key `HashMap<u32,u64>` plus `HashSet<u32>`, run 4,000 churn steps (572 keys satisfy `key % 7 == 0`), query every thirteenth key in `0..36,000`, then scan the map. Checksums match at `17351022467741104803`.

| Harness phase | Native median / p95 (min–max), µs | Compact median / p95 (min–max), µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Build map and set | 713.97 / 753.77 (696.33–886.97) | 2,808.40 / 2,935.66 (2,790.30–3,011.58) | 3.93× |
| Mutation | 306.92 / 330.04 (292.60–332.40) | 1,168.47 / 1,210.69 (1,158.17–1,557.65) | 3.81× |
| Lookup and scan | 147.68 / 160.36 (145.32–171.28) | 515.80 / 528.84 (507.68–675.97) | 3.49× |
| End-to-end | 1,166.27 / 1,223.25 (1,143.17–1,369.01) | 4,676.60 / 4,890.48 (4,663.32–5,094.68) | 4.01× |

The default hash-only probe hashes 16,000 equal keys per sample. These timings isolate builder and byte-hashing cost from table probing:

| Key/hash operation | Native median / p95, µs | Compact median / p95, µs | Compact/native median |
| --- | ---: | ---: | ---: |
| `u32`, default builder | 170.96 / 185.40 | 195.68 / 215.76 | 1.14× |
| Short string, default builder | 255.24 / 266.44 | 349.16 / 424.88 | 1.37× |
| Long string, default builder | 504.80 / 597.73 | 709.61 / 727.37 | 1.41× |
| `u32`, FNV-1a diagnostic builder | 64.28 / 67.52 | 64.28 / 80.36 | 1.00× |
| Short string, FNV-1a diagnostic builder | 113.12 / 123.76 | 113.04 / 119.28 | 1.00× |
| Long string, FNV-1a diagnostic builder | 1,415.53 / 1,510.85 | 1,418.17 / 1,624.93 | 1.00× |

FNV is a deterministic diagnostic builder, not a recommendation. With that same builder and key stream on both sides, the map phase results are:

| FNV map phase | Native median / p95, µs | Compact median / p95, µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Build | 168.44 / 279.16 | 1,200.97 / 1,431.01 | 7.13× |
| Hit lookup | 96.20 / 104.00 | 850.37 / 1,022.45 | 8.84× |
| Churn | 53.08 / 60.12 | 382.56 / 410.16 | 7.21× |

Hit checksums match at `2175864000`; both churn maps end at 19,428 keys. This separates the larger table-operation gap from the default-builder difference.

The same-process compact probe model shares the collection's `CompactBuildHasher` state and replays the actual A5 key stream. It validates final capacity, length, and key sets against the real compact map/set. It reports:

| Compact probe phase | Operations | Control groups | Full-key candidates | Tombstones seen | Tombstone slots after phase |
| --- | ---: | ---: | ---: | ---: | ---: |
| Map build | 16,000 | 16,008 | 7,763 | 0 | 0 |
| Set build | 16,000 | 16,004 | 7,839 | 0 | 0 |
| Map churn | 8,572 | 8,622 | 12,737 | 135 | 479 |
| Set churn | 4,572 | 4,631 | 8,364 | 85 | 504 |
| Map lookups | 2,770 | 2,806 | 6,203 | 149 | 479 |
| Set lookups | 2,770 | 2,790 | 5,827 | 133 | 504 |

Capacity is 32,768 slots. Final map load is 19,428 keys; map tombstones are 479/32,768 (1.46%). The model sees one initial allocation/rehash for each table and no churn rehash or moved-group visits. For constant-hash collisions on 512 keys, native capacity is 896 and compact capacity is 1,024; both make 131,328 full equality comparisons, while compact visits 8,448 16-byte control groups. This case is deliberately adversarial.

The sampled A5 profiles captured 662 native and 2,551 compact samples, with no lost samples. Native samples center on `hashbrown::HashMap::insert` and its hash operations. Compact samples center on `CompactHashMap::insert` and `CompactHashSet::insert` (about 29% each in the inclusive stack report), with `find_slot_in` around 9–10% below each and `classify_control_group` around 2.7–2.9%. Cage `as_slice`/`as_mut_slice` and `read_header` are visible below these probes. The replay predicts little probing and no churn rehash, so the sampled result points to repeated insertion/table access rather than growth as the measured cost.

## B6: order book

The shared path is `scenarios.rs::order_book`: build two 1,024-level sides, run eight rounds of 192 bid and 192 ask indexed updates, retain nonzero quantities on both sides each round, and copy-rebuild both sides on four odd rounds. Then read best price/depth/flags, copy a snapshot, and drop. Checksums match at `14879364451954781671`.

| Harness phase | Native median / p95 (min–max), µs | Compact median / p95 (min–max), µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Build | 1.80 / 2.08 (1.72–8.08) | 48.50 / 60.56 (48.40–62.88) | 26.9× |
| Indexed updates + retain + rebuild | 34.14 / 49.40 (34.04–102.32) | 87.70 / 117.40 (87.52–125.48) | 2.57× |
| Best-price/depth/flags read | 1.36 / 1.44 (1.32–1.48) | 1.36 / 1.44 (1.28–1.64) | 1.00× |
| Snapshot copy | 2.60 / 2.72 (2.56–6.80) | 2.80 / 3.00 (2.76–13.92) | 1.08× |
| Drop | 0.20 / 0.28 (0.20–4.00) | 0.22 / 0.24 (0.16–0.28) | 1.10× |
| End-to-end | 40.10 / 66.08 (39.92–108.52) | 140.58 / 171.52 (140.24–178.48) | 3.51× |

The focused probe times concrete indexed mutation loops, retains, and snapshot rebuilds separately; its compact branch directly indexes `CompactVec` rather than turning it into a slice before the timer. Subphase and combined-mutation results are:

| Mutation subphase | Native median / p95, µs | Compact median / p95, µs | Compact/native median |
| --- | ---: | ---: | ---: |
| Indexed quote updates | 11.36 / 13.16 | 61.84 / 79.04 | 5.44× |
| Two retains per round | 18.28 / 192.24 | 21.60 / 22.64 | 1.18× |
| Snapshot rebuilds | 4.64 / 14.32 | 5.00 / 5.28 | 1.08× |
| Combined focused mutation | 34.96 / 265.68 | 89.20 / 106.68 | 2.55× |

The median gaps between combined mutation and the sum of its three separately timed subphases are 0.80 µs native and 0.80 µs compact (about 2.2% and 0.9% of combined time). Compared with the unchanged harness mutation median, the focused combined timer is +2.4% native and +1.7% compact. This is the observed timer/instrumentation gap. Subphase medians show indexed compact updates as the largest difference; `CompactVec`'s `IndexMut` calls `as_mut_slice`, and the sampled compact profile repeatedly sees `read_header` on this path. Other relevant functions are `CompactVec::{push,retain,try_clone_copy}` in [vec.rs](crates/compact_collections/src/vec.rs).

Compact B6 retains 49,184 cage bytes after build, 42,848 after mutation, and the same 42,848 after snapshot copy. Drop returns live cage use to zero. The compact callgraph has 80 samples and no lost samples; `read_header` is 34.4% self, `CompactVec::push` is 30.5% inclusive, `retain` 10.6% self, and `try_clone_copy` 2.6% inclusive. Native has only 34 samples, too few to split its already-small phases reliably; much of its work is inlined into the scenario closure. Some compact clone-copy addresses are unresolved by `perf report`.

## Cage bytes and process RSS

`CompactRuntime::used_bytes()` and the harness cage live deltas count live compact-cage allocation bytes, not process memory. The table below combines the harness's per-phase cage deltas with independent process RSS readings. The values are not interchangeable.

| Scenario | Compact cage after build | After main mutation | After drop | Native/compact `/usr/bin/time -v` peak RSS, KiB | Native/compact harness `/proc` HWM, KiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| A4 | 32,768 | 65,536 | 0 | 2,660 / 2,788 | 2,728 / 2,896 |
| A5 | 720,960 | 720,960 | 0 | 3,172 / 3,428 | 3,356 / 3,504 |
| B6 | 49,184 | 42,848 | 0 | 2,660 / 2,788 | 2,808 / 2,872 |

For A4, the standalone push-only phase grows an 80,000-element compact queue and reaches 640,016 live cage bytes; all diagnostic allocations are released by process end. For B6 the standalone probe reports 42,848 live bytes after snapshot and zero after drop. The A5 standalone probe runs extra hash/string and collision cases, so its own process HWM is larger than the harness: `/proc` reports 5,464 KiB native and 6,756 KiB compact, with final cage use zero. Do not use the diagnostic process RSS as the A5 scenario footprint.

`/usr/bin/time` peak RSS and `/proc/self/status` HWM differ slightly on this host (up to about 0.18 MiB in these captures). The report preserves both readings rather than treating either as cage usage. The focused probe also emits current/HWM `/proc` RSS: A4 2,564/2,564 KiB native and 2,568/2,568 compact; A5 4,792/5,464 and 5,552/6,756; B6 2,104/2,104 and 2,084/2,084.

## Hardware counters, overhead, and uncertainty

`perf stat` counter means below are per 30-repetition process, averaged over five launches. The `perf record` captures reported zero lost samples. The counter tool's elapsed time has 0.8–3.8% run-to-run variation; the uninstrumented harness above remains the latency result.

| Scenario | Variant | Cycles | Instructions | Branches | Branch misses | Cache misses | `perf stat` elapsed |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A4 | native | 35.68 M | 69.59 M | 7.90 M | 15.75 K | 49.38 K | 13.16 ms |
| A4 | compact | 187.25 M | 616.60 M | 151.17 M | 21.70 K | 153.80 K | 64.58 ms |
| A5 | native | 113.55 M | 252.80 M | 29.98 M | 237.65 K | 2.36 M | 41.53 ms |
| A5 | compact | 435.89 M | 1,113.12 M | 224.51 M | 2.24 M | 2.98 M | 149.49 ms |
| B6 | native | 5.85 M | 12.89 M | 2.36 M | 19.25 K | 81.85 K | 2.80 ms |
| B6 | compact | 15.40 M | 44.12 M | 10.82 M | 25.91 K | 104.37 K | 6.01 ms |

Callgraph capture used 500 harness repetitions at 997 Hz with DWARF stacks. It collected 197/1,039 A4 native/compact samples, 662/2,551 A5 samples, and 34/80 B6 samples, with zero lost samples. A4 and A5 profiles resolve the dominant collection functions. B6 native has too few samples for fine-grained attribution. `perf record` is costly on these short processes: `/usr/bin/time -v` wall time for matched unprofiled → profiled 500-repetition runs was A4 native 0.18 → 0.34 s (+89%), A4 compact 1.00 → 1.23 s (+23%), A5 native 0.61 → 0.84 s (+38%), A5 compact 2.37 → 2.85 s (+20%). B6 was 0.02 → 0.18 s native and 0.07 → 0.23 s compact; process startup and perf recording dominate, so those percentages are not useful. Use sampled stacks only to locate functions, never to rank latency.

The timings are 30 samples, with nearest-rank p95; p95 and max are sensitive to one or two delayed runs. Most compact A4/A5 results are tight, while native A4 and both B6 diagnostic subphase tails are not. Examples include A4 native mutation 452.92–753.97 µs and B6 native focused retain p95 192.24 µs against an 18.28 µs median. These wide tails do not move the median materially, but they limit confidence in tail comparisons. The container has only two CPUs, so the reported percentiles describe this shared host and workload window, not a general machine guarantee.

## Ranked follow-up experiments

1. **A4 repeated writable-header resolution.** This is the largest compact gap and has support from both the focused phase split and the sampled callgraph. Prototype a scoped writable-slice or cached-handle path while preserving wraparound, growth, and safety invariants; compare the exact full-ring churn workload and its native baseline.
2. **A5 table probing and access cost.** Test the existing probe loop separately from hashing and storage access. The trace puts map/set insertion and `find_slot_in` at the center, while the replay reports only about one control group per ordinary operation, no churn rehash, and a low final tombstone rate. Use the matched FNV path to hold hash semantics constant.
3. **B6 indexed compact-vector updates.** A focused update is 5.44× native while retain and rebuild are only 1.18× and 1.08×. Test a safe batched access path for the update loop, then confirm the complete B6 checksum and mutation/end-to-end timing.

These are profiling leads only. This workstream makes no optimization changes.
