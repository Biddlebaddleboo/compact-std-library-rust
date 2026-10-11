# V2.5 Append-First Allocation and Background Maintenance Profile

- **Date:** 2026-10-11
- **Repository baseline:** `ec0dc24d07b0424ae470c0469e33974bc84b907a`
- **Plan commit:** `33e9fcb62cbfb93a7de514955337646de78641b5`
- **Host:** AArch64 Linux 6.17, two vCPUs; Rust 1.95.0

## Decision

The append-first experiment did not produce a repeatable end-to-end benefit. Its reduced free-list work was offset by cursor allocation work, and it increased cursor growth and peak RSS in B1 and B4. The temporary policy was removed. The existing reuse-first allocator remains unchanged.

The background worker phase was not implemented. The plan requires a worker to show repeatable benefit beyond append-first; that prerequisite was not met. No worker thread, scheduling controls, new free-space registry, or page-reclamation syscall was added.

The original exact 32-byte local reuse path, public APIs, allocation headers, and compact layouts remain unchanged.

## Method

The baseline is the plan's verified source commit. The default allocator policy is B (pending reuse enabled, size-class cache disabled). Performance measurements used telemetry-free release builds and the existing `benchmark_compare` harness, with 31 samples per phase and alternating native/compact variant order. All sixteen scenarios were captured for both baseline and a temporary append-first selector. The selector probed existing exact size classes, tried the cursor, then synchronously searched the general free list if the cursor was exhausted. It was removed after measurement.

Separate telemetry-enabled captures were used for allocator diagnostics; their timings are not compared with telemetry-free results. A temporary counter recorded cursor allocations that had visited at least one nonempty free-list node before reaching the cursor. This counter and its harness output were removed after the capture.

The key workloads B2, B4, B7, and B10 were repeated with 51 samples. B9 was repeated with 101 samples after an initial outlier. All benchmark runs reported matching native and compact checksums. A separate `/usr/bin/time -v` capture covered the full suite for process-tree CPU and page-fault counts.

## Full-suite performance

Compact end-to-end median times from the 31-sample capture:

| Scenario | Reuse-first | Append-first | Median change |
| --- | ---: | ---: | ---: |
| A1 | 0.072 ms | 0.072 ms | +0.5% |
| A2 | 1.676 ms | 1.690 ms | +0.8% |
| A3 | 0.585 ms | 0.607 ms | +3.8% |
| A4 | 1.066 ms | 1.065 ms | -0.1% |
| A5 | 2.762 ms | 2.762 ms | +0.0% |
| A6 | 1.272 ms | 1.269 ms | -0.2% |
| B1 | 17.848 ms | 18.067 ms | +1.2% |
| B2 | 1.153 ms | 1.151 ms | -0.2% |
| B3 | 17.775 ms | 18.126 ms | +2.0% |
| B4 | 7.182 ms | 7.353 ms | +2.4% |
| B5 | 4.806 ms | 4.820 ms | +0.3% |
| B6 | 0.079 ms | 0.079 ms | +0.7% |
| B7 | 14.205 ms | 14.748 ms | +3.8% |
| B8 | 21.380 ms | 21.479 ms | +0.5% |
| B9 | 4.494 ms | 5.149 ms | +14.6% |
| B10 | 1.194 ms | 1.200 ms | +0.5% |

The initial B9 increase was not repeatable. In its 101-sample rerun, the compact median was 4.498 ms for reuse-first and 4.439 ms for append-first; p95 was 5.329 ms and 5.055 ms, respectively.

The decision-critical repeats were:

| Scenario | Median change, 51 samples | p95 change, 51 samples |
| --- | ---: | ---: |
| B2 | -1.1% | -2.3% |
| B4 | +2.2% | -0.7% |
| B7 | +0.1% | +0.2% |
| B10 | +3.6% | +7.1% |

B10's compact/native median ratio was 0.812 for reuse-first and 0.811 for append-first, so the two-worker relative benefit remained intact. Append-first did not improve it.

The external full-suite capture measured 6.62 s user CPU and 7.23 s elapsed for reuse-first, versus 6.65 s and 7.31 s for append-first. Minor page faults were 530,277 and 536,914 (about +1.3%); major page faults were zero in both runs. This shows no suite-level CPU or page-fault benefit.

## Search work and memory

The temporary diagnostic found cursor allocations after a nonempty free-node visit in 75,040 of 75,456 B1 cursor allocations (99.4%) and 8,800 of 9,056 B2 cursor allocations (97.2%). B4 had 1,279,968 allocations served from the general free list, but none of its 131,168 cursor allocations followed a nonempty-node visit. B6 had 32 such cursor allocations out of 192. This shows that the searched-before-cursor pattern is workload-specific; free-list reuse itself is frequent in B1 and B4.

Append-first reduced general free-list allocations to zero in the scenarios where it ran, but this did not improve overall time. For example, B1's general-list allocations fell from 564,960 to zero while cursor allocations rose from 75,456 to 640,416. B4's general-list allocations fell from 1,279,968 to zero while cursor allocations rose from 131,168 to 1,411,136.

The telemetry-free per-process RSS and peak cursor measurements show the memory tradeoff:

| Scenario | Peak RSS reuse-first → append-first | Cursor high-water reuse-first → append-first |
| --- | ---: | ---: |
| B1 | 7,464 → 8,572 KiB (+14.8%) | 3,277,000 → 4,265,560 bytes (+30.2%) |
| B2 | 3,696 → 3,644 KiB (-1.4%) | 46,720 → 60,576 bytes (+29.6%) |
| B4 | 12,416 → 14,244 KiB (+14.7%) | 1,076,000 → 2,947,608 bytes (+174%) |
| B7 | 38,788 → 38,852 KiB (+0.2%) | 15,186,960 → 15,186,960 bytes |
| B10 | 3,460 → 3,524 KiB (+1.8%) | 256,040 → 256,040 bytes |

The maximum per-process RSS across the suite was B7 and changed by 0.2%; B1 and B4 individually exceeded the plan's 5% peak-RSS budget. The harness also verified zero live cage bytes after every measured scenario. It does not measure retained RSS across a process lifetime, since each scenario runs in a child process that exits after measurement.

## Acceptance

Append-first fails the plan's benefit gate: full-suite medians were flat or worse, the repeated B4 and B10 medians regressed, and B1/B4 showed higher per-scenario peak RSS. B7 was flat on repeat. The B10 relative compact/native ratio stayed the same. Therefore append-first was rejected and the worker phase was skipped. No p99 is reported because the existing harness emits median and p95 only; no optimization was accepted on this evidence.

No source or benchmark-harness changes from the temporary experiment remain in the final tree.

## Validation

- `cargo fmt --all -- --check` — passed.
- `cargo check --workspace --all-features --locked` — passed.
- `cargo test --workspace --all-features --locked` — passed (124 tests).
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check` — passed for all 16 scenarios.
- Apple cross-target checks with all features — passed for `x86_64-apple-darwin` and `aarch64-apple-ios`.
- The repository's seven nightly Miri workflow commands — passed (100 tests total), with `MIRIFLAGS=-Zmiri-disable-isolation` and `PROPTEST_CASES=16`.
- The benchmark harness CI self-check workflow — passed.
