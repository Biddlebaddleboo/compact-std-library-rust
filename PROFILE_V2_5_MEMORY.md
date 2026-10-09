# V2.5 memory and layout baseline

## Scope and provenance

This records the V2.4 runtime as the pinned memory baseline for V2.5. `ec68fb7` adds plan documents after `9ff38cd`; the allocator, owners, collections, scenarios, and accounting harness have no source changes since that V2.4 commit. The prior Round 4 accounting capture was built from the dirty `cd4c0d1` worktree whose Round 4 changes were subsequently committed as `9ff38cd`.

The measurement artifact is `/tmp/csl-v24-round4-baseline-final/runs/20261009T003519Z-1931430/measure-stats.tsv`, with host/toolchain metadata in the adjacent `host.txt`. It contains all 16 scenarios, with native/compact checksums matched. The build used Rust 1.95.0, LLVM 22.1.2, AArch64 Neoverse-N1 (2 vCPUs), Linux kernel 6.17.0-1020-oracle, `--no-default-features --features json,toml`, and the release profile with allocator telemetry disabled. Measure mode enabled native allocation counting and compact allocator snapshots. Its elapsed times are accounting-instrumented and are not timing results.

No new memory run was started in this workstream because the orchestrator was capturing a CPU baseline on the shared 2-vCPU host. The unchanged runtime source makes the existing capture the current V2.5 starting point; it is not a fresh capture from `ec68fb7`.

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
| A1 | 400,000 | 400,016 | +16 | 800,024 | 3,296 / 3,480 |
| A2 | 600,000 | 900,016 | +300,016 | 900,024 | 3,328 / 3,704 |
| A3 | 13,361,600 | 13,368,816 | +7,216 | 13,368,824 | 22,156 / 22,460 |
| A4 | 32,768 | 32,784 | +16 | 98,344 | 2,828 / 2,864 |
| A5 | 720,912 | 720,960 | +48 | 720,968 | 3,468 / 3,600 |
| A6 | 428,180 | 528,016 | +99,836 | 528,024 | 3,784 / 4,096 |
| B1 | 4,360,722 | 2,692,880 | −1,667,842 | 3,277,000 | 10,556 / 7,496 |
| B2 | 56,629 | 45,424 | −11,205 | 46,720 | 3,580 / 3,656 |
| B3 | 13,071,260 | 11,903,936 | −1,167,324 | 11,903,944 | 42,596 / 34,888 |
| B4 | 565,270 | 486,112 | −79,158 | 1,076,000 | 12,868 / 12,368 |
| B5 | 4,846,280 | 3,440,672 | −1,405,608 | 3,440,680 | 13,836 / 11,224 |
| B6 | 49,169 | 49,184 | +15 | 65,576 | 2,896 / 3,020 |
| B7 | 14,459,152 | 15,186,952 | +727,800 | 15,186,960 | 41,592 / 38,744 |
| B8 | 3,528,488 | 3,620,248 | +91,760 | 4,424,784 | 35,772 / 35,272 |
| B9 | 5,193,776 | 3,840,072 | −1,353,704 | 3,840,080 | 17,220 / 19,176 |
| B10 | 256,048 | 256,032 | −16 | 256,040 | 3,300 / 3,400 |

All compact scenarios returned to zero live cage bytes at completion. The per-phase release snapshots showed zero post-drop reusable bytes in these runs; B8 is the exception during its deliberate cache-churn cycles and is summarized below. Scenario repetitions were the harness defaults: 15 for A1–A6, 9 for B1–B4/B6/B8, and 7 for B5/B7/B9/B10. B10 used two build, two churn, and two traversal workers; the host had only two vCPUs.

## Fragmentation and allocator slack

The B8 diagnostic has a discrepancy that needs resolution before it is used as a V2.5 regression threshold. `PROFILE_V2_4_ROUND4.md` summarizes a 4,424,784-byte cursor high-water and a largest free extent of 804,528 bytes. The raw all-scenario `measure-stats.tsv` has 64 B8 cache-cycle snapshots and reaches a 5,211,232-byte cursor, 1,590,976 total free bytes, a 1,588,240-byte largest extent, and four free blocks. The table above reports the maximum from the ordinary per-phase rows (4,424,784 B); the separate cache-cycle diagnostic is a different path and must not be silently combined with it. Preserve both artifact readings until the orchestrator reconciles the earlier narrative with the raw run.

The production allocator's small size-class cache is bounded: four exact block sizes `[32, 40, 112, 528]`, up to 32 cached extents per class. Their maximum combined cage footprint is 22,784 bytes. The release collector contains 64 eight-byte extents (512 bytes) plus its length field and is stack-local to an active batch; TLS stores only a pointer to that stack object. The current code therefore has no unbounded per-thread allocation cache. This is a source-derived bound, not a measured per-thread RSS delta. B10 on this two-vCPU host does not establish 4/8-worker scaling or thread-exit RSS reclamation.

## Measurement gaps and next capture

The old harness records requested cage capacity (128 MiB in `benchmark_compare`), live bytes, cursor, free extents, and process `VmHWM`. It does not record `/proc/<pid>/status` `VmSize`, committed pages, or current `VmRSS` after teardown/quiescence. The 128 MiB setting is a cage capacity request, not a resident-memory measurement. The current artifacts therefore cannot establish virtual reservation, page commitment, or idle RSS relative to V2.4.

For the integrated V2.5 capture, retain this same compact-side baseline per scenario and add separate non-timing snapshots for current RSS after the workload has dropped owners and the process has quiesced, virtual mappings, and worker-thread exit. Keep timing binaries free of allocator telemetry. Record absolute-byte deltas alongside ratios; for scenarios with very small baseline RSS, compare byte deltas and repeatability rather than interpreting a rounded percentage as a meaningful alarm.

No runtime layout or allocator policy change is made by this workstream. The only code change is the test fixture for owner and descriptor layouts. No V2.5 memory-budget exception is requested.
