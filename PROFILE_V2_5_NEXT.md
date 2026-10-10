# V2.5 Performance Gap Reduction — Next Pass

## Result

One runtime optimization passed the A5/B8/B5 gate: the randomized compact table builder now uses keyed SipHash-1-3. Nine paired runs improved compact medians by 5.6% in A5 and 6.0% in B8; B5 was expanded to 30 pairs and improved 3.3%. Full-workload checksums, retained bytes, allocator high-water marks, and free-range shapes matched.

The ordinary deque mask specialization was rejected after a 1.7% A4 median regression. No unsafe owner-affine allocator chunks were added. The allocator model now records fresh chunk generations and quarantine behavior, while production chunk allocation remains blocked by unresolved concurrent-safety requirements.

The new memory probe and its script provide repeatable process and cage snapshots without inserting allocator telemetry into timing runs. This report replaces the `PLAN*.md` execution notes; all `PLAN*.md` files were removed in the implementation commit.

## Source and measurement identity

| Role | Revision or artifact |
| --- | --- |
| Pinned V2.5 runtime baseline | `4f1c1aa222bf6a62ba6a72b4585ab24799135c42` |
| Measurement parent | `0cab59460b2c3c52bdbce567e0fd678e8d2f4fe0` |
| Baseline runtime source | Equal to the pinned runtime baseline under `crates/`, `Cargo.toml`, and `Cargo.lock`; verified with `git diff --quiet 4f1c1aa HEAD -- crates Cargo.toml Cargo.lock` before local changes |
| Hash candidate binary | SHA-256 `7943ed285e3a69585b473f250bfe9baebf8641fb14bb7361aa679224631c6d78` |
| Baseline `hash_map.rs` | SHA-256 `a784fbbc725e46149f8198bf38e2052c03875b3fb4bf6dc3b1e5cdcd59c580f1` |
| Integrated `hash_map.rs` | SHA-256 `5be12cd1126a70e1796835cd669d3bd2ab91ef864a7a851e2598067c0da892d3` |
| Baseline profile binary | SHA-256 `29c13748ad3c2f0b5b493093bfb7058bdbf82cfcc15475aff35b10baad10aec5` |
| Hash candidate profile binary | SHA-256 `0e3138b00a088a4a9a16bfc71ec8641b077e0a83931ea45cfde59fdd9aec398f` |
| Hash candidate checksum binary | SHA-256 `11696c6d4ffbd3f32b2175faee6cdb92cc31c5da809630657938741a51a9f193` |
| Implementation commit | SHA recorded in the executor handoff; the report is part of that commit |

The timing and counter captures used Linux AArch64 on a two-vCPU Neoverse-N1 host, Rust 1.95.0 / LLVM 22.1.2, and telemetry-free release builds with the `json,toml` features. The baseline full-suite run ID was `v25-next-baseline-20261010`; artifacts are under `/tmp/csl-v25-next-baseline/runs/`. The hash A/B artifacts are under `/tmp/csl-v25-next-hash-ab/`. No timed run used allocator snapshots or a counting global allocator.

## Pinned-runtime performance snapshot

The baseline harness ran the accounting-free all-16 suite twice and repeated A2/A4/A5/B6/B8/B3/B5/B10 nine times. The table shows compact and native medians from those nine-run baseline captures. Timings are nanoseconds per scenario run.

| Scenario | Compact median / p95 | Native median / p95 | Compact / native median |
| --- | ---: | ---: | ---: |
| A2 | 1,803,168 / 2,096,845 | 1,331,245 / 1,392,720 | 1.35× |
| A4 | 1,162,219 / 1,191,494 | 396,712 / 412,403 | 2.93× |
| A5 | 3,187,080 / 4,161,193 | 1,304,850 / 1,370,469 | 2.44× |
| B6 | 103,321 / 113,210 | 60,209 / 62,440 | 1.72× |
| B8 | 27,628,386 / 28,614,691 | 13,488,757 / 14,657,148 | 2.05× |
| B3 | 23,118,295 / 23,485,352 | 32,175,415 / 32,492,071 | 0.72× |
| B5 | 7,504,850 / 7,700,652 | 10,191,338 / 11,176,900 | 0.74× |
| B10, two workers | 2,835,552 / 3,195,652 | 426,312 / 632,822 | 6.65× |

B3 and B5 remain faster than native. A2, A4, A5, B6, B8, and B10 remain the main runtime gaps. The separate B10 one-worker capture measured compact at 593,416 ns median / 1,329,427 ns p95 and native at 282,425 ns / 657,262 ns. The host has two vCPUs; these measurements make no physical multicore scaling claim.

## Accepted hash candidate

The profile identified `CompactHashMap::insert` and `find_index_in` as material A5/B8 paths. `SipHasher24::finish` accounted for 10.87% of A5 compact samples and 7.03% of B8 compact samples in the baseline sample capture. The implementation generalizes the existing hasher over compile-time round counts and configures the default randomized table builder for SipHash-1-3. `SipHasher24` remains available internally for the existing 2-4 reference tests. The default builder still derives its keys from `RandomState`; table storage, key/value layout, control bytes, probing, and APIs do not change.

Rust documents SipHash-1-3 as the current standard-library `HashMap` default, while noting that this may change. The Linux implementation describes the 1-3 variant as suitable for hash tables only. This implementation is for keyed table hashing; it is not a message-authentication or general cryptographic API. See the [Rust `HashMap` documentation](https://doc.rust-lang.org/std/collections/hash_map/struct.HashMap.html) and [Linux SipHash implementation notes](https://github.com/torvalds/linux/blob/master/lib/siphash.c).

The baseline and candidate binaries were run in alternating order with native/compact ordering alternated as well. Every run matched native/compact checksums, and candidate checksums matched baseline checksums. Compact timing comparisons use the same per-scenario workload and `--seconds 0` accounting-free driver. B5 was expanded from nine to 30 pairs after the initial native-control p95 varied.

| Scenario | Pairs | Compact median baseline → candidate (ns) | Paired median change | Compact p95 baseline → candidate (ns) | Native median baseline → candidate (ns) |
| --- | ---: | ---: | ---: | ---: | ---: |
| A5 | 9 | 3,153,677 → 2,975,395 | −5.56% | 3,426,588 → 3,056,849 | 1,300,244 → 1,293,148 |
| B8 | 9 | 27,626,689 → 25,936,890 | −5.97% | 27,822,344 → 26,156,171 | 13,415,966 → 13,383,726 |
| B5 | 30 | 7,507,410 → 7,265,646 | −3.31% | 7,767,058 → 7,552,468 | 10,026,960 → 10,068,926 |

The candidate compact p95 improved in all three scenarios. B5’s native median moved by +0.06% across the 30 paired runs; its native p95 moved by +3.47%, while compact p95 improved by 2.76%. The paired p95 of compact timing deltas was +0.20%, so the high-tail timing does not show a material compact regression.

`perf stat --repeat 3` used the full benchmark repetition counts: A5/B5 3,000 runs per window and B8 600. These are whole-process counts, not phase-only counts. All requested events were available. Compact candidate counts changed as follows:

| Scenario | Cycles | Instructions | Branches | Branch misses | Cache misses |
| --- | ---: | ---: | ---: | ---: | ---: |
| A5 | −5.36% | −4.76% | −3.24% | +0.07% | −0.12% |
| B8 | −6.02% | −5.74% | −3.01% | −1.03% | −0.69% |
| B5 | −4.42% | −5.45% | −3.64% | +0.81% | −0.27% |

The hardware counters support the timing result: the candidate executes fewer cycles and instructions for all three compact workloads. Small branch-miss increases in A5 and B5 were below 1% and did not produce a compact p95 regression.

Allocator-accounted `benchmark_compare` runs were used only for memory. Baseline and candidate retained bytes, cursor high-water, and free extents were identical:

| Scenario | Compact live bytes | High-water cursor | Free bytes / blocks / largest block |
| --- | ---: | ---: | ---: |
| A5 after build and mutation | 720,960 | 720,968 | 0 / 0 / 0 |
| B5 after build | 3,440,672 | 3,440,680 | 0 / 0 / 0 |
| B8 after build | 3,620,248 | 3,620,256 | 0 / 0 / 0 |
| B8 after fixed-population churn | 3,620,360 | 4,424,784 | 804,416 / 2 / 802,848 |

Single-run peak RSS was 3,540 → 3,468 KiB in A5, 11,096 → 11,032 KiB in B5, and 35,208 → 35,144 KiB in B8. These small decreases are treated as measurement noise, not as a memory benefit. The `benchmark_profile` binary was 16 bytes smaller (2,135,704 → 2,135,688 bytes).

## Rejected deque candidate and collection findings

A fresh A4 sample profile placed 59.40% of compact samples in `CompactVecDeque::push_back` and 35.75% in `pop_front`. The isolated candidate specialized `physical_index` to use a masked add for power-of-two capacities and retained the original path otherwise. Across nine alternating A4 pairs, checksums matched, but compact median latency regressed 1.70% (1,226,662 → 1,236,840 ns). Candidate p95 was lower (1,348,877 vs 1,469,682 ns), but the median did not pass. Cage live bytes, high-water cursor, and free-range shape were identical. Candidate binary SHA-256: `905a592bb34e67822a155c80897efb870705ed6642b9abcb06e22be6043bfd7f`. The patch was discarded.

The fresh B6 profile places 27.91% of compact samples in `CompactVec::retain` and 17.55% in `IndexMut`. Existing `CompactVec::as_mut_slice` and deque `with_view` batching remain available; no new vector or batched-access change was justified in this pass. A5/B8 were reprofiled after the hash change. Their sampled paths still center on map insertion, probing, and control classification; the low-overhead sample sets are diagnostic and are not used as timing percentages.

The prior seven-bit fingerprint and EMPTY-only classifier results remain rejected as recorded in [HASH_V2_5.md](HASH_V2_5.md). The classifier was not repeated because the plan forbids rerunning that failed design without a new hypothesis.

## Allocator model, memory probe, and remaining gates

`allocator_model.rs` now assigns each model chunk a monotonic generation, associates remote-release pins with that generation, and models quarantined chunks. Quarantine preserves live/pending accounting and forbids reuse, remote flush, or reclamation. New unit tests cover a pinned release surviving quarantine and a reused arena interval receiving a fresh generation. These are sequential model transitions, not a production synchronization proof.

The eight open requirements before unsafe owner-affine chunks are:

1. Prove production offset-to-record routing and that unique-owner `Drop` is the only release source.
2. Define the real registry pin publication/removal ordering and its synchronization primitive.
3. Integrate thread-local destruction and reaping with in-flight operations on real threads.
4. Bound remote pending storage without allocation in `Drop`; define overflow, nested collectors, and flush-error behavior.
5. Define panic, poisoned-lock, allocation-failure, retry, and quarantine behavior during reclamation.
6. Keep live/free/reserved accounting and stats snapshots coherent without changing frozen layouts.
7. Bound chunk sizes, metadata, queues, fragmentation, and thread-churn memory costs with measurements.
8. Add concurrent stress plus model-checker evidence and obtain central review of a concrete unsafe implementation.

No Loom dependency is present. The model and Miri runs cannot establish the real concurrent ordering needed for an unsafe chunk implementation. The current B10 global-mutex path therefore remains unchanged. The prior BPF critical-section histogram is invalid because it paired lock returns with wildcard transaction-drop events instead of reliable one-to-one lock/transaction pairs; this pass reports no lock-hold-time estimate. B10 perf sampling produced no usable stack samples, so timing and hardware counters are the available evidence.

The new `crates/compact_std/examples/memory_profile.rs` probe and `scripts/profiling/memory_profile.sh` script ran five telemetry-free snapshots of a 128 MiB cage with 512 KiB live payload. Median Linux process/cage readings were:

| Snapshot | VmSize / VmRSS (KiB) | Cage live / cursor (bytes) |
| --- | ---: | ---: |
| Initialized | 133,980 / 1,820 | 0 / 8 |
| Main-thread owners live | 133,980 / 2,336 | 526,336 / 526,344 |
| Main owners dropped; 200 ms quiet | 133,980 / 2,404 | 0 / 8 |
| Worker owners live | 201,648 / 2,420 | 526,336 / 526,344 |
| Worker exited; 200 ms quiet | 201,628 / 2,488 | 0 / 8 |

After both drops there were zero live cage bytes, zero free extents, and no swap. RSS remained resident after touching and releasing the payload. Worker creation increased virtual size by about 67.6 MiB, which remained mapped after join. This probe reports `VmSize`, RSS, anonymous/private-dirty values, and allocator stats; this host did not expose an exact per-process committed-page total. RSS after thread exit is not interpreted as retained live cage bytes.

## Validation

The integrated tree passed:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check` — all 16 checksums matched between measure and profile modes.
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- The complete `.github/workflows/miri.yml` command set using nightly Miri: core (3), collection ownership (22), deque view (8), batched access (9), allocator integration (1), allocator library (38), and V2.4/Serde (7) tests passed.
- After the hash change, Miri also passed `siphash13_matches_reference_vectors` and `hash_collisions_tombstones_and_panicking_hash_are_consistent`.
- The candidate accounting-free full 16-scenario profile suite passed twice with alternating native/compact order. The harness self-check passed for all 16 scenarios on the integrated tree.

The all-16 baseline and candidate checksum outputs, scenario summaries, memory data, perf reports, counters, and binaries are under `/tmp/csl-v25-next-baseline/`, `/tmp/csl-v25-next-memory-baseline-final/`, `/tmp/csl-v25-next-hash-ab/`, and `/tmp/csl-v25-next-hash-samples/`. No implementation changes were made to the frozen owner/header/deque layouts or typed raw-offset validation.
