# V2.5 system-wide deterministic extent reuse

## Outcome

The existing contention-gated local extent cache remains the selected runtime
policy. The always-attempt, broad-size, and direct-lock candidates each lost at
least one measured gate, so none was integrated. The four exact size classes
(`[32, 40, 112, 528]`), 4 KiB cage-wide cache budget, 16-owner limit, and
16-entry per-owner limit remain unchanged. This pass adds opt-in cache telemetry
and an exact-match regression test for a cache containing mixed extent sizes;
it does not change allocation or release behavior in the default build.

The latest plan commit was `3d6d6d5` (`plan: expand system-wide deterministic
extent reuse`), fast-forwarded before implementation. This report records the
experiments and the reason the incumbent policy was retained. The earlier
allocator report measured a 55.5% two-worker B10 improvement and a 5.4%
one-worker B10 regression; the one-worker gap remains open.

## Scope of the change

- `crates/compact_backend_std/src/cage.rs` adds fixed atomic counters behind
  `allocator-telemetry` for local-cache lookups, miss reasons, release outcomes,
  evictions, current and peak cached bytes, and active owners. `AllocatorStats`
  exposes these fields only when that optional feature is enabled.
- `crates/compact_backend_std/src/deterministic_memory.rs` adds a unit test
  proving an exact compatible extent can be found when other sizes precede it
  in the existing bounded cache.
- `crates/compact_std/examples/benchmark_compare/main.rs` emits one
  `META local_reuse_summary` row per scenario when built with
  `allocator-telemetry`.

The telemetry consists of bounded counters and does not add timers or atomics
to default builds. The opt-in telemetry fields extend the feature-specific
`AllocatorStats` shape; the default feature set and the frozen allocation,
header, vector, and deque layouts are unchanged. No collection code, allocator
hierarchy, owner-affine region, or background reclamation thread was added.

## Host and method

Measurements used Linux AArch64 with two available CPUs and Rust 1.95.0. The
comparison harness covered all 16 scenarios with `json,toml`. Timing builds
did not enable allocator telemetry. The all-scenario harness records 7–15
repetitions per scenario, depending on the scenario; full-suite comparisons
below contain two paired captures and are exploratory. Dedicated allocator
comparisons used 101 samples per pair where noted. The harness reports median
and p95, so this pass does not claim p99 or p99.9 results.

The original all-16 timing capture is `/tmp/v25-system-reuse-baseline.tsv`.
The two incumbent/broad-cache captures are
`/tmp/v25-full-{baseline,broad}-{a,b}.tsv`. The final opt-in telemetry capture
is `/tmp/v25-system-reuse-final-telemetry.tsv`. These temporary files are not
part of the repository; the results needed to interpret the decision are
recorded below.

## Candidate decisions

### Always attempt local reuse

Enabling the local-cache path without waiting for allocator contention helped
single-worker B10 in two 101-sample pairs: end-to-end medians changed from
0.4537 to 0.4303 ms and from 0.4543 to 0.4302 ms (about 5.1–5.3% faster).
The same policy regressed compact B4 sharply in two isolated pairs: baseline
medians were 7.176 and 7.416 ms; candidate medians were 13.606 and 13.548 ms,
about 89% slower. Telemetry showed 440,980 local lookups, 41,190 hits, 64,980
empty-cache misses, 334,810 size/alignment-ineligible lookups, and 19,630
evictions in the adaptive B4 run. The miss and release-path overhead outweighed
the reuse benefit for this mixed event workload. This policy was rejected.

### Broader extent sizes and alignments

A bounded broad-size candidate validated each block through `block_layout`,
kept the existing byte budget, and preserved the exact-class fast path. In
three dedicated 101-sample B10 pairs, its median deltas were +22%, +2.7%, and
+1.3%; the first pair was an outlier, but the candidate did not show a
repeatable median win. Two full-suite pairs produced the following broad-cache
median changes versus their corresponding incumbent capture:

| Scenario | Pair A | Pair B |
| --- | ---: | ---: |
| A1 | −0.1% | +0.1% |
| A2 | +0.3% | −1.7% |
| A3 | +0.8% | +5.9% |
| A4 | −0.4% | −0.4% |
| A5 | −0.2% | +1.0% |
| A6 | −0.1% | +0.5% |
| B1 | +1.1% | +1.6% |
| B2 | +1.3% | −3.2% |
| B3 | +3.1% | +2.1% |
| B4 | +3.2% | 0.0% |
| B5 | +1.0% | +0.6% |
| B6 | +0.2% | +0.5% |
| B7 | −4.1% | +0.8% |
| B8 | −2.7% | −0.3% |
| B9 | +4.8% | −1.9% |
| B10 | −5.2% | −8.8% |

Positive values are slower. Each full-suite pair has only one harness capture;
scenario repetitions within that capture are not independent paired runs.
Telemetry found no demonstrated broad-policy reuse opportunity in B10: the
16 extents at 1 KiB or larger exceed the 4 KiB local budget, while repeated
64 KiB-owner allocations already use the exact 32-byte class. The gains in
B10 therefore did not establish that broader matching caused a benefit, and
B3 was slower in both captures. This candidate was rejected.

### Direct mutex lock

Replacing the allocation-path `try_lock` with `Mutex::lock` while keeping
contention-gated cache activation made one-worker B10 slower in two 101-sample
pairs: end-to-end medians increased by 1.5% and 1.7%, and p95 increased by
2.7% and 7.7%. This candidate was rejected; the incumbent lock path remains.

## Final-policy telemetry

The final all-16 telemetry run matched all measure/profile checksums. These
counters are for diagnosis only; telemetry-enabled timing is not compared with
the timing builds.

| Scenario | Local lookups | Hits | Empty misses | Ineligible releases | Cached releases | Evictions | Peak local bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| B4 | 440,980 | 0 | 440,980 | 339,030 | 0 | 0 | 0 |
| B10 | 64,016 | 63,964 | 45 | 16 | 63,980 | 0 | 64 |

B4 had no allocator contention, so the selected policy made no local-cache
hits. B10 reused exact-class extents almost every time after contention
activated the cache. The final cache occupancy returned to zero after worker
exit and stats flush. Across all scenarios there were no owner-limit misses;
the largest observed local-cache occupancy was 64 bytes against the 4 KiB
budget.

## Acceptance limits and remaining work

No runtime candidate met the plan's broad-benefit gate. The selected policy
preserves the previously measured two-worker B10 gain and avoids the measured
B4 regression from always-attempt reuse. The default allocator's local-cache
budget, owner bounds, reuse compatibility, and memory behavior are unchanged.
Because no runtime policy was adopted, this pass did not collect a new repeated
retained-RSS comparison; the prior report's one-sample RSS values are not
treated as proof of retained-memory behavior.

The timing evidence comes from one two-vCPU AArch64 host. Full-suite comparisons
have only two captures, and the harness does not provide statistically useful
p99/p99.9 estimates here. No Loom or sanitizer run was performed. Miri and the
existing concurrent allocator tests passed, but they do not prove every
concurrent interleaving. The local counters also do not add new measurements
for pending-queue depth or in-place resize outcomes; existing allocator stats
continue to report lock acquisitions, free-list visits, release batches,
cursor high-water, and pending-reuse scans.

## Validation

The final source passed:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check` — all 16 checksums matched.
- The `.github/workflows/miri.yml` test sequence with nightly Miri, `MIRIFLAGS=-Zmiri-disable-isolation`, and `PROPTEST_CASES=16`: core (3), collection ownership (22), deque view (8), batch access (9), allocator integration (1), allocator library (38), and V2.4/Serde (7) tests passed.
- `git diff --check`

The existing stress tests and Miri runs cover ownership and release paths, but
the concurrent proof limits above remain. No frozen owner/header/collection
layout changed.
