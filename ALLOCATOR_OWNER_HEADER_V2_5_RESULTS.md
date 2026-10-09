# V2.5 owner-header validation experiment

**Status: accepted and integrated on `main` at `bdeccd9`.** Central review approved the owner-header fast path after source audit, full Miri, focused/full-suite timing, memory accounting, and compact-only perf counters. The real over-aligned owner resize/reuse integration guard is at `c941e9e`. No chunk/TLS code, public layout change, or API change was made.

## Candidate and artifacts

The candidate uses `read_owner_header<T>(state, &CageAllocation<T>)` for private owners. It retains all generic `read_header` checks and skips only `validate_typed_header`. Typed raw offsets still use `read_typed_header<T>`; byte offsets and the type-erased private Drop path retain full generic `read_header` checks. The owner provenance and header-writer audit is in `ALLOCATOR_OWNER_HEADER_V2_5.md`.

Pinned baseline source: `b3ca8786448c5fa0b7e503e915414170eaa31e31`.

| Artifact | Path | SHA-256 |
| --- | --- | --- |
| Baseline `benchmark_profile` | `/tmp/csl-v25-targeted-baseline/target-plain/release/examples/benchmark_profile-574d1082e0027351` | `d9d596aa07bd47ded1707d087dcd8c4f6973697b54fbb6c38a634ccb7863d612` |
| Baseline `benchmark_compare` | `/tmp/csl-v25-targeted-baseline/target-plain/release/examples/benchmark_compare-8d33004f27ec8ddd` | `5182e93f5afcd9e31d91bb85c460383c3a4b66fc170cc37db04a85c4643937c5` |
| Candidate `benchmark_profile` | `/tmp/csl-v25-owner-header-candidate/plain/release/examples/benchmark_profile-574d1082e0027351` | `29c13748ad3c2f0b5b493093bfb7058bdbf82cfcc15475aff35b10baad10aec5` |
| Candidate `benchmark_compare` | `/tmp/csl-v25-owner-header-candidate/plain/release/examples/benchmark_compare-8d33004f27ec8ddd` | `35f66ca8d23026afc0696df0e21339d7bb68940eff434320cb3cbf6035019a94` |

The candidate binaries were built telemetry-free with release defaults, `--no-default-features --features json,toml`, and no `RUSTFLAGS`. Their build log is `/tmp/csl-v25-owner-header-candidate/build/plain-build.jsonl`. Pinned baseline artifacts were not modified.

## Timing results

Five targeted scenarios used nine paired baseline/candidate captures each, alternating process and native/compact order. The full 16-scenario suite used two paired accounting-free captures with alternating suite and variant order. Times below are nanoseconds per scenario repetition, normalized by the harness's `runs_per_window`. Each harness run and each baseline/candidate pair had matching native/compact checksums.

| Scenario | Focused compact delta, 9 pairs | Full-suite compact median delta, 2 pairs | Full-suite compact p95 delta, 2 pairs |
| --- | ---: | ---: | ---: |
| A2 | −8.99% | −8.29% | −8.81% |
| A4 | −17.73% | −16.78% | −16.58% |
| B6 | −11.24% | −9.16% | −9.27% |
| B3 sentinel | −1.06% | −0.61% | −0.49% |
| B5 sentinel | −3.73% | −1.16% | −0.94% |
| B10 | — | −23.94% | −10.33% |

The complete 16-scenario baseline/candidate median, p95, RSS, and checksum table is [owner-header-all16/summary.tsv](/tmp/csl-v25-owner-header-candidate/runs/owner-header-all16/summary.tsv). Small full-suite median shifts were +0.46% A1, +0.98% A6, +0.31% B2, and +1.03% B4; their p95 shifts were between −2.10% and +3.63%. With two full-suite observations, these are within run noise. A5's median improved 0.69%, while its p95 increased 2.01%.

The focused 9-pair table and raw per-run logs are in `/tmp/csl-v25-owner-header-candidate/runs/owner-header-5scenarios/`. Full-suite raw TSV/logs are in `/tmp/csl-v25-owner-header-candidate/runs/owner-header-all16/`.

### Compact-only hardware counters

Root captured compact-only `perf stat --repeat 3` against the pinned baseline and candidate binaries. Checksums matched. Values are totals per capture, ordered by cycles, instructions, branches, branch misses, and cache misses.

| Scenario | Counter | Baseline | Candidate | Delta |
| --- | --- | ---: | ---: | ---: |
| A2 | Cycles | 26,882,977,841 | 24,354,538,508 | −9.41% |
| A2 | Instructions | 78,624,066,793 | 67,241,631,773 | −14.48% |
| A2 | Branches | 19,148,832,486 | 17,397,683,353 | −9.14% |
| A2 | Branch misses | 3,142,156 | 2,719,540 | −13.45% |
| A2 | Cache misses | 87,452,360 | 90,487,657 | +3.47% |
| A4 | Cycles | 19,471,942,370 | 15,951,419,710 | −18.08% |
| A4 | Instructions | 66,554,519,068 | 55,059,245,890 | −17.27% |
| A4 | Branches | 14,850,882,029 | 13,208,571,763 | −11.06% |
| A4 | Branch misses | 5,229,460 | 478,209 | −90.86% |
| A4 | Cache misses | 17,404,510 | 16,010,145 | −8.01% |
| B6 | Cycles | 16,295,483,054 | 14,390,251,497 | −11.69% |
| B6 | Instructions | 54,756,844,165 | 47,319,626,934 | −13.58% |
| B6 | Branches | 12,401,865,037 | 11,405,325,570 | −8.04% |
| B6 | Branch misses | 20,591,615 | 19,415,300 | −5.71% |
| B6 | Cache misses | 138,121,776 | 135,622,933 | −1.81% |
| B10 | Cycles | 11,404,226,535 | 11,362,466,877 | −0.37% |
| B10 | Instructions | 11,287,369,151 | 10,871,132,629 | −3.69% |
| B10 | Branches | 2,372,144,033 | 2,300,018,592 | −3.04% |
| B10 | Branch misses | 24,982,336 | 26,370,843 | +5.56% |
| B10 | Cache misses | 61,537,693 | 63,212,753 | +2.72% |

A2 had a 3.47% cache-miss increase; B10 had 5.56% more branch misses and 2.72% more cache misses. The sampled A2/A4/B6 hot paths improved in wall time and cycles, and no retained-byte/high-water changes occurred.

### B10 timing limits

B10 is especially sensitive to scheduler noise on this two-vCPU VM. In the
two-suite full profile, compact's median moved −23.94%. A separate nine-pair
zero-second run moved +10.56% (2.715 to 3.001 ms/rep), with p95 3.059 to
3.992 ms. Nine paired 0.5-second windows moved −10.18% (3.074 to 2.761
ms/rep), with p95 3.156 to 3.072 ms. The 0.5-second compact counter capture
was nearly flat: cycles −0.09%, instructions −2.08%, branches −0.32%, branch
misses +2.32%, cache misses +2.69%; native counters varied by 16–20% between
the two release binaries. Raw captures are under
`/tmp/csl-v25-owner-header-counters/b10-stable-window/` and
`/tmp/csl-v25-owner-header-candidate/runs/b10-owner-header-9pairs/`.

Treat the B10 latency delta as inconclusive, not as a candidate win or a
confirmed regression. Acceptance rests on the repeatable A2/A4/B6 gains,
improving B3/B5 sentinels, unchanged retained/high-water bytes, and no
material full-suite p95 regression; the small p95 shifts are within the
two-run noise. Continue B10 monitoring on a multicore host when available.

## Memory and accounting

One all-16 `benchmark_compare --mode measure` run per binary verified checksums and returned zero live cage bytes after every compact scenario. Across all scenarios:

- Peak compact cage high-water bytes were identical between baseline and candidate.
- Peak requested/native bytes were identical between baseline and candidate.
- Post-scenario retained cage bytes were zero.
- Two-run accounting-free compact RSS medians differed by −132 to +10 KiB (at most +0.31%).
- Single-run measure-mode compact RSS deltas ranged from −60 to +132 KiB; the largest increase was B6 at +4.57%. Peak high-water bytes did not change, so the RSS movement is process/host noise rather than an allocator footprint change.

Per-scenario memory values are in [measure-memory-deltas.tsv](/tmp/csl-v25-owner-header-candidate/runs/measure-memory-deltas.tsv); raw measure outputs are `measure-baseline.tsv/.log` and `measure-candidate.tsv/.log` under the same `runs` directory. Root also captured compact-only `perf stat --repeat 3` cycles, instructions, branches, branch misses, and cache misses for A2/A4/B6/B10; raw files are in `/tmp/csl-v25-owner-header-counters/` and are summarized above. The strong cycles/instructions reductions in A2/A4/B6 support the resolved-header hypothesis; B10 counters are effectively unchanged, as expected for allocator-mutex contention.

## Validation

- `cargo fmt --all` — completed.
- `CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate cargo test --locked -p compact_backend_std --lib` — 36 passed, 0 failed.
- All 16 benchmark checksum pairs matched in both full suites; all five targeted 9-pair runs matched checksums.
- Full Miri workflow used nightly, `MIRIFLAGS=-Zmiri-disable-isolation`, and `PROPTEST_CASES=16`. Exact commands:

```sh
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri setup
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_core
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_collections --test cage_collections
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_collections --test deque_view
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_collections --test collection_batch_access
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_backend_std --test integration
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_backend_std --lib
MIRIFLAGS=-Zmiri-disable-isolation PROPTEST_CASES=16 CARGO_TARGET_DIR=/tmp/csl-v25-owner-header-candidate/miri cargo +nightly miri test -p compact_std --all-features --test v2_4
```

`compact_core` passed 3; `cage_collections` 22; `deque_view` 8; `collection_batch_access` 9; backend integration 1; backend lib 36; and `compact_std --all-features --test v2_4` 7. **Total: 86 passed, 0 failed.** The modified backend library Miri run took 131.42 seconds.

An initial backend Miri invocation omitted the workflow's `PROPTEST_CASES=16` and was interrupted while the default-sized property test was running; it is not counted. The exact workflow-configured rerun passed.

## Decision

Accepted. The constructor/write/resize audit and tests support the payload-fit invariant for allocator-issued owners. Nine-pair measurements show useful gains in A2, A4, and B6; B3/B5 sentinels do not regress. All-16 high-water and retained-byte metrics are unchanged, full Miri passes, and compact-only cycles/instructions support the wall-time results. Integrate only this bounded owner-header fast path; do not extend it into chunk/TLS work.
