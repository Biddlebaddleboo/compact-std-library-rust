# V2.5 deterministic buffer growth and reuse results

## Outcome

The accepted runtime change avoids flushing the current thread's local reuse
cache and taking a second allocator lock after a failed in-place resize when
there is no published adjacent extent and the current cache has no extent
beginning exactly at the allocation's physical end. A short published extent
keeps the prior flush and retry path because a contiguous cached extent may
extend it. The check reads only the current thread's bounded cache. Resize
diagnostics
behind `allocator-telemetry` classify growth failures and report cache retry
work, physical block sizes, ownership transfers, and free-list search visits.

The path avoids flushing and retrying when the current cache cannot contribute
an adjacent extent. It did not show a reproducible end-to-end timing
improvement in B2, so this report makes no wall-clock speedup claim. It
preserves the default allocation, header, vector, and deque layouts, and does
not change the 32-byte local reuse rule.

## Source and benchmark identity

- Plan commit: `e481aa29a9c4b22c71321b9bec3dab94d049eb49`.
- Baseline source: `3448c8ce313d840012a6498386d760966e68881b`.
- Final allocator source blob: `1c1b79a4d1d507a68d75da11e8d0c94d72a5251e`.
- Candidate benchmark output source blob: `0c012b34028e01042850620998dcdf8e4926a52a`.
- Scenario definitions source blob: `1c553bb78de827c79bcb69b2fd3d3771115c114f`.
- Host: Linux AArch64, 2 available CPUs, kernel `6.17.0-1020-oracle`.
- Toolchain: `rustc 1.95.0 (59807616e 2026-04-14)`, LLVM 22.1.2.
- Release timing builds used `json,toml` and did not enable allocator telemetry.
  Telemetry runs were separate diagnostic builds.

## Measurement method and all-scenario results

The comparison used three baseline captures and four final-source candidate
captures, each covering all sixteen scenarios with 31 runs and matching
checksums. Candidate order was alternated twice and fixed twice. The table
compares the median of capture medians and the median of capture p95 values.
These samples support median and p95 summaries only; they do not support p99
or p99.9 claims. The TSV captures are `/tmp/profile-v25-buffer-baseline-a.tsv`,
`baseline-b.tsv`, `final-baseline.tsv`, and `final-guard-{a,b,c,d}.tsv`.

Times are compact end-to-end milliseconds. Percent changes compare candidate
with baseline; positive values are slower.

| Scenario | Baseline median / p95 | Candidate median / p95 | Median change | p95 change |
| --- | ---: | ---: | ---: | ---: |
| A1 | 0.072 / 0.088 | 0.071 / 0.100 | −0.3% | +14.1% |
| A2 | 1.673 / 1.713 | 1.700 / 2.851 | +1.6% | +66.4% |
| A3 | 0.607 / 0.706 | 0.600 / 0.663 | −1.2% | −6.1% |
| A4 | 1.072 / 1.103 | 1.069 / 1.637 | −0.3% | +48.3% |
| A5 | 2.758 / 2.930 | 2.787 / 4.109 | +1.1% | +40.2% |
| A6 | 1.262 / 1.603 | 1.286 / 2.058 | +1.9% | +28.4% |
| B1 | 17.960 / 18.742 | 17.964 / 18.567 | +0.0% | −0.9% |
| B10 | 1.236 / 1.997 | 1.213 / 1.726 | −1.9% | −13.5% |
| B2 | 1.163 / 1.369 | 1.158 / 1.282 | −0.4% | −6.3% |
| B3 | 17.628 / 18.183 | 17.850 / 19.170 | +1.3% | +5.4% |
| B4 | 7.167 / 7.664 | 7.186 / 7.628 | +0.3% | −0.5% |
| B5 | 4.815 / 4.884 | 4.827 / 5.037 | +0.3% | +3.1% |
| B6 | 0.079 / 0.096 | 0.079 / 0.096 | +0.2% | −0.8% |
| B7 | 14.340 / 15.028 | 14.429 / 14.963 | +0.6% | −0.4% |
| B8 | 21.557 / 22.358 | 21.607 / 23.685 | +0.2% | +5.9% |
| B9 | 4.616 / 5.507 | 4.782 / 6.280 | +3.6% | +14.0% |

Median differences are small and mixed. Several p95 values moved substantially
even in scenarios without resize activity; these captures do not establish a
causal p95 effect. A dedicated B2 101-run pair measured baseline medians/p95s
of 1.136/1.171 ms and candidate values of 1.146/1.179 ms (+0.9%/+0.7%), which
is flat within noise. A dedicated B5 101-run pair measured −0.6% median and
−5.7% p95; B5 had no resize activity.

Dedicated B10 101-run captures showed no resize activity, so they do not
exercise the accepted path. One-worker baseline/candidate median and p95 were
0.400/0.513 ms and 0.406/0.534 ms. Two-worker values were 1.234/1.313 ms and
1.212/1.347 ms. These results do not show a repeatable B10 regression, and do
not establish a benefit.

## Resize diagnosis

The B2 diagnostic capture covered 31 runs. Counters below are for the
telemetry-enabled build and are not timing comparisons.

| B2 resize diagnostic | Count or bytes |
| --- | ---: |
| Initial locked resize attempts | 768 |
| No-growth / cursor-growth / adjacent-free growth | 32 / 32 / 64 |
| Failed growths | 640 |
| Adjacent published free extent too short | 32 |
| No adjacent published free extent | 608 |
| No-adjacent cases with free bytes elsewhere | 576 |
| Current-cache flush retries skipped as unrelated | 32 |
| Cache bytes flushed on resize retry | 0 |
| Successful ownership transfers / initialized bytes moved | 640 / 467,200 |
| Old / requested / incremental physical block bytes examined | 524,800 / 1,037,056 / 512,256 |

Before the optimization, the 32 unrelated local-cache cases flushed 1,024
bytes, took a second lock, and retried resize without succeeding. In the final
capture, these retries were skipped; observed allocator lock acquisitions
dropped by 96. B2 ended with no allocation-exhaustion errors, and replacement
allocations preserved the vector contents.

Each attempt ran under the allocator lock after its normal pending-release
drain. No size-class merge calls occurred in B2. Of the 608 cases with no
adjacent published extent, 576 had shared free space elsewhere. The other 32
had local cache occupancy but no exact-adjacent local extent. Under B2's
single-threaded allocator invariants, those 32 cases had no reclaimable
adjacent free extent; the intervening bytes therefore blocked in-place
growth. Scattered free bytes do not make contiguous growth possible. The
remaining 32 failed growths had an adjacent extent that was too short.

## Candidate decisions

| Candidate | Decision | Evidence |
| --- | --- | --- |
| A. Skip size-class merging for cursor growth | Rejected | Across all sixteen scenarios the resize merge counter was zero. There was no observed merge cost to remove. |
| B. Combine adjacent free-node lookup and consumption | Rejected | A single-pass prototype did not produce a reproducible end-to-end gain. The existing lookup and consumption remain. Free-list visit telemetry now counts resize traversal work, but its expanded instrumentation makes before/after visit totals incomparable. |
| C. Skip an unrelated local-cache flush and retry | Accepted | Only a no-adjacent-published-extent result can take the fast path. In B2, 32 such failed growths had no exact-adjacent extent in the current thread's bounded cache. The final run skipped those flushes, 1,024 flushed bytes, and 32 second resize attempts. A published but short adjacent extent retains the flush/retry because a contiguous local extent may extend it. Timing was flat within noise, so acceptance is based on avoiding retry work when the current cache cannot contribute. |
| D. Change replacement allocation scanning | Deferred | The measurements did not isolate a replacement-scan bottleneck. A scan-policy change needs a separate bounded-fragmentation experiment. |
| E. Change ownership transfer | Deferred | Telemetry counted moves and initialized bytes, but no transfer implementation showed a measured gain. Movement semantics remain unchanged. |
| F. Change capacity growth factor | Deferred | This would alter retained capacity across collections; the resize diagnosis gave no evidence that the current growth factor was the cause of B2's failed in-place growth. |
| G. Change shrink and tail reuse | Deferred | No repeated grow/shrink bottleneck was identified in this pass, so changing reclamation policy lacked a measured target. |

The resize layout remains fixed: no owner/header/vector/deque size or public
collection behavior changed. No live values are moved by the accepted resize
optimization. The current thread's local cache is examined through its
existing bounded TLS state while the allocator lock is held; shared free-list
state is inspected only under the existing allocator synchronization.

## Memory and limitations

Retained/free/high-water/extra allocator memory columns matched between
baseline and candidate for every scenario. Across two final-source RSS runs,
the largest median peak increase against the baseline run was 0.3% (B10).
This stays within the plan's +2% retained memory and +5% peak RSS limits.

Measurements came from one two-vCPU AArch64 host. The all-scenario captures
have 31 runs per scenario and only a few paired capture sets. The Miri and
existing concurrent tests exercise ownership and release behavior but do not
prove every concurrent interleaving. No p99/p99.9 result or general concurrency
proof is claimed.

## Validation

The final source passed:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check` — all sixteen measure/profile checksums matched.
- Four final-source sixteen-scenario timing captures, two dedicated 101-run B2 captures, and the B2 telemetry profile; scenario checksums matched.
- `git diff --check`

The complete `.github/workflows/miri.yml` sequence passed 100 tests on the
immediately preceding C-only source: core (3), collection ownership (22),
deque view (8), batch access (9), allocator integration (1), allocator
library (50), and V2.4/Serde (7). The final source's only follow-up was to
narrow the cache shortcut to the explicit no-adjacent outcome; the full native
workspace tests, strict Clippy run, and Apple cross-target check above passed
after that change. The Miri sequence was not repeated after this internal
control-flow adjustment.

No default allocation or collection layout changed. The optimization preserves
fallback behavior through the caller's existing replacement-allocation path
when in-place growth returns `false`.
