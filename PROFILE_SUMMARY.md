# V2.4 performance investigation summary

## Scope and result

This is a profiling-only investigation of baseline
`fc0a58bf08aa3bffc577ec2da8151d262b6a5f53`. It makes no production
collection, allocator, hashing, ownership, layout, or public API changes. The
CPU, allocator, and collection workstreams ran in separate worktrees and are
documented in [PROFILE_CPU.md](PROFILE_CPU.md),
[PROFILE_ALLOCATOR.md](PROFILE_ALLOCATOR.md), and
[PROFILE_COLLECTIONS.md](PROFILE_COLLECTIONS.md).

The full release benchmark suite was run twice on source-identical code using
`json,toml` features. All 16 native/compact scenario checksums matched in both
runs. The table gives the end-to-end median for the plan's eight primary and
sentinel scenarios. Each value is the median across the benchmark's default
per-scenario repetitions (15 for A scenarios, 9 for B1/B2/B3/B4/B6/B8, and 7
for B5/B7/B9/B10).

| Scenario | Native → compact median, run 1 (ms) | Native → compact median, run 2 (ms) | Compact/native ratio, run 1 / run 2 |
| --- | ---: | ---: | ---: |
| A2 allocation | 1.912 → 2.521 | 1.745 → 2.510 | 1.32× / 1.44× |
| A4 deque | 0.359 → 2.208 | 0.360 → 1.992 | 6.14× / 5.54× |
| A5 hash | 1.167 → 4.800 | 1.190 → 4.671 | 4.11× / 3.93× |
| B3 request batch | 35.352 → 24.244 | 33.834 → 23.955 | 0.69× / 0.71× |
| B5 dispatch | 9.308 → 6.431 | 8.971 → 6.368 | 0.69× / 0.71× |
| B6 order book | 0.040 → 0.141 | 0.040 → 0.147 | 3.49× / 3.65× |
| B8 cache churn | 12.110 → 31.388 | 11.886 → 30.237 | 2.59× / 2.54× |
| B10 concurrent allocation | 1.250 → 2.904 | 0.665 → 1.114 | 2.32× / 1.68× |

B10 varied substantially between runs on this two-vCPU host. A4's full-suite
ratio also differs from the separate 30-repetition collection capture
(4.29× end-to-end); its compact medians were close, while the native median
was about 28% higher in that capture. Use the focused A4 mutation comparison
and its sample profile to locate work, and treat the full-suite ratio as
host-sensitive. The repeated runs support the broader gaps in A4, A5, B6, and
B8, while B3 and B5 remain compact wins.

## Ranked follow-up measurements

These are candidates for the next profiling pass. No optimization is included
in this change.

1. **B8 cache churn — separate collection and release cost.** It has the
   largest measured absolute gap: compact was 18.35–19.28 ms slower per
   end-to-end repetition (2.54–2.59× native). The CPU profile places
   `CompactHashMap::find_slot_in` at 17.78% of compact self samples. The
   allocator sample also sees hash lookup, hashing, header access, and
   `release_many_locked`; its stacks are partly unresolved, so they do not
   rank those costs. Telemetry shows 3.25 pending candidates per active
   lookup and just 50 active lookups without an exact compatible extent,
   which rejects deep pending scans as the obvious dominant cause. First
   measure map mutation and release work separately with a lower-overhead
   capture. Do not infer allocator policy changes from the current evidence.

2. **A5 hash tables — isolate slot search and cage access from hashing.** The
   full-suite compact/native ratio was 3.93–4.11×, with 3.48–3.63 ms of extra
   median time. In the focused collection capture, matching an FNV diagnostic
   builder still left compact map build, hit lookup, and churn at 7.13×,
   8.84×, and 7.21× native. Default hash-only costs were much closer
   (1.14–1.41×). Sampled `find_slot_in`/insertion paths and repeated cage
   accesses are stronger leads than the SIMD control classifier, which was
   only 2.38% of compact A5 CPU samples (about 2.7–2.9% in the collection
   profile). Measure table access and probe-loop cost independently before
   changing the map.

3. **A4 deque — test scoped writable access reuse.** The focused 30-run
   collection capture measured 1,925.50 µs compact versus 459.76 µs native
   for push/pop mutation (4.19×), and 1,984.44 versus 461.98 µs end-to-end
   (4.29×). Compact sampled CPU had `read_header` as its leading exclusive
   symbol at 38.56%. A header-resolution-only probe took 727.29 µs, but it is
   instrumented and must not be subtracted from operation timing. A future
   experiment can test operation-scoped writable access while checking ring
   wrap, growth, and ownership invariants.

4. **B6 order book — isolate indexed vector updates.** Full-suite compact
   medians were 0.141–0.147 ms versus 0.040 ms native, a 3.49–3.65× ratio
   and roughly 0.10 ms extra. The focused probe measured indexed updates at
   61.84 versus 11.36 µs (5.44×); retain was 1.18× and snapshot rebuild
   1.08×. `read_header` was 30.85% of compact sampled CPU. This is a clear
   path-level gap with modest end-to-end absolute cost; measure a safe batched
   update path before considering a change.

5. **A2 allocation — attribute owner/header and allocation work.** Compact
   end-to-end medians were 2.510–2.521 ms versus 1.745–1.912 ms native
   (1.32–1.44×). The CPU profile puts `read_header` at 24.95% of compact
   self samples; the allocator's 200 resolved samples put it at 31% of
   leaves, with `CageAllocation::as_slice` at 18%. The sample is limited, and
   allocator telemetry disables a production cursor fast path, so use a
   low-overhead call-count or sampling capture to distinguish header checks,
   owner access, and allocation. Native samples include the benchmark's
   `CountingAllocator` atomics and are not pure `System` allocator cost.

6. **B10 concurrent allocation — obtain usable worker stacks and lock data.**
   The two full-suite runs disagree materially: compact/native was 2.32× in
   run 1 and 1.68× in run 2. Compact CPU samples show
   `alloc_owned_value` at 52.45% inclusive and an AArch64 CAS helper at
   29.71% self, but native B10 produced no CPU samples. The allocator uprobe
   histogram suggests contention, but instrumentation slowed the workload
   about 25–27× and the lock/transaction pairing was invalid. A lower-overhead
   profiler with worker symbols is needed before ranking contention or making
   allocator changes.

## Memory and representation

The following compares native tracked live bytes after build with compact
cage live bytes after build, alongside whole-process peak RSS from the two
full-suite runs. Cage bytes and RSS measure different things. All benchmark
scenarios returned live allocation accounting to zero after completion.

| Scenario | Native live bytes after build | Compact cage bytes after build | Peak RSS native / compact, run 1 / run 2 (KiB) |
| --- | ---: | ---: | ---: |
| A2 | 600,000 | 900,016 | 3,336 / 3,516; 3,276 / 3,504 |
| A4 | 32,768 | 32,784 | 2,700 / 2,868; 2,704 / 2,868 |
| A5 | 720,912 | 720,960 | 3,344 / 3,476; 3,344 / 3,476 |
| B3 | 13,071,260 | 11,903,936 | 42,344 / 34,700; 42,344 / 34,696 |
| B5 | 4,846,280 | 3,440,672 | 13,644 / 10,972; 13,640 / 10,972 |
| B6 | 49,169 | 49,184 | 2,768 / 2,832; 2,768 / 2,832 |
| B8 | 3,528,488 | 3,620,248 | 35,652 / 35,144; 35,648 / 35,144 |
| B10 | 256,048 | 256,032 | 3,156 / 3,272; 3,140 / 3,288 |

The compact cage is about 50% larger than native live data in A2, close in
A4/A5/B6/B10, about 2.6% larger in B8, and about 9% and 29% smaller in B3
and B5. RSS is slightly higher for compact in A2/A4/A5/B6/B10 and lower in
B3/B5/B8. B8 used roughly 35 MiB process RSS, including workload fixtures,
data, runtime, and allocator state; the configured 128 MiB virtual cage
reservation is not resident memory.

The frozen representation measurements were not changed: 4-byte compact
owners, 12-byte deque, 16-byte allocation header, 8-byte frozen descriptors,
single cage, and u32 offset ownership. The near-full allocator probe used a
separate 16 MiB cage: a 15 MiB payload occupied 15,728,656 bytes including
header/padding; an expected 2 MiB failure left accounting unchanged; after
drop, a 1 MiB recovery allocation succeeded; final live bytes returned to
zero and allocator validation passed. It did not fault in the payload or
measure RSS.

## Evidence limits and rejected explanations

- The machine was a shared two-vCPU AArch64 Neoverse-N1 VM running Linux
  `6.17.0-1020-oracle`, Rust/Cargo `1.95.0`, and LLVM `22.1.2`. Results should
  not be generalized to other CPUs or workloads.
- CPU and collection profiles use different repetition counts and separately
  instrumented binaries. Their samples locate functions; their timings do
  not replace the uninstrumented full-suite medians.
- Allocator stack captures have only 144–246 weighted samples per scenario;
  B8 is partly unresolved and B10 wholly unresolved. Telemetry totals overlap,
  and its feature changes A2's cursor-allocation path. Uprobes strongly
  distort runtime and their critical-section histogram is invalid.
- The shared benchmark `CountingAllocator` adds atomic cost to native
  allocation scenarios. This affects the baseline, especially native A2,
  B3, and B5 profiles, but the harness was kept identical for both variants.
- Global size-class reuse is not an explanation for these policy-B runs:
  telemetry observed zero size-class hits. The A5 SIMD classifier is also a
  low-priority lead. The shallow B8 pending scans do not support a deep-scan
  bottleneck claim.
- Instrumentation timing totals overlap and inclusive CPU call paths overlap;
  they are not additive. No instrumented wall time is used as a production
  performance result.

## Reproduction artifacts

The full-suite outputs are `/tmp/profile-baseline-run1.tsv` and
`/tmp/profile-baseline-run2.tsv`; the final targeted release run is
`/tmp/profile-targeted-final.tsv`. CPU raw captures and reports are under
`/tmp/csl-v24-cpu-profile/`, allocator captures under
`/tmp/compact-allocator-profile-v2.4/`, and collection captures under
`/tmp/csl-collection-profile/`. Machine-specific raw profiles remain outside
the repository. The checked-in workstream scripts record the commands and
reproduction settings.

## Round-two update

A second profiling round (`PROFILE_V2_4_ROUND2.md`) re-measured the tree after
the round-one optimizations and after the benchmark-methodology change. It
supersedes the numbers above in two ways:

- The headline ratios recorded here were captured with the harness
  `CountingAllocator` installed. That counter charges its atomic cost to the
  native variant only, because compact collections allocate from the cage
  rather than the global allocator, so the ratios above are instrumented and
  systematically flatter compact. The production-representative
  (accounting-free) ratios are A2 1.53x, A4 3.58x, A5 2.78x, B6 2.34x,
  B8 2.36x, B10 ~2-9x (host-noise band), B3 0.68x, B5 0.75x.
- Per-process sampling never captured the B10 worker threads; only
  system-wide capture does. The B10 concurrency path is now attributed with
  valid low-overhead samples: roughly 60% lock/futex/atomic machinery versus
  native malloc/cfree in per-thread tcaches.

`read_header` has zero out-of-line symbols on both aarch64 and x86-64, so the
remaining A4/B6/B8 distance is the intrinsic per-operation cage-header
validation under the frozen layouts, not a redundant helper call. No
production change was landed in round two; the candidates and their quantified
ceilings are recorded in `PROFILE_V2_4_ROUND2.md` and `BENCHMARKS.md`.
