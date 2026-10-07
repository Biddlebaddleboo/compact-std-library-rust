# V2.4 native Rust comparison

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
model, pointer model, owner/header, or allocator redesign belongs in V3.
Correctness fixes, documentation corrections, test improvements, and benchmark
maintenance remain allowed.
