# V2.5 allocation-pattern policies

## Outcome

The allocator now allows the existing bounded, exact local cache to serve
32-byte blocks before allocator contention. This captures repeatable
single-worker B10 reuse. When real contention occurs, the previous policy
continues to allow all existing eligible size classes. The shared cage
allocator remains authoritative, and releases still fall back to its shared
free-space path whenever local reuse is unavailable.

The policy is selected from block size and alignment; it does not inspect
collection names. It adds no allocator, region hierarchy, thread-affine
allocation ownership, or background reclamation. The four-byte
`CageAllocation<T>`, 16-byte allocation header, collection layouts, and default
public APIs are unchanged. New alignment, queue, and resize measurements are
available only with the existing `allocator-telemetry` feature.

The exact 40-byte pre-contention policy was measured and rejected: it roughly
doubled B4 time in both 101-run captures. No non-B10 scenario showed a
reproducible policy-specific speedup in this pass. Other policies were not
added without such evidence. The existing report
[`PROFILE_V2_5_SYSTEM_WIDE_REUSE.md`](PROFILE_V2_5_SYSTEM_WIDE_REUSE.md)
records prior rejected always-attempt, broad-size, and direct-lock candidates.

## Baseline and method

The plan was fast-forwarded to `6166106286eb4a202d5c1c7c032ca6f17b106be2`;
that commit is the source baseline used for these comparisons. The plan cites
`184ee63b9b196d5c5ecb858fb7cbf6080326dd9f` as the earlier verified allocator
baseline. The accepted candidate was measured from the implementation
working tree based on `6166106`.

The host was Linux AArch64 with two available CPUs, Rust 1.95.0 / LLVM 22.1.2.
Timing binaries used `json,toml`, release mode, and no allocator telemetry.
The all-scenario captures used the benchmark harness's profile mode, 31
repetitions per scenario, and two capture orders (`alternate` and `fixed`).
Each table value below is the median of the two capture medians or p95s; these
are two captures, not 62 independently paired process runs. Native and compact
checksums matched in every capture. B10 also had dedicated profile captures
with 101 repetitions for one and two workers. The host showed substantial
scheduling variation, especially in B10 p95, so these results support the
policy choice but do not establish a general performance guarantee. No p99 or
p99.9 claim is made.

## All 16 scenarios

Times are milliseconds. A positive delta is slower for the candidate.

| Scenario | Median, baseline → class-32 | Δ | p95, baseline → class-32 | Δ |
| --- | ---: | ---: | ---: | ---: |
| A1 | 0.071 → 0.071 | −0.1% | 0.093 → 0.105 | +12.9% |
| A2 | 1.665 → 1.657 | −0.5% | 1.973 → 1.784 | −9.6% |
| A3 | 0.602 → 0.611 | +1.4% | 0.729 → 0.785 | +7.8% |
| A4 | 1.073 → 1.073 | −0.1% | 1.120 → 1.097 | −2.0% |
| A5 | 2.739 → 2.729 | −0.3% | 2.859 → 2.880 | +0.7% |
| A6 | 1.260 → 1.264 | +0.3% | 1.303 → 1.289 | −1.1% |
| B1 | 16.286 → 16.190 | −0.6% | 17.011 → 16.553 | −2.7% |
| B2 | 0.994 → 1.008 | +1.4% | 1.072 → 1.063 | −0.8% |
| B3 | 18.009 → 17.909 | −0.6% | 18.614 → 18.968 | +1.9% |
| B4 | 7.239 → 7.229 | −0.1% | 7.304 → 8.377 | +14.7% |
| B5 | 4.815 → 4.822 | +0.1% | 4.881 → 5.213 | +6.8% |
| B6 | 0.078 → 0.078 | −0.1% | 0.102 → 0.087 | −14.9% |
| B7 | 14.227 → 14.393 | +1.2% | 15.131 → 14.872 | −1.7% |
| B8 | 21.514 → 21.595 | +0.4% | 22.085 → 22.988 | +4.1% |
| B9 | 4.536 → 4.643 | +2.4% | 5.643 → 5.493 | −2.7% |
| B10, default two workers | 1.025 → 1.201 | +17.2% | 1.730 → 1.673 | −3.2% |

Outside B10, the geometric mean of the per-scenario median ratios was +0.31%.
The largest outside-B10 median movement was B9 at +2.4%; B4's median was
unchanged within measurement resolution. The B10 all-suite median is noisy:
its two baseline capture medians were 0.843 ms and 1.206 ms, while candidate
medians were 1.197 ms and 1.205 ms. The dedicated B10 captures below provide
more repetitions for that workload.

### Dedicated B10 captures

Each row is a separate 101-repetition capture. Pair B changes the scenario
order by including A1 before B10.

| Workers | Pair | Baseline median / p95 | Candidate median / p95 | Median Δ | p95 Δ |
| ---: | :---: | ---: | ---: | ---: | ---: |
| 1 | A | 0.734 / 3.910 ms | 0.406 / 0.767 ms | −44.7% | −80.4% |
| 1 | B | 1.712 / 4.018 ms | 0.427 / 0.929 ms | −75.0% | −76.9% |
| 2 | A | 2.215 / 4.874 ms | 1.238 / 2.015 ms | −44.1% | −58.7% |
| 2 | B | 2.051 / 6.013 ms | 1.220 / 1.623 ms | −40.5% | −73.0% |

The static policy accounts for the one-worker change: the incumbent does not
reuse local extents until contention, while the candidate caches and reuses
exact 32-byte blocks without contention. In two-worker B10, actual contention
already enables the incumbent policy, so the implementation leaves that route
unchanged. Its repeated measurements remain favorable to the candidate here,
but the all-suite and dedicated results disagree on the size of that benefit;
this pass treats preservation of the two-worker path as the relevant gate.

## Telemetry and memory

Telemetry came from separate `allocator-telemetry` builds in measure mode and
was not used for timing. B10 requests are all 8-byte aligned; its repeated
small block is 32 bytes, alongside a handful of 1 KiB+ blocks. B4 is primarily
4-byte aligned and allocates 40- and 48-byte blocks.

| Workload | Allocator locks | Free-list visits | Local lookups / hits | Cached releases | Peak local bytes / 4 KiB budget |
| --- | ---: | ---: | ---: | ---: | ---: |
| B4, class-32 candidate | 841,716 | 1,168,170 | 440,980 / 0 | 0 | 0 / 4,096 |
| B10, incumbent, 1 worker | 64,076 | 0 | 32,008 / 0 | 0 | 0 / 4,096 |
| B10, class-32 candidate, 1 worker | 92 | 0 | 32,008 / 31,992 | 32,000 | 32 / 4,096 |
| B10, incumbent, 2 workers | 124 | 8 | 64,016 / 63,984 | 64,000 | 64 / 4,096 |
| B10, class-32 candidate, 2 workers | 124 | 8 | 64,016 / 63,984 | 64,000 | 64 / 4,096 |

The B10 two-worker telemetry is unchanged by the candidate, as expected from
the retained contention path. The new request-alignment histogram,
pending-release queue counters, and resize outcomes are emitted as separate
metadata rows. The B4 and B10 timing scenarios ended with zero pending-release
queue depth and had no resize attempts. B2's 250 resize attempts included 10
no-growth resizes, 10 cursor growths, 20 free-range growths, and 210 attempts
that could not grow in place. These counters distinguish the queue and resize
paths from local-cache behavior.

Five retained-memory probe runs per version produced identical median
snapshots:

| Probe phase | RSS / HWM, baseline and candidate | Cage live bytes | Cursor high-water |
| --- | ---: | ---: | ---: |
| Main thread, 512 KiB live payload | 2,336 / 2,336 KiB | 526,336 | 526,344 |
| Main thread dropped, quiescent | 2,404 / 2,404 KiB | 0 | 8 |
| Worker thread, 512 KiB live payload | 2,424 / 2,424 KiB | 526,336 | 526,344 |
| After worker exit, quiescent | 2,488 / 2,488 KiB | 0 | 8 |

The measured retained RSS and peak RSS delta was zero. This probe uses a 512 KiB
payload and does not itself populate the 32-byte local cache; the B10 telemetry
above measured its peak occupancy at only 32–64 bytes. The cache keeps its
existing 4 KiB budget, 16-owner cap, and 16-entry per-owner cap, and worker
exit flushes cached descriptors to shared reclamation.

## Candidate decisions and safety

- **Accepted:** pre-contention local reuse for exactly 32-byte blocks with
  alignment no greater than eight. The unit test covers the size/alignment
  selector, and the integration test allocates, drops, and reuses an actual
  same-offset 32-byte extent. All other pre-contention size classes keep the
  incumbent behavior.
- **Rejected:** pre-contention 40-byte reuse. In two 101-repetition B4
  captures, baseline medians of 7.927 and 7.311 ms became 16.700 and 16.736
  ms (+110.7% and +128.9%). The exact 40-byte class therefore failed the
  mixed-size B4 gate; 48-byte requests were not admitted to the cache.
- **Not added:** adaptive per-class counters, broader compatible extents,
  alignment expansion, and a new large-buffer route. B8 already records
  extensive exact reuse through its batched pending-release path, and the
  measured scenarios did not establish another local-cache opportunity worth
  adding state or work for.

No per-collection allocator changes were made. Existing tests cover bounded
cache accounting, release batching, exhaustion, remote release, panics, and
thread exit. Miri and the real threaded integration tests passed; they do not
prove every possible concurrent interleaving. No Loom or sanitizer run was
performed.

## Validation

Passed against the final class-32 source:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- `cargo run --locked --release -p compact_std --example benchmark_compare --features json,toml -- --self-check` — measure/profile checksums matched in all 16 scenarios.
- The complete `.github/workflows/miri.yml` sequence: core (3), collection ownership (22), deque view (8), batch access (9), allocator integration (1), allocator library (49), and V2.4/Serde (7) tests passed.
- Two 31-repetition, all-16 profile captures per version; all checksums matched.
- Two 101-repetition B10 captures at each worker count; all checksums matched.
- `git diff --check`.
