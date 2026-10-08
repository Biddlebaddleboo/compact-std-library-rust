# V2.4 pre-optimization native Rust comparison

Benchmark report for the frozen V2.4 architecture. The measured source revision is
`1700bdc4d1cb8f242fdb2e565ab64fb1af324f27` (benchmark implementation commit).
Two complete release suites were run on 2026-10-07; all 16 native/compact pairs
produced matching checksums in both suites.

## Method

The authoritative suite is `crates/compact_std/examples/benchmark_compare/`.
It compares ordinary Rust `std` models against V2.4 cage-backed models. The
orchestrator runs each scenario and variant in a fresh child process, in fixed
scenario order, native first. Each child gets one untimed warm-up and then the
measured repetitions below. Inputs are deterministic; JSON and TOML payloads
are prepared before timed parse/build phases. The same Serde ecosystem versions
are locked for both sides: Serde 1.0.229, `serde_json` 1.0.151, and `toml`
0.8.23.

| Scenario group | Measured repetitions per child |
| --- | ---: |
| A1–A6 container diagnostics | 15 |
| B1–B4, B6, B8 application workloads | 9 |
| B5, B7, B9, B10 larger workloads | 7 |

Each timed phase reports median and p95. The optional TSV output also records
minimum, maximum, run count, allocator calls, requested bytes, live-byte delta,
and cage diagnostics. `end_to_end` is the sum of that repetition’s timed build,
mutation, read, and drop intervals; it excludes the per-phase measurement and
allocator-stat snapshot overhead.

Native memory is requested `System` allocator bytes, excluding malloc metadata.
Compact retained memory is cage live bytes (including cage headers and padding)
plus any retained native auxiliary bytes. Auxiliary requested bytes are
cumulative during build; auxiliary peak is maximum additional live native bytes.
The retained-memory ratio is `(cage live + auxiliary live) / native live`.
The compact runtime is initialized with a 128 MiB cage before measurement; the
configured reservation is not counted as per-workload live memory. Generated
fixtures and serialized input are also prepared outside allocation accounting.
Linux `VmHWM` is collected as supplemental process-level RSS, not as the
canonical retained-memory measure.

## Environment

- Ubuntu 24.04.4 LTS; Linux `6.17.0-1020-oracle`
- AArch64, ARM Neoverse-N1, 2 logical CPUs
- `rustc 1.95.0 (59807616e 2026-04-14)`; Cargo 1.95.0
- Release profile, default target flags; benchmark crate features `json,toml`
- Source revision: `1700bdc4d1cb8f242fdb2e565ab64fb1af324f27`
- Repetition counts above; two full suite runs on the same host/toolchain

Run with:

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-benchmark.tsv
```

## A1–A6 microbenchmark summary

These are diagnostics rather than headline application results. Times show the
first suite’s end-to-end median/p95; the ratio column gives suite 1 / suite 2.
Ratios near zero-time native phases are especially sensitive to timer resolution.

| ID | Workload | Native median / p95 (ms) | V2.4 median / p95 (ms) | Time ratio, run 1 / run 2 | Retained memory ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| A1 | vector build/traverse/churn | 0.287 / 0.441 | 16.879 / 17.633 | 58.79x / 58.64x | 1.311x |
| A2 | boxed objects | 1.723 / 1.909 | 4.679 / 5.181 | 2.72x / 2.69x | 1.500x |
| A3 | strings and byte lengths | 4.216 / 4.736 | 0.667 / 0.690 | 0.16x / 0.16x | 1.001x |
| A4 | FIFO deque churn | 0.359 / 0.393 | 4.504 / 4.969 | 12.54x / 12.58x | 1.000x |
| A5 | hash map and set churn | 1.201 / 1.295 | 9.372 / 9.452 | 7.80x / 9.30x | 1.000x |
| A6 | path build and query | 1.173 / 1.206 | 2.135 / 2.627 | 1.82x / 1.83x | 1.233x |

## B1–B10 application summary

The time ratio is compact/native; values below 1.0 mean V2.4 was faster.
Memory uses the retained-memory definition above. Run 1 medians and p95s are
shown; both full suites produced matching checksums.

| ID | Workload | Native end-to-end median / p95 (ms) | V2.4 end-to-end median / p95 (ms) | Time ratio, run 1 / run 2 | Retained memory ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| B1 | 10k-record JSON API response | 13.278 / 14.108 | 21.971 / 23.022 | 1.65x / 1.67x | 0.618x |
| B2 | TOML service configuration | 0.965 / 1.023 | 1.344 / 1.966 | 1.39x / 1.37x | 0.802x |
| B3 | HTTP/request metadata batch | 28.183 / 33.349 | 35.706 / 36.382 | 1.27x / 1.28x | 0.911x |
| B4 | Bounded logging history | 6.401 / 6.904 | 10.549 / 11.016 | 1.65x / 1.60x | 0.860x |
| B5 | Mobility/dispatch state | 8.834 / 11.232 | 9.496 / 10.082 | 1.07x / 1.07x | 0.710x |
| B6 | Market-data order book | 0.040 / 0.068 | 1.081 / 1.145 | 27.25x / 27.33x | 1.000x |
| B7 | 100k filesystem catalog | 14.780 / 19.700 | 28.597 / 28.890 | 1.93x / 1.86x | 1.050x |
| B8 | Fixed-population cache churn | 11.865 / 12.778 | 89.125 / 90.271 | 7.51x / 7.08x | 1.026x |
| B9 | Immutable/frozen catalog | 10.991 / 12.859 | 7.011 / 7.940 | 0.64x / 0.63x | 0.739x |
| B10 | Concurrent worker state | 1.275 / 1.415 | 3.578 / 4.231 | 2.81x / 2.59x | 1.000x |

### Application phase details

Phase medians and p95s are in milliseconds. This highlights build, mutation,
traversal/query, and drop costs without pasting the full TSV output.

| ID | Phase | Native median / p95 (ms) | V2.4 median / p95 (ms) | V2.4/native | Runs |
| --- | --- | ---: | ---: | ---: | ---: |
| B1 | build | 10.778 / 11.576 | 19.149 / 20.175 | 1.78x | 9 |
| B1 | selected lookup | 0.005 / 0.006 | 0.010 / 0.011 | 1.94x | 9 |
| B1 | traverse | 0.216 / 0.249 | 0.404 / 0.461 | 1.87x | 9 |
| B1 | drop | 2.287 / 2.340 | 2.391 / 2.448 | 1.05x | 9 |
| B2 | build | 0.919 / 0.976 | 1.068 / 1.697 | 1.16x | 9 |
| B2 | repeated reads | 0.027 / 0.028 | 0.203 / 0.210 | 7.46x | 9 |
| B2 | drop | 0.019 / 0.020 | 0.065 / 0.110 | 3.34x | 9 |
| B3 | build | 17.333 / 22.523 | 21.854 / 22.393 | 1.26x | 9 |
| B3 | selected lookups | 0.025 / 0.030 | 0.062 / 0.577 | 2.48x | 9 |
| B3 | status scan | 0.197 / 0.243 | 0.145 / 0.190 | 0.74x | 9 |
| B3 | drop | 10.470 / 11.589 | 13.497 / 13.996 | 1.29x | 9 |
| B4 | build | 0.519 / 0.756 | 0.581 / 1.082 | 1.12x | 9 |
| B4 | retained scan | 0.010 / 0.011 | 0.167 / 0.187 | 16.02x | 9 |
| B4 | steady state mutation | 5.554 / 5.773 | 9.342 / 9.891 | 1.68x | 9 |
| B4 | drop | 0.316 / 0.364 | 0.385 / 0.417 | 1.22x | 9 |
| B5 | build | 4.203 / 6.590 | 6.512 / 7.118 | 1.55x | 7 |
| B5 | status and expiry updates | 1.064 / 1.179 | 1.670 / 1.691 | 1.57x | 7 |
| B5 | zone status and id queries | 0.136 / 0.152 | 0.178 / 0.201 | 1.30x | 7 |
| B5 | drop | 3.429 / 3.502 | 1.114 / 1.208 | 0.32x | 7 |
| B6 | build | 0.002 / 0.030 | 0.088 / 0.093 | 51.16x | 9 |
| B6 | best price and depth | 0.001 / 0.001 | 0.001 / 0.001 | 1.00x | 9 |
| B6 | quote updates and snapshot rebuilds | 0.034 / 0.036 | 0.913 / 0.935 | 26.86x | 9 |
| B6 | snapshot copy | 0.003 / 0.005 | 0.078 / 0.130 | 30.58x | 9 |
| B6 | drop | 0.000 / 0.000 | 0.000 / 0.000 | 1.33x | 9 |
| B7 | build | 9.120 / 13.608 | 17.994 / 18.359 | 1.97x | 7 |
| B7 | path queries and category scan | 0.991 / 1.239 | 2.225 / 2.441 | 2.24x | 7 |
| B7 | subset updates | 0.052 / 0.060 | 0.498 / 0.537 | 9.50x | 7 |
| B7 | drop | 4.596 / 4.793 | 7.834 / 7.911 | 1.70x | 7 |
| B8 | build | 0.859 / 1.094 | 2.829 / 2.901 | 3.29x | 9 |
| B8 | cache lookup and scan | 0.070 / 0.073 | 0.343 / 0.360 | 4.90x | 9 |
| B8 | fixed population churn | 10.369 / 11.014 | 35.093 / 36.980 | 3.38x | 9 |
| B8 | drop | 0.582 / 0.665 | 50.769 / 51.693 | 87.19x | 9 |
| B9 | build | 6.379 / 7.855 | 1.776 / 2.812 | 0.28x | 7 |
| B9 | deterministic random lookup | 0.283 / 0.305 | 1.163 / 1.336 | 4.11x | 7 |
| B9 | parallel read traversal | 0.385 / 0.500 | 1.455 / 1.496 | 3.78x | 7 |
| B9 | sequential traversal | 0.197 / 0.245 | 2.611 / 2.639 | 13.28x | 7 |
| B9 | drop | 3.721 / 4.001 | 0.001 / 0.001 | 0.00x | 7 |
| B10 | build | 0.084 / 0.222 | 0.417 / 0.999 | 4.99x | 7 |
| B10 | parallel allocate drop churn | 0.964 / 1.244 | 3.044 / 3.151 | 3.16x | 7 |
| B10 | parallel traversal | 0.100 / 0.265 | 0.081 / 0.106 | 0.81x | 7 |
| B10 | drop | 0.001 / 0.041 | 0.001 / 0.001 | 0.78x | 7 |

## Retained memory and auxiliary allocation results

Bytes are measured during build. Native columns show retained/requested bytes.
V2.4 shows cage live bytes. Auxiliary columns show retained/requested/peak
native bytes. All compact auxiliary retained values were 0 except B10 (16 B);
parser and frozen-builder temporary allocations are visible in requested/peak.

| ID | Native retained / requested | V2.4 cage live | V2.4 auxiliary live / requested / peak | Retained ratio |
| --- | ---: | ---: | ---: | ---: |
| A1 | 400,000 / 400,000 B | 524,304 B | 0 / 0 / 0 B | 1.311x |
| A2 | 600,000 / 600,000 B | 900,016 B | 0 / 0 / 0 B | 1.500x |
| A3 | 13,361,600 / 13,361,600 B | 13,368,816 B | 0 / 0 / 0 B | 1.001x |
| A4 | 32,768 / 32,768 B | 32,784 B | 0 / 0 / 0 B | 1.000x |
| A5 | 720,912 / 720,912 B | 720,960 B | 0 / 0 / 0 B | 1.000x |
| A6 | 428,180 / 428,180 B | 528,016 B | 0 / 0 / 0 B | 1.233x |
| B1 | 4,360,722 / 7,112,562 B | 2,692,880 B | 0 / 630,007 / 23 B | 0.618x |
| B2 | 56,629 / 1,354,902 B | 45,424 B | 0 / 1,287,272 / 536,137 B | 0.802x |
| B3 | 13,071,260 / 13,071,260 B | 11,903,936 B | 0 / 0 / 0 B | 0.911x |
| B4 | 565,270 / 565,270 B | 486,112 B | 0 / 0 / 0 B | 0.860x |
| B5 | 4,846,280 / 4,846,280 B | 3,440,672 B | 0 / 0 / 0 B | 0.710x |
| B6 | 49,169 / 49,169 B | 49,184 B | 0 / 0 / 0 B | 1.000x |
| B7 | 14,459,152 / 14,459,152 B | 15,186,952 B | 0 / 0 / 0 B | 1.050x |
| B8 | 3,528,488 / 3,528,488 B | 3,620,248 B | 0 / 0 / 0 B | 1.026x |
| B9 | 5,193,776 / 5,193,776 B | 3,840,072 B | 0 / 11,765,728 / 6,522,880 B | 0.739x |
| B10 | 256,048 / 256,328 B | 256,032 B | 16 / 344 / 328 B | 1.000x |

For the headline workloads, V2.4 retained 38% less memory in B1, 20% less in
B2, 9% less in B3, 14% less in B4, 29% less in B5, and 26% less in B9.
B7 retained about 5% more and B8 about 3% more; B6 and B10 were effectively
neutral. B2 requested 1.29 MB of native auxiliary allocations during parse,
with a 536 KB peak and no auxiliary bytes retained after build. B9’s frozen
builder requested 11.77 MB of temporary native allocations, peaked at 6.52 MB,
and retained none after freeze.

## B8 fixed-population churn

B8 held 8,192 entries through 64 remove/insert cycles, replacing one eighth of
the population per cycle. The compact map stayed at capacity 16,384. Its
observed high-water cursor moved between 3,621,936 B and 4,424,784 B in a
repeating pattern; it contracted from the higher level at cycles 23 and 46,
and the final cycle remained at 4,424,784 B. At cycle 64, 804,416 B was free
in two blocks, with 802,848 B in the largest block. The free range therefore
remained mostly coalesced and the cursor did not grow monotonically across the
fixed-population run.

Compact B8’s fixed-population mutation phase was 35.1 ms versus 10.4 ms native.
Final compact teardown was 50.8 ms versus 0.58 ms native, making the total
89.1 ms versus 11.9 ms. Inspection ties the teardown cost to dropping each
occupied map entry; larger `CompactBytes` payloads own separate cage blocks
whose release takes the allocator lock. The tested memory stayed bounded, so no
allocator redesign was made; this remains a substantial throughput tradeoff.

## Frozen catalog

B9 used 40,000 immutable records with names, categories, five related IDs, and
flags. The conventional native model used owned `String` and `Vec` fields;
V2.4 used `FrozenBuilder` and one `FrozenGraph`. V2.4 construction/freeze was
0.28x native time and end-to-end was 0.64x, while sequential traversal was
13.28x slower, deterministic random lookup 4.11x slower, and parallel read
traversal 3.78x slower. The final retained memory ratio was 0.739x. The
compact builder’s temporary native allocation peak is reported separately above.

## RSS and interpretation

Each Linux child reported `/proc/self/status` `VmHWM`. Selected run 1 peaks:

| Scenario | Native peak RSS | V2.4 peak RSS |
| --- | ---: | ---: |
| B1 | 10,516 KiB | 7,284 KiB |
| B3 | 42,540 KiB | 34,548 KiB |
| B7 | 41,476 KiB | 38,400 KiB |
| B8 | 35,688 KiB | 35,884 KiB |
| B9 | 16,948 KiB | 18,964 KiB |

RSS includes the prepared fixture and parser/build temporaries, so it does not
measure only the retained model. For example, B9’s compact peak RSS was higher
even though its final retained cage data was smaller; the retained-byte ratio
and process RSS answer different questions.

The two suite runs gave similar end-to-end ratios for most workloads. A5 varied
from 7.80x to 9.30x and B10 from 2.59x to 2.81x; randomized hash layout and
thread scheduling are plausible contributors. Absolute timings are specific to
this two-core host. Near-zero native phases can produce very large ratios with
little absolute time impact. B6 is a visible example: about 1.08 ms compact
versus 0.040 ms native, dominated by quote updates/snapshot copies; the same
loop structure and checksums were used on both sides. No correctness defect or
memory-growth pathology was found, so this was documented rather than used to
justify an architecture redesign.

Other useful tradeoffs: A3’s mixed-length string/byte workload was about 6x
faster end-to-end on V2.4 with effectively equal retained memory. B5 was near
CPU-neutral end-to-end with 29% lower retained memory. B1/B3/B4 save memory at
moderate CPU cost. B7 is slower and slightly larger. B8’s teardown and B9’s
repeated graph reads are the clearest workloads where native Rust wins.

## Limitations

- One ARM host with two logical CPUs; results do not predict other machines.
- Each scenario/variant receives one warm-up and 7–15 measured repetitions per
  suite; two complete suites expose some run-to-run variability but are not a
  multi-host benchmark campaign.
- Requested `System` bytes omit allocator metadata; cage bytes include its
  headers and padding. Neither value is identical to RSS.
- Prepared fixtures and serialized payloads are outside timed build allocation
  counters. Their process memory remains visible in supplemental RSS.
- The 128 MiB cage capacity is a configured reservation, not counted as live
  per-scenario data. OS residency depends on pages actually touched.
- Randomized hashers and thread scheduling can affect timing and iteration
  order; checksums are order-independent and matched for every pair.
- Container microbenchmarks are diagnostic. A high ratio on a sub-microsecond
  native phase should not be read as a large application-level cost.

## Validation and V2.4 freeze

Passed on the benchmark source revision:

```text
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo +nightly miri test -p compact_core
cargo +nightly miri test -p compact_collections --test cage_collections
cargo +nightly miri test -p compact_backend_std --test integration
cargo +nightly miri test -p compact_backend_std --lib
cargo +nightly miri test -p compact_std --all-features --test v2_4
```

The only performance adjustment made in response to measurements was a
`needs_drop` fast path for truncating element buffers with no destructor. It
avoids per-element no-op drop work for byte and scalar buffers, while preserving
initialized-length updates, allocation layout, and destructor behavior. A
separate doc-hidden allocator snapshot adds no retained owner or header fields
and does not change allocation policy.

**V2.4 is frozen after this benchmark and validation pass.** Further memory
model, pointer model, owner/header, or allocator changes that alter the retained
contract belong in V3. Correctness fixes, documentation corrections, test
improvements, benchmark maintenance, and bounded global allocator-policy
optimizations that preserve retained layouts remain allowed. The later passes
recorded below preserve that contract.

# V2.4 optimized hot-path results

Optimization commit: `0d7214810d40e48d2865c1a7357389f908040d68`, based on the
verified `main` baseline `585f73e1abb02b7818719c4c622720c2a6af512a`. This is an
implementation optimization within the frozen V2.4 retained representation.

## Environment and method

The authoritative `benchmark_compare` release suite ran twice on 2026-10-07 on
the same ARM64 host and toolchain:

- Ubuntu 24.04.4; Linux `6.17.0-1020-oracle`; AArch64 ARM Neoverse-N1, 2 CPUs
- `rustc 1.95.0 (59807616e 2026-04-14)`; LLVM 22.1.2
- Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`; release profile, default target flags
- Features `json,toml`; default 7–15 repetitions per scenario and variant

The command was:

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-optimized-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-optimized-run2.tsv
```

All native/compact checksums matched in both runs for all 16 scenarios. Times
below are end-to-end medians and p95s from run 1; the ratio column gives
optimized V2.4/native for runs 1 and 2. Retained memory uses the same definition
as the pre-optimization report: compact cage live bytes plus retained native
auxiliary bytes, divided by native live bytes.

## Primary result: native Rust vs optimized V2.4

| ID | Workload | Native median / p95 (ms) | Optimized V2.4 median / p95 (ms) | Time ratio, run 1 / run 2 | Retained memory ratio |
|---|---|---:|---:|---:|---:|
| A1 | vector build/traverse/churn | 0.067 / 0.175 | 0.071 / 0.086 | 1.07x / 1.06x | 1.000x |
| A2 | box and object allocation | 1.731 / 1.773 | 2.730 / 3.091 | 1.58x / 1.58x | 1.500x |
| A3 | string and byte lengths | 4.719 / 5.467 | 0.614 / 0.683 | 0.13x / 0.14x | 1.001x |
| A4 | FIFO deque churn | 0.363 / 0.394 | 2.727 / 3.243 | 7.51x / 7.58x | 1.000x |
| A5 | hash map and set churn | 1.193 / 1.315 | 5.647 / 6.113 | 4.73x / 4.88x | 1.000x |
| A6 | path build and query | 1.178 / 1.338 | 1.535 / 1.591 | 1.30x / 1.27x | 1.233x |
| B1 | 10k record JSON API response | 13.444 / 15.025 | 20.006 / 20.507 | 1.49x / 1.39x | 0.618x |
| B2 | large TOML service configuration | 0.984 / 1.013 | 1.266 / 1.315 | 1.29x / 1.30x | 0.802x |
| B3 | 24k request metadata batch | 28.303 / 34.681 | 22.880 / 23.498 | 0.81x / 0.75x | 0.911x |
| B4 | bounded logging history churn | 6.416 / 6.785 | 7.949 / 8.473 | 1.24x / 1.25x | 0.860x |
| B5 | mobility dispatch state | 8.888 / 11.505 | 7.296 / 7.896 | 0.82x / 0.79x | 0.710x |
| B6 | market data order book | 0.038 / 0.042 | 0.600 / 0.634 | 15.70x / 15.46x | 1.000x |
| B7 | 100k filesystem catalog | 17.455 / 22.582 | 17.586 / 18.092 | 1.01x / 1.21x | 1.050x |
| B8 | fixed population cache churn | 12.030 / 13.533 | 83.193 / 84.033 | 6.92x / 6.96x | 1.026x |
| B9 | immutable/frozen catalog | 12.670 / 14.171 | 4.545 / 5.905 | 0.36x / 0.44x | 0.739x |
| B10 | concurrent worker state | 0.694 / 1.785 | 1.028 / 1.173 | 1.48x / 2.18x | 1.000x |

A1 now uses matched bulk operations: native `collect`/`extend` and compact
`try_from_iter`/`try_extend`. The earlier published A1 measurement used
per-element pushes, so its pre/post number is not a like-for-like speedup. Both
versions still build and mutate the same element counts and produce equal
checksums.

## Secondary result: published V2.4 vs optimized V2.4

This table measures the implementation optimization, not the native comparison.
The previous published V2.4 medians are preserved from the pre-optimization
report above. The optimized column is run 1 from the new suite.

| ID | Published pre-optimization V2.4 median (ms) | Optimized V2.4 median / p95 (ms) | Old / new median |
|---|---:|---:|---:|
| A1 | 16.879 | 0.071 / 0.086 | not directly comparable† |
| A2 | 4.679 | 2.730 / 3.091 | 1.71x |
| A3 | 0.667 | 0.614 / 0.683 | 1.09x |
| A4 | 4.504 | 2.727 / 3.243 | 1.65x |
| A5 | 9.372 | 5.647 / 6.113 | 1.66x |
| A6 | 2.135 | 1.535 / 1.591 | 1.39x |
| B1 | 21.971 | 20.006 / 20.507 | 1.10x |
| B2 | 1.344 | 1.266 / 1.315 | 1.06x |
| B3 | 35.706 | 22.880 / 23.498 | 1.56x |
| B4 | 10.549 | 7.949 / 8.473 | 1.33x |
| B5 | 9.496 | 7.296 / 7.896 | 1.30x |
| B6 | 1.081 | 0.600 / 0.634 | 1.80x |
| B7 | 28.597 | 17.586 / 18.092 | 1.63x |
| B8 | 89.125 | 83.193 / 84.033 | 1.07x |
| B9 | 7.011 | 4.545 / 5.905 | 1.54x |
| B10 | 3.578 | 1.028 / 1.173 | 3.48x |

† A1 changed from per-element pushes to matched bulk construction and mutation
on both native and compact sides. Its new result demonstrates the optimized
bulk path against native bulk operations; the old/new ratio mixes methodology
and implementation changes, so it is omitted.

For the main B9 traversal phases, the old and optimized compact medians/p95s
were:

| B9 phase | Published V2.4 median / p95 (ms) | Optimized run 1 median / p95 (ms) | Optimized run 2 median / p95 (ms) |
|---|---:|---:|---:|
| Sequential traversal | 2.611 / 2.639 | 1.231 / 1.283 | 1.233 / 1.276 |
| Deterministic random lookup | 1.163 / 1.336 | 0.827 / 0.853 | 0.877 / 1.200 |
| Parallel read traversal | 1.455 / 1.496 | 0.751 / 0.811 | 0.761 / 0.831 |

## Implementation and remaining costs

The V2.4 retained layouts remain unchanged, including the 4-byte allocation
owner, 4-byte vector, 16-byte allocation header, and 8-byte frozen descriptors.
Temporary resolved views are scoped by Rust borrows. Bulk append publishes its
initialized prefix even if an iterator, deserializer, or element constructor
panics.

Hash probing uses copied 16-byte control groups with AArch64 NEON on this
benchmark host and SSE2 on x86-64; the scalar classifier remains the semantic
reference. Release disassembly contains NEON lane compares. The x86-64 SSE2
path and differential test targets cross-compile for `x86_64-apple-darwin`; it
was not runtime-benchmarked. Intrinsics generated the required vector compares,
so inline assembly was not needed.

B5 retained memory stayed at 0.710x native, matching the previous report. B8
retained memory stayed at 1.026x native. Its compact drop phase remains about
49.5–49.8 ms versus 0.57–0.60 ms native, so per-entry cage release is still the
dominant residual cost. This pass did not change allocator release policy.

## Validation

Passed after the implementation changes:

```text
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
```

Miri passed `compact_core`, `compact_backend_std` unit and integration tests,
`compact_collections/tests/cage_collections.rs`, and
`compact_std/tests/v2_4.rs`. The collections property test used
`PROPTEST_CASES=8` with Miri isolation disabled because `proptest` accesses the
working directory; all 15 tests passed. Release assembly was inspected for the
AArch64 NEON classifier, the A4 deque scan, and the B9 frozen view traversal.

## V2.4 allocator and teardown results

The allocator pass was measured against the verified pre-pass revision
`1743dfead3e4db1ba82fb374d224644072f7e061`. Both worktrees used the same host
and toolchain listed above. The full `benchmark_compare` release suite ran
twice at the pinned baseline and twice again after the final allocator changes,
using features `json,toml` and no telemetry in the timed runs. All native and
compact checksums matched in all 16 scenarios on every run.

Commands in the baseline and implementation worktrees:

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-base-1743-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-base-1743-run2.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-final-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-final-run2.tsv
```

The table shows final implementation medians and p95s from run 1; the ratio is
compact/native for runs 1 and 2. Memory ratio is retained compact cage plus
auxiliary bytes divided by native retained live bytes.

| ID | Workload | Native median / p95 (ms) | Allocator V2.4 median / p95 (ms) | Time ratio, run 1 / run 2 | Retained memory ratio |
|---|---|---:|---:|---:|---:|
| A1 | vector build/traverse/churn | 0.067 / 0.173 | 0.072 / 0.114 | 1.07x / 1.07x | 1.000x |
| A2 | box and object allocation | 1.747 / 2.237 | 2.551 / 2.626 | 1.46x / 1.45x | 1.500x |
| A3 | string and byte lengths | 4.260 / 5.127 | 0.636 / 0.941 | 0.15x / 0.15x | 1.001x |
| A4 | FIFO deque churn | 0.354 / 0.370 | 2.674 / 2.791 | 7.56x / 7.46x | 1.000x |
| A5 | hash map and set churn | 1.163 / 2.126 | 5.632 / 5.841 | 4.84x / 4.79x | 1.000x |
| A6 | path build and query | 1.153 / 1.171 | 1.463 / 1.614 | 1.27x / 1.27x | 1.233x |
| B1 | 10k record JSON API response | 13.754 / 14.367 | 18.385 / 20.522 | 1.34x / 1.34x | 0.618x |
| B2 | large TOML service configuration | 0.971 / 0.990 | 1.272 / 1.468 | 1.31x / 1.26x | 0.802x |
| B3 | 24k request metadata batch | 33.375 / 36.203 | 22.940 / 23.772 | 0.69x / 0.70x | 0.911x |
| B4 | bounded logging history churn | 6.477 / 6.887 | 8.408 / 8.572 | 1.30x / 1.28x | 0.860x |
| B5 | mobility dispatch state | 8.812 / 10.005 | 7.268 / 7.543 | 0.82x / 0.81x | 0.710x |
| B6 | market data order book | 0.038 / 0.045 | 0.593 / 0.719 | 15.47x / 15.81x | 1.000x |
| B7 | 100k filesystem catalog | 19.989 / 21.692 | 16.691 / 17.030 | 0.84x / 0.83x | 1.050x |
| B8 | fixed population cache churn | 11.821 / 12.604 | 36.547 / 37.204 | 3.09x / 3.04x | 1.026x |
| B9 | immutable/frozen catalog | 10.895 / 12.267 | 4.420 / 5.581 | 0.41x / 0.42x | 0.739x |
| B10 | concurrent worker state | 1.262 / 1.296 | 2.931 / 3.631 | 2.32x / 2.29x | 1.000x |

The pinned-baseline and final medians for the priority teardown and concurrency
scenarios were:

| Scenario phase | `1743dfe` run 1 / 2 (ms) | Allocator run 1 / 2 (ms) | Old / new |
|---|---:|---:|---:|
| B3 drop | 6.990 / 7.014 | 6.356 / 6.753 | 1.10x / 1.04x |
| B3 end-to-end | 22.780 / 22.655 | 22.940 / 23.353 | 0.99x / 0.97x |
| B5 drop | 0.197 / 0.196 | 0.125 / 0.125 | 1.57x / 1.57x |
| B5 end-to-end | 7.332 / 7.283 | 7.268 / 7.206 | 1.01x / 1.01x |
| B8 drop | 49.732 / 50.123 | 3.133 / 3.141 | 15.88x / 15.96x |
| B8 end-to-end | 82.439 / 83.110 | 36.547 / 37.007 | 2.26x / 2.25x |
| B10 end-to-end | 2.980 / 3.188 | 2.931 / 2.853 | 1.02x / 1.12x |

B5 retained memory stayed at 0.710x native and B8 at 1.026x, unchanged from the
pinned baseline. B3 teardown improved by 4–10%; end-to-end time was 1–3% slower
than baseline. B10's raw end-to-end medians stayed within the pinned baseline
range; its concurrent phase has substantial host scheduling variance.

With `allocator-telemetry` enabled, B8's drop phase reduced mutex acquisitions
from 6,556 to 103 and ordered free-list visits from 3,579,404 to 319,406. The
new run processed 6,556 extents in 103 batches, with a maximum batch of 64.
Instrumentation was excluded from timing. The measured exact classes are 32,
40, 112, and 528 bytes, with 32 cached blocks per class. In the instrumented B8
process, allocation recorded 8 class hits and 486,599 misses; its teardown gain
therefore comes primarily from batched release and coalescing rather than
class reuse. The caches remain global, bounded metadata and are drained before
merging or resizing.

Exact retained sizes remain unchanged: the allocation owner and vector are
4 B, the common header is 16 B, and frozen descriptors are 8 B. This pass adds
no architecture-specific intrinsics or inline assembly.

## V2.4 recycling and collection hot paths

Implementation source revision: `f9c0e0b`, integrated on top of the pinned
`8698490347a5e798df4a6a3ab5f516ce3338711a` baseline. Two full release suites
ran on 2026-10-07 with the same host and Rust toolchain listed above. They used
features `json,toml` without telemetry and the production allocator policy:
pending exact reuse enabled, global size-class lookups disabled. All native and
compact logical checksums matched across all 16 scenarios in both runs.

The full suite commands were:

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-implemented-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/compact-v24-implemented-run2.tsv
```

For each timing cell below, `run 1; run 2` are `median/p95` milliseconds.
The pinned compact values come from the two complete suites at `8698490`;
retained memory is compact cage plus auxiliary live bytes divided by native
live bytes. The changed scenarios cover the allocator workloads and all three
collection targets.

| ID | Native med/p95 (R1; R2) | Pinned compact med/p95 (R1; R2) | New compact med/p95 (R1; R2) | Old/new median (R1/R2) | New/native median (R1/R2) | Retained memory |
|---|---:|---:|---:|---:|---:|---:|
| A2 | 1.742/1.787; 1.764/2.355 | 2.547/3.037; 2.539/3.385 | 2.611/2.864; 2.585/2.641 | 0.98x/0.98x | 1.50x/1.47x | 1.500x |
| A3 | 4.277/4.874; 4.275/4.891 | 0.638/0.766; 0.612/0.767 | 0.681/0.898; 0.615/0.654 | 0.94x/1.00x | 0.16x/0.14x | 1.001x |
| A4 | 0.366/0.386; 0.359/0.426 | 3.112/3.633; 2.675/3.334 | 2.258/2.707; 2.029/2.646 | 1.38x/1.32x | 6.16x/5.65x | 1.000x |
| A5 | 1.176/1.543; 1.165/1.309 | 5.639/6.079; 5.616/5.829 | 5.001/5.174; 5.038/6.553 | 1.13x/1.11x | 4.25x/4.32x | 1.000x |
| B3 | 32.904/38.237; 32.911/33.838 | 22.949/23.685; 23.148/24.364 | 23.409/25.035; 24.424/24.576 | 0.98x/0.95x | 0.71x/0.74x | 0.911x |
| B5 | 8.886/10.281; 9.245/10.426 | 7.247/8.569; 7.211/7.471 | 6.906/6.982; 6.799/6.943 | 1.05x/1.06x | 0.78x/0.74x | 0.710x |
| B6 | 0.039/0.062; 0.038/0.062 | 0.595/0.638; 0.594/0.616 | 0.142/0.170; 0.141/0.161 | 4.19x/4.20x | 3.68x/3.70x | 1.000x |
| B8 | 11.759/12.932; 11.779/14.124 | 36.661/37.172; 36.706/37.405 | 32.129/32.658; 32.467/34.021 | 1.14x/1.13x | 2.73x/2.76x | 1.026x |
| B10 | 1.172/1.343; 1.184/1.814 | 3.113/3.180; 2.793/3.053 | 2.619/2.854; 2.806/2.992 | 1.19x/1.00x | 2.24x/2.37x | 1.000x |

The targeted hot phases improved as follows. Values are compact medians in
milliseconds, run 1 / run 2; percentages compare each run against its pinned
baseline.

| Scenario phase | Pinned baseline | New implementation | Change |
|---|---:|---:|---:|
| A4 deque mutation | 3.001 / 2.580 | 2.187 / 1.969 | 27% / 24% faster |
| A5 hash mutation | 1.410 / 1.405 | 1.239 / 1.253 | 12% / 11% faster |
| A5 hash lookup | 0.615 / 0.611 | 0.539 / 0.544 | 12% / 11% faster |
| B6 quote updates and snapshot rebuilds | 0.540 / 0.535 | 0.088 / 0.088 | 84% / 84% faster |
| B6 snapshot copy | 0.00428 / 0.00424 | 0.00284 / 0.00280 | 34% / 34% faster |
| B8 fixed-population churn | 31.230 / 31.325 | 27.526 / 27.834 | 12% / 11% faster |
| B8 drop | 3.118 / 3.143 | 2.419 / 2.447 | 22% / 22% faster |

The B6 scenario now calls the public `CompactVec::retain` path and uses
`try_clone_copy` for its `Copy` records. Its end-to-end median fell by about
76% while retained memory stayed unchanged. A4's no-growth ring pushes avoid
the second capacity resolution. A5 classifies physically contiguous control
groups directly and retains bounded scratch for wraparound and partial groups.

The allocator policy comparison used isolated release binaries and no
telemetry in timed runs. Policy A means pending reuse off/classes on; B means
pending reuse on/classes off and is the production default; C means both on.
The `benchmark-allocator-a`, `benchmark-allocator-b`, and
`benchmark-allocator-c` features select those comparison modes; a build with no
policy feature uses B. A build with `--all-features` selects C intentionally.
The B10 values below are the median of two 101-sample passes. B8 values are
the median of three 21-sample passes.

| Policy | B10 churn / end-to-end (ms) | B8 churn / end-to-end (ms) | A3 end-to-end (ms) |
|---|---:|---:|---:|
| Pinned baseline | 2.791 / 2.968 | 31.325 / 36.706 | — |
| A | 2.718 / 2.899 | — | 0.613 |
| B (default) | 2.682 / 2.866 | 29.919 / 34.691 | 0.613 |
| C | 2.764 / 2.955 | 29.722 / 35.167 | 0.612 |

The paired B10 repetitions kept B within the pinned baseline in both passes;
class-on C was slightly slower than B. A3 timings were effectively tied. With
telemetry enabled separately, the default B8 run reported 524,340 pending
exact-reuse hits and 124,594 misses: 124,544 had no active collector, 50 had
no exact block, and none failed alignment. In class-on policy C, B8 had zero
class hits and 93,407 misses, all from empty classes. B10 policy C likewise
had zero class hits and 816,000 empty 32-byte class misses. The class cache
therefore remains available for explicit A/C comparisons, but is disabled in
the default policy.

Retained sizes and memory ratios did not change. Formatting, all-feature
workspace check/tests, strict Clippy, the x86-64 Apple collection check, the
complete Miri workflow (`PROPTEST_CASES=16`), and the all-feature workspace
release build passed. Release AArch64 assembly showed direct deque slot writes,
a direct 16-byte contiguous hash control load, and one-slice `Copy` retain
compaction. The x86-64 SSE2 classifier also loads contiguous groups directly
(`movdqu`). Neither assembly contained software prefetch instructions.

## V2.4 measured completion follow-up

The pinned implementation base is `4a57dc713f1158347b2c912b6d374ea4dd7213f7`.
The plan-only `e487cfb` commit used for the before measurements added no
production code, so its benchmark baseline is code-identical to the pinned
base. The three isolated workstreams were integrated as `ae65114` (allocator),
`bc0aafb` (deque/vector), and `d8ec6b9` (hash). The runs used the same
AArch64 two-core host and Rust 1.95 toolchain described above, with production
allocator policy B and telemetry disabled. Both complete 16-scenario suites
matched every native/compact logical checksum.

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-performance-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-performance-run2.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml,allocator-telemetry -- --scenario A2 --scenario B8 --output /tmp/v24-allocator-profile.tsv
```

The baseline columns below are the two targeted runs taken immediately before
these changes. Each cell is median/p95 milliseconds, run 1 / run 2. The final
compact/native ratio uses the corresponding final run medians.

| ID | Native end-to-end | Baseline compact | Completed compact | Completed/native | Retained memory |
|---|---:|---:|---:|---:|---:|
| A2 | 1.745/1.822; 1.781/3.411 | 2.591/2.735; 2.587/3.297 | 2.511/2.555; 2.520/2.802 | 1.44x / 1.41x | 1.500x |
| A4 | 0.359/0.379; 0.393/1.013 | 2.032/2.049; 2.033/2.438 | 1.994/2.058; 1.986/2.149 | 5.55x / 5.05x | 1.000x |
| A5 | 1.172/1.336; 1.178/1.315 | 4.986/5.222; 4.997/5.305 | 4.689/5.188; 4.668/7.737 | 4.00x / 3.96x | 1.000x |
| B6 | 0.040/0.048; 0.040/0.049 | 0.142/0.172; 0.142/0.165 | 0.141/0.187; 0.141/0.148 | 3.51x / 3.52x | 1.000x |
| B8 | 11.890/12.641; 12.131/13.421 | 32.676/44.395; 33.349/33.982 | 30.358/32.322; 30.392/35.412 | 2.55x / 2.51x | 1.026x |
| B10 | 1.232/1.335; 1.250/1.385 | 3.048/3.207; 2.884/2.931 | 2.913/3.180; 2.969/3.139 | 2.36x / 2.37x | 1.000x |

The phase medians show where the measured changes occurred. Values are compact
milliseconds, run 1 / run 2; B6 uses microseconds because its phases are short.

| Phase | Baseline | Completed | Median change |
|---|---:|---:|---:|
| A2 build | 1.549 / 1.538 | 1.469 / 1.478 | 4.5% faster |
| A4 deque mutation | 1.972 / 1.973 | 1.935 / 1.927 | 2.1% faster |
| A5 hash build | 3.029 / 3.019 | 2.813 / 2.818 | 6.9% faster |
| A5 hash mutation | 1.234 / 1.240 | 1.174 / 1.169 | 5.3% faster |
| A5 hash lookup | 0.535 / 0.537 | 0.513 / 0.510 | 4.7% faster |
| B6 quote updates and snapshot rebuilds | 88.161 / 88.001 µs | 88.040 / 87.921 µs | 0.1% faster |
| B6 snapshot copy | 2.840 / 2.840 µs | 2.800 / 2.800 µs | 1.4% faster |
| B8 fixed-population churn | 28.063 / 28.669 | 25.838 / 25.916 | 8.8% faster |
| B8 drop | 2.425 / 2.446 | 2.422 / 2.413 | 0.7% faster |
| B10 parallel allocate/drop churn | 2.852 / 2.710 | 2.738 / 2.782 | 0.8% faster |

The A5 median improved in both full runs. Its second full-run p95 was an
outlier; two additional isolated A5 runs had end-to-end medians of 4.733 ms
and 4.663 ms, with p95 values of 6.102 ms and 4.759 ms. Hash results and
checksums remained stable. B10 stayed within baseline timing variation. Cage
retained bytes were unchanged: A2 900,016 B; A4 32,784 B; A5 720,960 B;
B6 49,184 B; B8 3,620,248 B; B10 256,032 B. The frozen owner, deque, header,
and descriptor sizes remain 4 B, 12 B, 16 B, and 8 B respectively.

The allocator plan also names B3 and B5 as real-world regression sentinels.
They were measured in two additional release runs on the exact pinned
`4a57dc7` source and compared with the corresponding scenarios in the two full
completed runs. Values are compact median/p95 milliseconds, run 1 / run 2.

| Scenario | Pinned baseline | Completed | Retained memory |
|---|---:|---:|---:|
| B3 request metadata batch | 24.673/25.051; 24.372/24.619 | 23.784/24.408; 24.437/24.955 | 0.911x |
| B5 mobility dispatch state | 6.855/7.097; 6.824/7.736 | 6.444/8.935; 6.338/6.432 | 0.710x |

B3 was 3.6% faster in run 1 and 0.3% slower in run 2, with unchanged retained
memory. B5 medians improved by 6–7% in both runs with unchanged retained
memory. The first completed B5 run had a p95 outlier; its second-run p95 was
6.432 ms. Checksums matched between native and compact implementations in all
four targeted runs.

### Allocator profiling

A separate `allocator-telemetry` run gathered phase totals and pending-reuse
scan data for A2 and B8. These cumulative nanoseconds are diagnostic only:
timer and atomic instrumentation adds work, phases overlap, and telemetry
builds retain the full allocator accounting path rather than the default A2
empty-store fast path. They are excluded from all performance comparisons.

| Instrumented cumulative phase | A2 (ns) | B8 (ns) |
|---|---:|---:|
| Pending lookup | 13,699,822 | 182,708,678 |
| Layout computation | 13,457,886 | 63,012,839 |
| Allocator mutex wait | 16,434,410 | 7,361,422 |
| Reusable-store search | 13,409,512 | 11,183,217 |
| Cursor allocation | 44,370,842 | 8,088,464 |
| Header initialization | 12,612,623 | 20,424,489 |

Across one B8 telemetry child (warm-up plus nine measured repetitions), pending
reuse found 524,340 exact matches and missed 124,594 times. Of those misses,
124,544 had no active collector, 50 had no exact-size candidate, and none
failed alignment. The 524,390 lookups with an active collector scanned
1,704,488 candidates, averaging 3.25 candidates each. Scan depths were 0:10,
1:10, 2:22, 3:393,124, 4:131,084, 5:114, and 6:26. Candidate size buckets were
40 B:393,262, 112 B:393,282, 528 B:393,264, and 1,024 B or larger:524,680.
The shallow reverse scan did not justify adding an index. The same child
recorded 178,375 mutex acquisitions, 3,836,500 free-list nodes visited, and
53,576 release batches with a maximum size of 64.

### Validation and assembly

Both complete release suites passed, followed by the all-feature workspace
tests, strict Clippy, Apple x86-64 target check, full repository Miri workflow,
and workspace release build. Release assembly was inspected on AArch64 and
x86-64. No prefetch instruction, new inline assembly, object field, retained
pointer, or layout change was introduced.

## V2.4 profile-driven optimization results

Implementation of the profile-driven plan on top of the pinned profiling
baseline removed redundant cage-header resolution and cross-crate accessor
calls. Two consecutive uninstrumented release suites were captured with
`json,toml`; every native/compact and run-to-run checksum matched. End-to-end
medians and p95 are milliseconds over the default per-scenario repetitions.

| Scenario | Native med / p95 | Compact med / p95 | Ratio (run 1 / run 2) | Checksum |
| --- | ---: | ---: | ---: | ---: |
| A2 allocation | 1.755 / 1.883 ms | 1.830 / 2.140 ms | 1.04x / 1.04x | 6387817404620238636 |
| A4 deque | 0.291 / 0.332 ms | 1.283 / 1.385 ms | 4.40x / 4.42x | 2758306198405373103 |
| A5 hash | 1.166 / 1.340 ms | 3.248 / 3.314 ms | 2.78x / 2.77x | 17351022467741104803 |
| B3 request batch (sentinel) | 34.847 / 38.523 ms | 17.597 / 18.189 ms | 0.50x / 0.49x | 7757156252336854840 |
| B5 dispatch (sentinel) | 8.900 / 9.984 ms | 5.186 / 5.243 ms | 0.58x / 0.57x | 7959229199864024003 |
| B6 order book | 0.040 / 0.069 ms | 0.089 / 0.159 ms | 2.25x / 2.24x | 14879364451954781671 |
| B8 cache churn | 12.010 / 12.814 ms | 24.192 / 24.731 ms | 2.01x / 2.02x | 10141254246637991735 |
| B10 concurrent allocation | 1.295 / 2.075 ms | 2.744 / 3.172 ms | 2.12x / 2.18x | 214883317414038028 |

Relative to the profiled baseline, the compact/native gap narrowed on the
primary scenarios (A2 ~1.32-1.44x to 1.04x, A5 ~3.93-4.11x to 2.78x, B6
~3.49-3.65x to 2.25x, B8 ~2.54-2.59x to 2.01x), the B3/B5 sentinels stayed
comfortably ahead of native, and B10 stayed within its previously documented
host-noise band. Changes: `CompactHashMap::{insert,get_mut,remove_entry}`
resolve each table allocation once per operation, `CompactVec::push` appends
through `CageAllocation::extend_from_iter` to resolve the owner header once,
and the hot `CageAllocation`/header-resolution accessors are `#[inline]` so
they can fold across the non-LTO crate boundary. No layout, size, unsafe
contract, hashing, or accounting change was made; the intrusive allocator Miri
suite is green.

A4 remains the widest gap. Each `push_back`/`pop_front` already performs exactly
one essential cage-header resolution, and eliminating it would require caching a
validated native view across operations, which the frozen 12-byte deque layout
and no-persistent-pointer contract forbid. For B6, a caller that batches a
round of quote updates can borrow `CompactVec::as_mut_slice()` once per round
instead of indexing per update (`IndexMut` re-resolves the header per access);
this is an available usage pattern, not a library change, so the shared
benchmark retains its per-index workload.

## V2.4 round-two methodology correction and production-representative ratios

Baseline `1271594` plus the round-two harness commit `490eddd`. Two
consecutive release suites were captured and all 16 native/compact checksums
matched in both measurement modes (`benchmark_compare --self-check`).

**Correction to the section above.** Those suites ran with allocator
accounting ON. "Measure" mode installs a counting global allocator so the suite
can report allocation statistics. Compact collections allocate inside the cage
rather than through the global allocator, so the counter's atomic cost lands
almost entirely on the *native* variant. The compact/native ratios recorded
above are therefore instrumented and systematically flatter compact; the claim
that they were "uninstrumented" was wrong. The absolute compact times were
unaffected (mode-invariant within ~1%), which is why only the native
denominator moved.

Round two splits the harness: `--mode measure` keeps full accounting for
allocation statistics, `--mode profile` drops per-allocation counting and
per-phase allocator snapshots, and the new `benchmark_profile` example runs the
same scenario code with no counting allocator installed at all. Only the
accounting-free runs are production-representative for timing.

Paired, interleaved capture (five alternating measure/profile suites with
`--order alternate`, nine repetitions per phase, same clean release artifact;
medians in ms):

| Scenario | Measure native | Measure compact | Measure ratio | Profile native | Profile compact | Production ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A2 allocation | 1.733 | 1.801 | 1.04x | 1.177 | 1.795 | **1.53x** |
| A4 deque | 0.354 | 1.291 | 3.65x | 0.359 | 1.288 | 3.58x |
| A5 hash | 1.173 | 3.258 | 2.78x | 1.167 | 3.248 | 2.78x |
| B6 order book | 0.039 | 0.089 | 2.30x | 0.038 | 0.089 | 2.34x |
| B8 cache churn | 11.705 | 24.101 | 2.06x | 10.184 | 24.057 | 2.36x |
| B10 concurrent | 1.251 | 2.835 | 2.27x | 0.315 | 2.790 | **8.87x** |
| B3 request batch (sentinel) | 32.833 | 17.224 | 0.52x | 25.644 | 17.369 | 0.68x |
| B5 dispatch (sentinel) | 8.792 | 5.152 | 0.59x | 6.926 | 5.166 | 0.75x |

Corrected production picture: A2 is not near-native (1.53x, not 1.04x), B8 is
worse than recorded (2.36x), and B10 is far worse than recorded. B10 remains
the noisiest scenario on this two-vCPU host (compact medians span roughly
0.9-2.8 ms across captures, so the production ratio ranges about 2x to 9x); its
lock/atomic attribution is confirmed by system-wide sampling, not a single
number. B3/B5 remain genuine compact wins, though smaller (0.68x / 0.75x).
A4/A5/B6 are essentially accounting-invariant because those scenarios barely
touch the global allocator on the native side.

### Harness methodology facts that must travel with any numbers

- Per-process `perf record` samples no CPU for the B10 worker threads even
  with a long window; only system-wide capture (`-a`) recovers them. The
  round-one "native B10 has no samples" was this, not an idle process.
- `target/release/examples/benchmark_compare` is a hardlink that concurrent
  feature-different builds re-point. A sibling `allocator-telemetry` build
  inflated compact A2/B8/B3/B10 badly (A2 7.4x). Always benchmark an explicit
  hashed, telemetry-free artifact; `--self-check` guards checksum equivalence.
- The legacy allocator uprobes distorted runtime ~25-27x and mis-paired lock
  and transaction lifetimes; they are retired in favour of low-overhead
  sampling.

### Round-two optimization outcome (negative results, recorded)

No production change was landed; every candidate was rejected on evidence:

- Deque/vector/map: the remaining A4/B6/B8 cost is the intrinsic per-operation
  cage-header validation under the frozen 12-byte deque / no-retained-pointer
  layouts. `read_header` has zero out-of-line symbols (fully inlined) and is no
  longer a top self symbol. Batching `as_mut_slice()` once per update round is
  documented as a caller pattern, not a benchmark-only shortcut.
- Hash: `find_slot_in` probing plus the SIMD classifier is the residual; the
  classifier touches only ~3-15% of samples and no change reached the target
  without altering hashing security, which the plan forbids.
- Allocator: the largest allocator symbol in B8 is `release_many_locked` at
  ~5.9% (allocator total ~20%, hash map ~50%); removing the allocator entirely
  would still leave B8 near 1.6x. B10 is ~60% lock/futex/atomic machinery, so
  no critical-section shortening reaches the target; closing it needs a
  synchronization/design change the plan forbids. Micro-experiments fell below
  the ~1.2% harness resolution and were not landed.

The round-two profile evidence, codegen comparison, and memory-reconciliation
details are in `PROFILE_V2_4_ROUND2.md`; allocator-specific measurements are in
`PROFILE_ALLOCATOR.md`.

## V2.4 round-three results (CI repair, hash lookup, deque batch view)

### Benchmark CI repair: lockfile policy

GitHub Actions run 37725406606 failed at the contract step with
`error: cannot update the lock file ... because --locked was passed` — it was
**not** a checksum mismatch. Root cause: the committed `Cargo.lock` carried
seven trailing `[[patch.unused]]` blocks for the local `compact_*` crates. Those
entries are written by a developer-host `[patch.crates-io]` configuration (a
`~/.cargo`/`/.cargo` file, not part of the repository); CI has no such patch, so
cargo must drop the entries and `--locked` refuses to proceed.

Policy for this repository:

- `Cargo.lock` is tracked and must stay free of `[[patch.unused]]` blocks; those
  are a local-configuration artifact, not repository state.
- If a local `[patch.crates-io]` config is active, run cargo once without
  `--locked` only to resolve, then restore the tracked lock (`git checkout --
  Cargo.lock`) before committing. Verify with
  `cargo metadata --locked` in a config-free shell.
- Regenerate the lock with the intended toolchain and inspect dependency
  changes; do not silently drop `--locked` from CI.

With the phantom entries removed, `cargo metadata --locked` and the exact
workflow command (`benchmark_compare --features json,toml -- --self-check`)
resolve and pass, 16/16 scenarios, with `Cargo.lock` still at format version 3.

### Hash lookup probe (`find_index_in`) — landed

`CompactHashMap::{get,get_key_value,get_mut,remove_entry}` previously shared
`find_slot_in`, which tracks the earliest tombstone because `insert` needs an
insertion slot. Read-only lookups never need that slot, so they now use a
dedicated `find_index_in` probe that skips tombstones and stops at the first
`EMPTY`. Probe termination, key equality, the SIMD/scalar classifier and the
matched slot are unchanged; `find_slot_in` still backs `insert`.

| Phase (A5) | baseline | candidate |
|---|---|---|
| `lookup_scan` | 0.459–0.465 ms | 0.445–0.456 ms |
| `mutation` | 0.774–0.790 ms | 0.757–0.763 ms |
| `end_to_end` | 3.230–3.266 ms | 3.119–3.165 ms |

Two independent A/B captures (interleaved, 5–9 rounds, accounting-free profile
mode) agree on ~4% off `lookup_scan`; B8/B5 were neutral and the logical checksum
was identical. Hash-flood defense and the randomized default hash are untouched.

### Deque borrow-scoped batch view (`with_view`) — landed, opt-in

Every `CompactVecDeque::{push_back,pop_front}` resolves and fully re-validates
the backing cage allocation, which dominates steady-state A4 churn. A new
borrow-scoped `CompactVecDeque::with_view` resolves the ring once for a whole
batch of non-growing front/back operations. It stores only a borrow-scoped slot
slice plus head/len and writes the metadata back on return, early return and
unwind; the deque keeps its frozen 12-byte layout and retains no native pointer.
Growth is disallowed while borrowed (reserve first; an over-capacity push returns
`AllocationExhausted` with the deque left valid).

A4 mutation phase with the batched call pattern: **1.252 ms → 0.119 ms** (native
per-op is 0.356 ms); `end_to_end` 1.292 ms → 0.157 ms; identical checksum. This
is an opt-in call-site pattern: the shared A4 arm still measures ordinary
per-operation `push_back`/`pop_front` against the native `VecDeque` loop, and was
deliberately **not** rewired to the batch view, because a batched-compact arm
against a per-op-native arm would report an API advantage rather than a per-op
cost and would cease to be a like-for-like comparison.

### Allocator (B10/B8) and vector (B6) — analysis, no landed change

- B10 remains dominated by lock/atomic/futex machinery; the safe, local
  critical-section shortenings available do not reach the target, and the
  closed-form fix (bounded thread-local chunks / lock-free reclamation) is gated
  by the plan: it needs a documented reservation/accounting/publication/remote-
  free/thread-exit/reclamation contract plus central approval before any code.
  Recommendation recorded: do **not** implement without that contract.
- B8's allocator share is smaller than its hash share, so hash-side work matters
  more; no free-list/coalescing/tail change was landed because none showed a
  repeatable win with an exact-accounting proof.
- B6/`CompactVec`: batching `as_mut_slice()` once per order-book update round is a
  caller pattern (consistent with round-two), not a new API or a benchmark-only
  change. Codegen inspection found `read_header`/`validate_typed_header` fully
  inlined with no safe redundant range check to remove without weakening typed
  header/lifetime validation.

Accepted round-three changes: hash lookup probe, deque batch view, CI lockfile
repair. The integrated round-three evidence is in `PROFILE_V2_4_ROUND3.md`.

### Round-three integrated ratios (independent orchestration verification)

Paired/interleaved capture on `984f162` (three `measure` plus three `profile`
suites with `--order alternate`, clean telemetry-free release artifact, nine
repetitions per phase; medians in ms, ratio = compact/native):

| Scenario | measure native | measure compact | measure ratio | profile native | profile compact | production ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| A2 allocation | 1.240 | 1.281 | 1.03x | 1.208 | 1.808 | 1.50x |
| A4 deque | 0.354 | 1.281 | 3.62x | 0.354 | 1.288 | 3.64x |
| A5 hash | 1.173 | 3.152 | 2.69x | 1.170 | 3.115 | 2.66x |
| B6 order book | 0.039 | 0.089 | 2.29x | 0.038 | 0.089 | 2.33x |
| B8 cache churn | 11.826 | 23.977 | 2.03x | 10.237 | 23.994 | 2.34x |
| B10 concurrent | 1.259 | 2.795 | 2.22x | 0.424 | 2.795 | 6.59x |
| B3 request batch (sentinel) | 33.521 | 17.474 | 0.52x | 26.225 | 17.474 | 0.67x |
| B5 dispatch (sentinel) | 9.004 | 5.177 | 0.58x | 7.031 | 5.177 | 0.74x |

Against the round-two production table, A5 improves 2.78x -> 2.66x, which
matches the measured ~4% `lookup_scan` gain from the `find_index_in` probe. A4
is unchanged (3.58x -> 3.64x, within noise) because the shared arm still
measures per-operation deque operations rather than the opt-in batch view. B3
and B5 remain compact wins; B10 stays the noisiest scenario on this two-vCPU
host.

Reproducibility note for development hosts: this workstation had a stale
root-level `/.cargo/config.toml` carrying the v2.2.0 `[patch.crates-io]` that
writes the phantom `[[patch.unused]]` blocks back into the tracked lock. It was
disabled (backed up) so `cargo metadata --locked` and the exact harness workflow
command resolve against the committed lock, matching CI.
