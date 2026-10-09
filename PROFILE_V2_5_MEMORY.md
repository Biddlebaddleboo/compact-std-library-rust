# V2.5 memory and layout baseline

## Scope and provenance

This records the implementation baseline pinned by the targeted V2.5 plan: `8bfe6f7192c5cdd1258d1cffb7fc1b5dddfeab5e`. The current `b3ca878` commit adds plan files only; allocator, owners, collections, scenarios, and accounting source are unchanged from the pinned baseline. The prior Round 4 accounting capture was built from the dirty `cd4c0d1` worktree whose Round 4 changes were subsequently committed as `9ff38cd`.

The pinned baseline artifacts are under `/tmp/csl-v25-targeted-baseline/runs/v25-targeted-baseline/`. The accounting capture is `measure-stats.tsv`; host/toolchain and binary hashes are recorded in `host.txt`. It contains all 16 scenarios with native/compact checksum parity. The source was `b3ca8786448c5fa0b7e503e915414170eaa31e31`; the build used Rust 1.95.0, LLVM 22.1.2, AArch64 Neoverse-N1 (2 vCPUs), Linux kernel 6.17.0-1020-oracle, `--no-default-features --features json,toml`, and release settings with allocator telemetry disabled. Measure mode enabled native allocation counting and compact allocator snapshots. Its elapsed times are accounting-instrumented and are not timing results.

The owners, allocator, collections, and accounting harness in this capture are unchanged from the pinned `8bfe6f7` baseline. The `b3ca878` plan commit adds no runtime change.

## Retained owner and descriptor layouts

The layouts below are the AArch64 V2.4 values. The test `memory_layout.rs` locks the public owner/configuration and frozen descriptor sizes and alignments. `AllocationHeader` remains private; `cage.rs` has a compile-time size assertion for its four-`u32`, 16-byte representation.

| Type | Size | Alignment | Representation / note |
| --- | ---: | ---: | --- |
| `CageConfig` | 8 B | 8 B | One `usize` on AArch64 |
| `CageAllocation<u64>` | 4 B | 4 B | One non-zero cage offset |
| `Option<CageAllocation<u64>>` | 4 B | 4 B | Uses the non-zero offset niche |
| `CompactBox<u64>` | 4 B | 4 B | One allocation owner |
| `CompactVec<u64>` | 4 B | 4 B | Optional allocation owner |
| `CompactVecDeque<u64>` | 12 B | 4 B | Owner, head, and length |
| `AllocationHeader` | 16 B | 4 B | `#[repr(C)]`, four `u32` fields; size is compile-time asserted |
| `FrozenString`, `FrozenBytes` | 8 B each | 4 B | Two `u32` fields |
| `FrozenOsString`, `FrozenPathBuf` | 8 B each | 4 B | Newtype wrappers around `FrozenBytes` |
| `FrozenVec<T>`, `FrozenMap<K,V>`, `FrozenSet<T>` | 8 B each | 4 B | Two `u32` offset/length words, directly or through a wrapper |

An allocation's 16-byte header and any alignment prefix live in the cage block, outside its four-byte Rust owner. `AllocatorStats` is a copied diagnostic snapshot with a `cfg(feature = "allocator-telemetry")`-dependent set of fields; its Rust layout is not a retained-owner contract and is not frozen by this test.

## Per-scenario baseline

`native build-live` is the median native allocator live-byte delta at the end of the build phase. `compact build-live` is the median cage live-byte delta at that point, including header and padding. The delta is compact minus native and is contextual only: containers may reserve different capacities and use different native allocations. The V2.5 budget compares an integrated compact build against this same compact baseline, not against native memory. `compact cursor high-water` is the largest per-phase cage cursor observed during the scenario. RSS is each child process's Linux `VmHWM` high-water mark in KiB in accounting mode.

| Scenario | Native build-live (B) | Compact build-live (B) | Compact − native (B) | Compact cursor high-water (B) | Peak RSS native / compact (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| A1 | 400,000 | 400,016 | +16 | 800,024 | 3,100 / 3,288 |
| A2 | 600,000 | 900,016 | +300,016 | 900,024 | 3,316 / 3,576 |
| A3 | 13,361,600 | 13,368,816 | +7,216 | 13,368,824 | 21,908 / 22,336 |
| A4 | 32,768 | 32,784 | +16 | 98,344 | 2,628 / 2,724 |
| A5 | 720,912 | 720,960 | +48 | 720,968 | 3,344 / 3,472 |
| A6 | 428,180 | 528,016 | +99,836 | 528,024 | 3,652 / 3,980 |
| B1 | 4,360,722 | 2,692,880 | −1,667,842 | 3,277,000 | 10,416 / 7,368 |
| B2 | 56,629 | 45,424 | −11,205 | 46,720 | 3,576 / 3,652 |
| B3 | 13,071,260 | 11,903,936 | −1,167,324 | 11,903,944 | 42,468 / 34,760 |
| B4 | 565,270 | 486,112 | −79,158 | 1,076,000 | 12,676 / 12,244 |
| B5 | 4,846,280 | 3,440,672 | −1,405,608 | 3,440,680 | 13,704 / 11,088 |
| B6 | 49,169 | 49,184 | +15 | 65,576 | 2,764 / 2,888 |
| B7 | 14,459,152 | 15,186,952 | +727,800 | 15,186,960 | 41,464 / 38,620 |
| B8 | 3,528,488 | 3,620,248 | +91,760 | 4,424,784 | 35,712 / 35,204 |
| B9 | 5,193,776 | 3,840,072 | −1,353,704 | 3,840,080 | 17,092 / 19,028 |
| B10 | 256,048 | 256,032 | −16 | 256,040 | 3,216 / 3,348 |

All compact scenarios returned to zero live cage bytes at completion. The per-phase release snapshots showed zero post-drop reusable bytes in these runs; B8 is the exception during its deliberate cache-churn cycles and is summarized below. Scenario repetitions were the harness defaults: 15 for A1–A6, 9 for B1–B4/B6/B8, and 7 for B5/B7/B9/B10. Default B10 uses two build, two churn, and two traversal workers. A separate B10 weak-scaling capture used 1, 2, 4, and 8 workers with 8,000 records per worker; this shared host has only two vCPUs, so the 4/8-worker results show oversubscription rather than physical multicore scaling.

| B10 workers | Native build-live (B) | Compact build-live (B) | Compact cursor high-water (B) | Peak RSS native / compact (KiB) |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 128,048 | 128,016 | 128,024 | 2,952 / 3,084 |
| 2 | 256,048 | 256,032 | 256,040 | 3,332 / 3,348 |
| 4 | 512,096 | 512,064 | 512,072 | 3,612 / 3,868 |
| 8 | 1,024,192 | 1,024,128 | 1,024,136 | 5,080 / 4,952 |

The retained-byte and cursor totals grow with the fixed per-worker data set. These runs do not measure current RSS after thread exit or prove reclamation on a multicore host.

## Fragmentation and allocator slack

The B8 per-phase high-water is 4,424,784 B. Its separate 64-row cache-cycle trace in the pinned bundle reached a 5,211,232 B cursor, 1,590,976 B total free, up to four free blocks, and a 1,588,248 B largest extent. The prior Round 4 trace reached the same cursor/free high-water with a 1,588,240 B largest extent. The earlier Round 4 narrative treated the per-phase high-water as the cache-cycle maximum; the raw trace is authoritative and that summary is corrected in `PROFILE_V2_4_ROUND4.md`. The higher diagnostic high-water is a distinct cache-churn path and does not replace the per-phase value in the table.

The production allocator's small size-class cache is bounded: four exact block sizes `[32, 40, 112, 528]`, up to 32 cached extents per class. Their maximum combined cage footprint is 22,784 bytes. The release collector contains 64 eight-byte extents (512 bytes) plus its length field and is stack-local to an active batch; TLS stores only a pointer to that stack object. The current code therefore has no unbounded per-thread allocation cache. This is a source-derived bound, not a measured per-thread RSS delta. B10 on this two-vCPU host does not establish 4/8-worker scaling or thread-exit RSS reclamation.

## Measurement gaps and next capture

The old harness records requested cage capacity (128 MiB in `benchmark_compare`), live bytes, cursor, free extents, and process `VmHWM`. It does not record `/proc/<pid>/status` `VmSize`, committed pages, or current `VmRSS` after teardown/quiescence. The 128 MiB setting is a cage capacity request, not a resident-memory measurement. The current artifacts therefore cannot establish virtual reservation, page commitment, or idle RSS relative to V2.4.

The capture still cannot establish virtual reservation, page commitment, or current idle RSS relative to V2.4. For an integrated runtime change, add separate non-timing snapshots for current RSS after the workload has dropped owners and the process has quiesced, virtual mappings, and worker-thread exit. Keep timing binaries free of allocator telemetry. Record absolute-byte deltas alongside ratios; for scenarios with very small baseline RSS, compare byte deltas and repeatability rather than interpreting a rounded percentage as a meaningful alarm.

The accepted owner-header fast path changes only validation on private,
allocator-issued owners. Owner sizes, allocation headers, collection layouts,
public APIs, serialized descriptors, and allocator accounting are unchanged.
The all-16 candidate accounting capture found identical compact live bytes,
peak requested/cage bytes, and zero retained bytes after each scenario; the
small RSS differences are process-level noise. No V2.5 memory-budget exception
is requested.
