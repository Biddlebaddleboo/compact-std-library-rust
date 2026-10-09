# V2.5 Hash Workstream Result

## Decision

**Rejected for production.** The isolated candidate uses an EMPTY-only SIMD
mask inside `first_empty_slot`, which is called while rehash places entries.
That path consumed only the EMPTY mask from the original three-state
classifier. The ordinary lookup and insertion probes retain the unchanged
EMPTY/FULL/TOMBSTONE classifier, including invalid-byte detection. Hashing,
control-byte layout, per-entry metadata, randomized keys, and collision
resistance are unchanged.

The full-workload timing change was too small and inconsistent to justify
keeping another classifier implementation: compact medians improved by 1.14%
in A5, 0.30% in B5, and 0.43% in B8. B8 p95 worsened 4.07%; B5 p95 worsened
0.87%. Hardware counters showed no A5 or B5 instruction reduction. B8 showed
about 1.4% fewer whole-process cycles and instructions, but branch misses rose
2.69%, and the workload median improvement remained below one-half percent.
There was no retained-memory reduction.

## Candidate and source

- Baseline source: `b3ca8786448c5fa0b7e503e915414170eaa31e31`.
- Isolated candidate commit: `0324fd0696382652a09cbbd8bcb88bf01245c0fb`
  (`codex/v25-hash`). It is not merged or pushed.
- Changed files: `crates/compact_collections/src/hash_control.rs` adds scalar,
  SSE2, and NEON empty-only classification; `hash_map.rs` routes only
  `first_empty_slot` through it and tests that wrapped/partial scans preserve
  prior EMPTY-slot behavior, including invalid control bytes.
- Candidate has no table-layout or retained-byte changes.

## Tests

Commands:

```sh
cargo fmt --all
CARGO_TARGET_DIR=/tmp/csl-v25-hash-target cargo test --locked -p compact_collections
```

Formatting passed. All 54 package tests passed: 12 unit tests and 42
integration tests. The architecture classifier test compared the new
empty-only classifier with its scalar reference over all 256 byte values and
random control groups. The map tests covered wrapping, partial groups, and
invalid byte values. Miri and the full workspace suite were not run because the
candidate failed the focused workload gate.

## A/B method and timings

The pinned baseline binaries came from
`/tmp/csl-v25-targeted-baseline`, built at the baseline source commit with
telemetry-free release settings. The candidate used the same release settings.
The host was Linux AArch64, Neoverse-N1, 2 vCPUs. Each scenario had nine
baseline/candidate pairs; baseline-first and candidate-first pair order
alternated, as did native/compact order. Each driver run used `--seconds 0`,
the scenario's standard repetition count (A5 15, B5 7, B8 9), and verified
matching native/compact checksums. All 54 driver runs passed checksum parity;
baseline and candidate checksums also matched:

| Scenario | Checksum |
| --- | ---: |
| A5 | 17351022467741104803 |
| B5 | 7959229199864024003 |
| B8 | 10141254246637991735 |

Timings are median and nearest-rank p95 nanoseconds per scenario repetition.
The harness used the accounting-free `benchmark_profile` binary.

| Scenario | Variant | Baseline median / p95 (ns) | Candidate median / p95 (ns) | Median change |
| --- | --- | ---: | ---: | ---: |
| A5 | Compact | 3,441,796 / 3,724,601 | 3,402,500 / 3,457,796 | −1.14% |
| A5 | Native | 1,316,655 / 1,501,574 | 1,308,119 / 1,509,958 | −0.65% |
| B5 | Compact | 7,623,405 / 8,112,129 | 7,600,799 / 8,182,506 | −0.30% |
| B5 | Native | 10,071,949 / 10,549,530 | 10,218,419 / 12,429,155 | +1.45% |
| B8 | Compact | 28,562,612 / 31,137,404 | 28,440,064 / 32,405,747 | −0.43% |
| B8 | Native | 13,202,307 / 14,118,033 | 13,220,467 / 13,759,004 | +0.14% |

Raw paired samples and logs are in `/tmp/csl-v25-hash-ab/raw/`; aggregate
medians and p95 values are in `/tmp/csl-v25-hash-ab/summary.tsv`. The baseline
profile binary SHA-256 is
`d9d596aa07bd47ded1707d087dcd8c4f6973697b54fbb6c38a634ccb7863d612`; the
candidate binary SHA-256 is
`5d12ec622fe8e0339a7ea53bd7067fe33313ccd0531a32bd457161af0cf0fd61`.

## Hardware counters

`sudo -n perf stat --no-big-num -x, --repeat 3` counted the compact
`benchmark_profile --window <scenario> compact <standard-repetitions>
--seconds 0` process. These are whole-process counts, including startup and
post-window integrity checks; treat them as diagnostics, not phase-only counts.

| Scenario | Event | Baseline → candidate | Change |
| --- | --- | ---: | ---: |
| A5 | Cycles | 154,155,721 → 154,196,987 | +0.03% |
| A5 | Instructions | 348,575,735 → 348,599,876 | +0.01% |
| A5 | Branches | 56,262,326 → 56,263,589 | +0.00% |
| A5 | Branch misses | 1,148,903 → 1,150,934 | +0.18% |
| A5 | Cache misses | 1,529,671 → 1,532,613 | +0.19% |
| B5 | Cycles | 160,715,749 → 161,238,170 | +0.33% |
| B5 | Instructions | 290,224,993 → 290,387,857 | +0.06% |
| B5 | Branches | 49,536,743 → 49,546,911 | +0.02% |
| B5 | Branch misses | 630,964 → 647,042 | +2.55% |
| B5 | Cache misses | 2,374,017 → 2,364,651 | −0.39% |
| B8 | Cycles | 765,378,684 → 754,653,143 | −1.40% |
| B8 | Instructions | 1,499,912,506 → 1,478,437,120 | −1.43% |
| B8 | Branches | 281,707,331 → 280,073,889 | −0.58% |
| B8 | Branch misses | 3,139,642 → 3,224,235 | +2.69% |
| B8 | Cache misses | 14,608,529 → 14,440,296 | −1.15% |

Counter artifacts are in `/tmp/csl-v25-hash-ab/counters/`.

## Retained memory and RSS

Allocator-accounted `benchmark_compare --mode measure` runs are used only for
memory here; their elapsed times are not used in the A/B decision. The
candidate and pinned baseline had matching checksums.

| Scenario | Live after build, baseline → candidate (B) | Live after mutation, baseline → candidate (B) | High-water, baseline → candidate (B) | Free bytes / blocks / largest, baseline → candidate (B) | Peak RSS, baseline → candidate (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| A5 | 720,960 → 720,960 | 720,960 → 720,960 | 720,968 → 720,968 | 0 / 0 / 0 → 0 / 0 / 0 | 3,472 → 3,472 |
| B5 | 3,440,672 → 3,440,672 | 3,440,672 → 3,440,672 | 3,440,680 → 3,440,680 | 0 / 0 / 0 → 0 / 0 / 0 | 11,088 → 11,088 |
| B8 | 3,620,248 → 3,620,248 | 3,620,360 → 3,620,360 | 4,424,784 → 4,424,784 | 804,416 / 2 / 802,848 → 804,416 / 2 / 802,848 | 35,204 → 35,208 |

The 4 KiB B8 RSS increase is about 0.01%; live retained bytes, high-water,
free-space shape, and largest free block were unchanged for all three
scenarios. Baseline accounting is in
`/tmp/csl-v25-targeted-baseline/runs/v25-targeted-baseline/measure-stats.tsv`;
candidate accounting is in
`/tmp/csl-v25-hash-ab/candidate-measure-stats.tsv`.

## Reproduction commands

The pinned telemetry-free baseline executable was
`/tmp/csl-v25-targeted-baseline/target-plain/release/examples/benchmark_profile-574d1082e0027351`.
The candidate executable was built with:

```sh
env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
  CARGO_TARGET_DIR=/tmp/csl-v25-hash-target-plain \
  cargo build --locked --release --no-default-features -p compact_std \
  --features json,toml --example benchmark_profile
```

Each paired driver command used this shape, with each binary run once per pair
in alternating baseline/candidate order and `ORDER` alternating between
`alternate` and `alternate-reversed`:

```sh
<binary> --seconds 0 --order <ORDER> --scenario <A5|B5|B8> --output <run.tsv>
```

Counters used the command below for each scenario and binary; the pinned
baseline and candidate paths replace `<binary>`:

```sh
sudo -n perf stat --no-big-num -x, --repeat 3 \
  -e cycles,instructions,branches,branch-misses,cache-misses -o <events.csv> -- \
  <binary> --window <A5|B5|B8> compact <15|7|9> --seconds 0
```

The candidate memory binary was built with:

```sh
env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
  CARGO_TARGET_DIR=/tmp/csl-v25-hash-target-plain \
  cargo build --locked --release --no-default-features -p compact_std \
  --features json,toml --example benchmark_compare
```

Memory used the pinned baseline `benchmark_compare` and candidate release binary
with:

```sh
<binary> --scenario A5 --scenario B5 --scenario B8 --order alternate \
  --output <memory.tsv>
```
