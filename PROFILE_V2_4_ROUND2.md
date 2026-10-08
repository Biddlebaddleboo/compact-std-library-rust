# V2.4 round-two profiling report

## Scope and pinned facts

Profiling workstream deliverable for `PLAN_PROFILING.md`. It re-measures the
tree after the round-one profile-driven optimizations and after the round-two
benchmark-methodology change, and it makes no production or harness edits.

- Start SHA: `12715942dbc5b6b01cee049dfd7e7860b56366fe` (`main`, clean).
- The benchmark workstream landed `490eddd` ("separate measure and profile
  harness modes") at 03:38, *after* the first capture pass here. Fresh
  measurements were therefore recaptured on the post-`490eddd` harness, and a
  pre-change control pass is retained for continuity. Only
  `crates/compact_std/examples/{benchmark_compare,benchmark_profile}.rs` and
  `Cargo.toml` changed; production collections/allocator sources are untouched.
- Host: Linux `6.17.0-1020-oracle`, AArch64 Neoverse-N1 VM, two logical CPUs,
  one socket and NUMA node. `perf 6.17.13`, `perf_event_paranoid=4`,
  `yama/ptrace_scope=1`; sampling used `sudo -n perf` without changing sysctls.
- Toolchain: `rustc`/`cargo 1.95.0`, LLVM `22.1.2`, stable AArch64 GNU.
- Builds: `--release`, features `json,toml`, default allocator policy B. The
  measured binary had **no** `allocator-telemetry` (verified by absence of the
  phase-profile strings in the binary). Sampling used a separate
  `-C debuginfo=1 -C force-frame-pointers=yes` build.

This workstream owns `scripts/**` and this report only.

## Measurement-methodology findings (read before trusting any table)

Three methodology facts materially affect the numbers and must travel with
them.

1. **Feature-race on the shared example path.** `target/release/examples/benchmark_compare`
   is a hardlink to a hashed artifact, and a concurrent workspace build with a
   different feature set re-points it. A sibling build enabled
   `allocator-telemetry`, and a suite run against that path inflated compact
   allocation scenarios badly (A2 7.37x, B8 5.63x, B3 3.32x, B10 7.47x) with
   `allocator_phase_profile` rows present. Those numbers are instrumentation
   artifacts and are discarded. All results below use an explicit hashed
   telemetry-off artifact and are the authoritative ones. Any harness run that
   reads the un-suffixed path is unsafe while other agents build.
2. **The harness change moved headline ratios.** The post-`490eddd` measure
   mode changes warm-up/workload plumbing enough that native A4 and native B10
   differ from the pre-change control: A4 compact/native fell from 4.40x to
   ~3.3-3.6x (native A4 slower), and B10 fell from ~2.3x to ~1.4-1.5x (native
   B10 faster). The `490eddd` claim that measure-mode ratios reproduce the
   recorded baseline holds for most scenarios but **not** for A4/B10; see the
   two tables below. Ratios must be compared within one harness revision.
3. **Per-process `perf` misses B10 worker-thread CPU.** `perf record` without
   `-a` captured 0 samples for B10/native in round one and again here, even
   with a 5 s profile window, while the process did real work. System-wide
   (`-a`) capture recovered ~1K samples and 3.56 s of `cpu-clock:u`. B10
   sampling is valid only with `-a` (or equivalent thread inheritance).

Mode equivalence is otherwise proven: all 16 scenarios produce identical
native/compact logical checksums in both `measure` and `profile` modes
(32/32 pairs matched).

## Authoritative suite results (new harness, measure mode)

Two full 16-scenario suites on the explicit telemetry-off release artifact;
run 2 used `--order alternate`. End-to-end medians (ms) native → compact and
the compact/native ratio.

| Scenario | Run 1 native → compact | Ratio | Run 2 (alt order) native → compact | Ratio |
| --- | ---: | ---: | ---: | ---: |
| A1 | 0.067 → 0.072 | 1.08x | 0.068 → 0.071 | 1.06x |
| A2 allocation | 1.841 → 1.869 | 1.02x | 1.784 → 1.822 | 1.02x |
| A3 | 6.534 → 0.627 | 0.10x | 4.615 → 0.642 | 0.14x |
| A4 deque | 0.356 → 1.290 | 3.62x | 0.395 → 1.297 | 3.28x |
| A5 hash | 1.176 → 3.281 | 2.79x | 1.215 → 3.315 | 2.73x |
| A6 | 1.188 → 1.295 | 1.09x | 1.234 → 1.299 | 1.05x |
| B1 | 13.890 → 18.012 | 1.30x | 14.158 → 18.190 | 1.28x |
| B2 | 0.982 → 1.189 | 1.21x | 0.986 → 1.186 | 1.20x |
| B3 request batch (sentinel) | 34.344 → 17.906 | 0.52x | 34.536 → 18.146 | 0.53x |
| B4 | 6.512 → 7.045 | 1.08x | 6.842 → 7.566 | 1.11x |
| B5 dispatch (sentinel) | 9.180 → 5.289 | 0.58x | 9.053 → 6.313 | 0.70x |
| B6 order book | 0.038 → 0.089 | 2.32x | 0.039 → 0.090 | 2.32x |
| B7 | 21.471 → 14.868 | 0.69x | 22.111 → 14.551 | 0.66x |
| B8 cache churn | 12.355 → 25.539 | 2.07x | 12.297 → 25.029 | 2.04x |
| B9 | 11.236 → 5.247 | 0.47x | 11.298 → 5.329 | 0.47x |
| B10 concurrent allocation | 0.677 → 0.990 | 1.46x | 0.677 → 0.949 | 1.40x |

B3/B5 (and A3/B7/B9) compact wins are retained. A2 stays near-native. B5
run 2 (0.70x) is a single-run outlier outside its 0.58-0.59x norm.

### Pre-change control (old harness), for continuity

Same scenarios on the pre-`490eddd` artifact; these reproduce the numbers
recorded in `PLAN.md` and the round-one summary.

| Scenario | Run 1 ratio | Run 2 ratio |
| --- | ---: | ---: |
| A2 | 1.04x | 1.04x |
| A4 | 4.40x | 4.40x |
| A5 | 2.79x | 2.83x |
| B3 | 0.51x | 0.52x |
| B5 | 0.58x | 0.58x |
| B6 | 2.24x | 2.26x |
| B8 | 2.01x | 2.05x |
| B10 | 2.33x | 2.26x |

## Profiling-build control

The ordinary and `debuginfo=1 + force-frame-pointers` binaries agree within
0.4-3.7% on A4/A5/B6/B8/B3/B5. A2 grew ~11% under the instrumented build.
B10 swung by tens of percent from build flags alone, consistent with
scheduler sensitivity on two vCPUs; no B10 conclusion should rest on a single
pair of timings.

## Low-overhead profile-mode sampling (authoritative hotspots)

`benchmark_profile --window <scenario> <variant> 4` (profile mode: System-only,
no per-allocation counting, no per-phase snapshot), sampled with
`perf record -e cpu-clock:u -F 499 -g --call-graph dwarf,16384`. Four-second
windows give ~2.3-2.5 K samples per capture; zero lost samples. Sample counts
(A2 1761/2369, A4 2417/2456, A5 2399/2330, B6 2236/2299, B8 2478/2484,
B3 2133/2467, B5 2490/2467) all native/compact checksum-matched.

Top exclusive symbols (self time), compact:

| Scenario | Leading self symbols (share) |
| --- | --- |
| A4 deque | `CompactVecDeque::push_back` 58.3%; `pop_front` 37.1%; closure 3.6% |
| A5 hash | `find_slot_in` 18.5% + 15.6%; `classify_control_group` 15.2%; `insert` 14.1%; `CompactHashSet::insert` 12.0%; `SipHasher24::finish` 9.4% |
| B6 order book | closure 26.0%; `IndexMut` 22.2%; `CompactVec::retain` 22.1%; `CompactVec::push` 16.2% |
| B8 cache churn | `find_slot_in` 20.5%; `insert` 6.7%; `SipHasher24::finish` 6.6%; `classify_control_group` 6.3%; `release_many_locked` 6.0%; `remove` 5.9%; `truncate_inner` 4.2% |
| B5 dispatch | `find_slot_in` 17.0%; `CompactString::from_str` 13.2%; `compact_offer` 8.2%; `insert` 7.7%; `classify_control_group` 5.0% |
| A2 allocation | `CompactBox::get` 16.0%; `alloc_owned_value` 14.0%; `cas4_acq` 12.1%; `swp4_rel` 10.9%; `truncate_inner` 8.7%; `Drop::drop` 5.9%; `CompactVec::push` 5.4% |
| B3 request batch | `cas4_acq` 9.1%; closure 7.0%; `swp4_rel` 6.7%; `CompactString::from_str` 5.7%; `truncate_inner` 4.9% |
| B10 concurrent allocation | `cas4_acq` 28.0%; `swp4_rel` 18.4%; `futex Mutex::lock_contended` 15.9%; `alloc_owned_value` 5.7%; `syscall` 3.6%; `release_many` 3.4%; `lock` 3.2% |

Because profile mode removes the harness `CountingAllocator`, native samples
are now clean `System` allocator cost instead of atomics: A2/native is
`malloc` 15.5% + `cfree` 9.8% + `__rust_alloc` 3.8%; A5/native is
`hashbrown insert` 31.2% + 27.6% + `hash_one` 12.7%; B8/native is closure
30.6% + `hashbrown insert` 13.7% + `cfree` 8.0%. The round-one `__aarch64_ldadd8_relax`
native symbol is a counting-harness artifact and is absent here.

B10 is now usable. Compact B10 spends ~62% of self time in `cas4_acq` +
`swp4_rel` + contested futex, i.e. allocator lock/atomic contention, with
`alloc_owned_value` next. Native B10 (system-wide capture) spends most of its
worker CPU in `malloc` 13.0% + `cfree` 11.5% and `__rust_alloc` 3.2%. Worker
threads carry the load: `b10-compact-chu` 99.95% of compact samples;
`b10-native-chur` 52.3%, `b10-native-read` 7.0%, `b10-native-buil` 6.4%. The
named-thread instrumentation added in `490eddd` works.

### Counting-harness contrast (first pass)

The earlier `benchmark_compare` sampling (5 K-rep A2/A4, etc., with the
counting allocator installed) gave the same picture for compact — A4
`push_back` 58.4% + `pop_front` 37.5%, A5 `find_slot_in` 18.4% + 14.6% +
`classify` 12.9% + `insert` 13.3%, B6 `retain` 23.1% + `IndexMut` 20.6% +
`push` 17.3%, B8 `find_slot_in` 22.7% + `insert` 6.6% — so the two modes
corroborate on production paths and differ only in native accounting.

### What the round-one optimization changed

`read_header` is gone as a distinct cost: it has **zero** out-of-line symbols
in the release binary and is no longer a top self symbol anywhere; its work is
inlined into `push_back`/`pop_front`/`CompactVec::push`/`insert`. The
remaining A4/A5/B6/B8 distance is the intrinsic work of those operations under
the frozen layouts, not a redundant helper call.

## Generated-code observations

Assembly emitted for the example on both targets shows header validation truly
inlined and the cage base loaded once:

- AArch64 (`CompactVecDeque::push_back`, emitted with
  `cargo rustc --release -p compact_std --example benchmark_compare
  --features json,toml -- --emit asm`; the `.s` is transient and removed by the
  next ordinary build): one `adrp`/`ldr ... CAGE` GOT load, inline header
  checks (`ldr w13,[x15,#4]; cmn w13,#17; cmp w14,w13; ...`), a single
  `str x19,[x9,x8,lsl #3]` store, and exactly one
  `bl CageAllocation::allocate` on the growth path. No `read_header` call.
- x86-64 Apple (`target/x86_64-apple-darwin/release/examples/benchmark_compare-e6aadb73d3cd8338.s`):
  same shape — one `movq CAGE@GOTPCREL` load, inline `movl` header checks with
  `setae`/`setb`, a direct `movl` store, no `read_header` call
  (`grep -c read_header` = 0 on both files).

`find_slot_in` and `classify_control_group` remain real function symbols
(loop / SIMD-classifier entry), matching the profile. x86-64 was verified by
assembly emission only: the link step fails on this host because no macOS SDK
is installed, so no x86-64 binary and no x86-64 runtime measurement exist.

## Memory distinctions

Retained native live bytes and compact cage bytes after build are identical to
round one, confirming the frozen representation (4-byte owner, 12-byte deque,
16-byte header, 8-byte descriptors). Peak RSS is per-process and includes
runtime and fixtures; the 128 MiB cage is a virtual reservation.

| Scenario | Native live after build (B) | Compact cage after build (B) | Cage high-water (B) | Peak RSS native/compact (KiB) |
| --- | ---: | ---: | ---: | ---: |
| A2 | 600,000 | 900,016 | 900,024 | 3340 / 3448 |
| A4 | 32,768 | 32,784 | 32,792 | 2696 / 2740 |
| A5 | 720,912 | 720,960 | 720,968 | 3344 / 3408 |
| B6 | 49,169 | 49,184 | 49,192 | 2764 / 2768 |
| B8 | 3,528,488 | 3,620,248 | 3,620,256 | 35644 / 35076 |
| B3 | 13,071,260 | 11,903,936 | 11,903,944 | 42452 / 34636 |
| B5 | 4,846,280 | 3,440,672 | 3,440,680 | 13704 / 10904 |
| B10 | 256,048 | 256,032 | 256,040 | 3280 / 3272 |

## Suspected vs confirmed bottlenecks

Confirmed (sampled, checksum-matched, corroborated across two capture modes):

- A4 deque mutation is effectively the whole scenario (96% of compact A4) and
  lives inside `push_back`/`pop_front`, one header validation and ring index
  per op, with no cacheable validated view under the frozen 12-byte layout.
- A5 and B8/B5 hash paths are dominated by `find_slot_in` probing, with
  material secondary cost in the SIMD `classify_control_group` (now ~15% of
  A5, no longer a 2-3% footnote) and Sip hashing.
- B6 indexed mutation splits between `IndexMut`, `retain`, and `push`.
- A2 compact is allocation plus atomics; B10 compact is allocator
  lock/atomic contention (CAS + contended futex), native B10 is malloc/cfree.

Not claimed: any B10 hot path without `-a`; any "pure System" native number
taken from the counting harness; x86-64 runtime behaviour.

## Accepted / rejected experiments and next candidates

Accepted in round one and re-confirmed effective (kept): cross-crate inline of
cage/header accessors (out-of-line `read_header` eliminated), one header
resolution per `CompactVec::push`/`push_back`, single control/entry resolution
per `CompactHashMap::{insert,get_mut,remove_entry}`, and the deterministic
pending-reuse test.

Rejected or held (round one): allocator-internals changes for B10 without
attribution (now partially addressed — see below); editing the shared
benchmark to batch `as_mut_slice()` on only the compact arm (would game the
metric). Discarded here as an audit artifact: the telemetry-enabled binary race
described above.

Isolated reversible experiments for the next round, ranked (owners per
`PLAN.md`; production code is read-only to this workstream):

1. A4 deque — highest absolute gap. Test operation-scoped writable header
   reuse while proving ring-wrap, growth, and ownership invariants. High risk:
   the frozen 12-byte layout forbids caching a validated native view.
2. B10 allocator contention — now evidenced: ~62% of compact self time is CAS /
   atomic-swap / contended futex plus `alloc_owned_value`. The allocator
   workstream can now justify a bounded critical-section or sharding
   experiment against the `-a` profile. This is the change round one correctly
   deferred.
3. A5 probing + classifier — A/B a cheaper probe loop and re-check the ~15%
   classifier cost on the hit path.
4. B8 map vs release — release (`release_many_locked`, `truncate_inner`) is
   only ~10% exclusive next to ~37% hash lookup/insert/remove; prioritize the
   map.
5. B6 indexed update — measure a bounded batched update path before changing
   `CompactVec`.

## Evidence limits

- One shared two-vCPU ARM VM; `cpu-clock:u` software event, not cycles or
  instructions. CPU seconds are approximate; inclusive paths overlap and are
  never summed.
- A2 shows an ~11% instrumented-build delta; B10 absolute timing is
  scheduler-sensitive and build-flag-sensitive.
- Native numbers from the counting harness include harness atomics; the
  profile-mode numbers here do not.
- System-wide B10/native capture includes incidental system tasks (driver
  11%, perf, other agents) alongside the named benchmark worker threads.
- Some glibc frames remain unsymbolized; application frames are symbolized.
- This workstream changed no production or harness source, so it introduces no
  correctness risk; its timing deltas are measurement context only.

## Reproduction artifacts

Ordinary-release suites: `/tmp/csl-v24-round2/suite3-run{1,2}.tsv` (new
harness) and `suite-run{1,2}.tsv` (pre-change control). Profile-mode driver and
windows: `profilemode-all.{tsv,log}`. Low-overhead samples and reports:
`/tmp/csl-v24-round2/perf2/{data,logs,reports}`. Counting-harness samples:
`/tmp/csl-v24-round2/{perf-data,profile-logs,reports}`. System-wide B10/native:
`perf2/data/B10-native-a.data`. Host facts: `host.txt`. The
`scripts/profile_cpu.sh` `suite` command was added to reproduce the full
two-suite capture. Machine-specific raw profiles stay outside the repository.
