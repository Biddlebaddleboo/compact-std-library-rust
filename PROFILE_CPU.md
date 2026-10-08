# V2.4 CPU profiling report

## Scope and host

Profiles were collected from source commit
`fc0a58bf08aa3bffc577ec2da8151d262b6a5f53` in the isolated
`codex/cpu-profiling` worktree. The only changes on this branch are this
report and `scripts/profile_cpu.sh`; no production source or benchmark harness
was changed.

Host: Linux `6.17.0-1020-oracle` on an AArch64 Neoverse-N1 VM with two logical
CPUs. Toolchain: Rust/Cargo `1.95.0`, LLVM `22.1.2`, stable AArch64 GNU; Linux
`perf 6.17.13`. `kernel.perf_event_paranoid` was `4`; `sudo -n perf` was used
for sampling without changing the sysctl.

The sampled executable was a separate optimized release build with
`-C debuginfo=1 -C force-frame-pointers=yes`. Samples used the user-space
software event `cpu-clock:u` at 499 Hz and frame-pointer call chains. Each
profile launched the benchmark child directly with `--child`, so the parent
runner was not the sampled workload. The build enabled `json,toml`, used the
default allocator policy B, and left `allocator-telemetry` disabled. Native
and compact checksums matched for all eight scenarios.

## Hotspots

Inclusive costs are the largest workload call path in `perf report` after
skipping the benchmark wrapper frames. Exclusive costs are the top self-time
symbol from `perf report --no-children`. CPU seconds are estimated from the
`cpu-clock:u` event count (`event count / 1e9`), multiplied by the reported
sample share. Inclusive paths overlap and must not be added together.

| Scenario | Variant | Samples / sampled CPU seconds | Largest inclusive path (share; CPU seconds) | Top exclusive symbol (share; CPU seconds) |
| --- | --- | ---: | --- | --- |
| A2 | Native | 3,669 / 7.35 s | `Vec::from_iter` (64.27%; 4.73 s) | `__aarch64_ldadd8_relax` (38.32%; 2.82 s) |
| A2 | Compact | 6,629 / 13.28 s | `box_objects` closure (59.27%; 7.87 s) | `read_header` (24.95%; 3.31 s) |
| A4 | Native | 1,013 / 2.03 s | `deque_churn` closure (99.11%; 2.01 s) | `deque_churn` closure (99.11%; 2.01 s) |
| A4 | Compact | 5,205 / 10.43 s | `deque_churn` closure (96.45%; 10.06 s) | `read_header` (38.56%; 4.02 s) |
| A5 | Native | 1,817 / 3.64 s | `hash_churn` closure (62.41%; 2.27 s) | `HashMap::insert` (29.77%; 1.08 s) |
| A5 | Compact | 7,342 / 14.71 s | `hash_churn` closure (60.15%; 8.85 s) | `read_header` (20.81%; 3.06 s) |
| B3 | Native | 3,394 / 6.80 s | `Vec::from_iter` (59.19%; 4.03 s) | `__aarch64_ldadd8_relax` (31.94%; 2.17 s) |
| B3 | Compact | 3,055 / 6.12 s | `request_batch` closure (71.85%; 4.40 s) | `read_header` (15.16%; 0.93 s) |
| B5 | Native | 13,512 / 27.08 s | `dispatch` closure (47.22%; 12.79 s) | `__aarch64_ldadd8_relax` (23.65%; 6.40 s) |
| B5 | Compact | 9,702 / 19.44 s | `dispatch` closure (69.61%; 13.53 s) | `CompactHashMap::find_slot_in` (19.25%; 3.74 s) |
| B6 | Native | 1,220 / 2.44 s | `order_book` closure (79.67%; 1.95 s) | `order_book` closure (70.25%; 1.72 s) |
| B6 | Compact | 4,596 / 9.21 s | `order_book` closure (62.36%; 5.74 s) | `read_header` (30.85%; 2.84 s) |
| B8 | Native | 3,703 / 7.42 s | `cache_churn` closure (87.31%; 6.48 s) | `cache_churn` closure (33.16%; 2.46 s) |
| B8 | Compact | 9,344 / 18.73 s | `cache_churn` closure (84.73%; 15.87 s) | `CompactHashMap::find_slot_in` (17.78%; 3.33 s) |
| B10 | Native | 0 / no sampled CPU time | No samples; no hotspot ranking | No samples; no hotspot ranking |
| B10 | Compact | 1,104 / 2.21 s | `CompactRuntime::alloc_owned_value` (52.45%; 1.16 s) | `__aarch64_cas4_acq` (29.71%; 0.66 s) |

The sampled evidence points to repeated compact allocation-header work in A2,
A4, B3, and B6: `read_header` is their leading compact self-time symbol. A4's
`push_back` path accounts for 50.82% inclusive time in compact A4; header reads
under the backing allocation operations are the largest self cost. B6's
compact order-book loop divides mainly between `retain` (14.27% inclusive)
and `push` (29.59% inclusive), with header reads taking 30.85% exclusive.

Compact A5's classifier is only 2.38% of sampled CPU in that scenario; hash
insertion and header reads cost more. In B5 and B8, `find_slot_in` is the
largest compact self-time symbol at 19.25% and 17.78%. Compact B10 shows an
allocation/locking path: `alloc_owned_value` is the largest useful inclusive
path and the AArch64 compare-and-swap helper is the largest self symbol.

The native A2, B3, and B5 profiles prominently include
`__aarch64_ldadd8_relax`. The shared benchmark harness installs a
`CountingAllocator` and updates atomic counters while these phases run, so
those native costs include harness accounting overhead and are not production
`System` allocator costs. The harness was kept unchanged as required.

B10/native completed and produced the same checksum as compact, but
`perf report` reported that its capture contained no samples, even at 1,000
repetitions. The cause was not established. B10/native therefore has no
hotspot conclusion; B10/compact does.

## Instrumentation timing deltas

Two unprofiled suites were run with the ordinary release executable and the
separate debuginfo/frame-pointer release executable. Each used the harness's
default repetitions. Values below are the median of the two suite medians for
end-to-end time. They measure the build-flag effect without `perf`; they are
not performance results for the library.

| Scenario | Native plain → frame-pointer (delta) | Compact plain → frame-pointer (delta) |
| --- | ---: | ---: |
| A2 | 1.751 → 1.731 ms (−1.1%) | 2.520 → 2.646 ms (+5.0%) |
| A4 | 0.360 → 0.369 ms (+2.4%) | 2.301 → 2.077 ms (−9.8%) |
| A5 | 1.175 → 1.212 ms (+3.1%) | 4.691 → 4.876 ms (+4.0%) |
| B3 | 34.033 → 34.733 ms (+2.1%) | 23.612 → 24.521 ms (+3.9%) |
| B5 | 8.898 → 8.937 ms (+0.4%) | 6.479 → 6.459 ms (−0.3%) |
| B6 | 0.040 → 0.039 ms (−3.9%) | 0.141 → 0.150 ms (+6.1%) |
| B8 | 12.039 → 12.081 ms (+0.3%) | 30.359 → 30.726 ms (+1.2%) |
| B10 | 0.819 → 0.966 ms (+17.9%) | 3.047 → 3.044 ms (−0.1%) |

The flag deltas vary in both directions. Most are within 6.1%; A4/compact
(−9.8%) and B10/native (+17.9%, about 0.15 ms) are larger outliers. Two
suite measurements per build are too few to separate those changes from host
and run-to-run variation.

Within the sampled runs, the child-reported end-to-end median was generally
close to the frame-pointer build without `perf`: among captures with samples,
the differences ranged from −2.3% to +4.9%. B10/native was +29.2% in its
1,000-repetition timer result but had no CPU samples, so that timing is also
inconclusive. B10/compact differed by −1.8%. These are sanity checks for
sampling perturbation, not corrected benchmark numbers.

## Generated-code observations

The AArch64 and x86-64 Apple assembly files already present in the workspace
were generated from the same production sources as `fc0a58b` (the later
`b9b7551` revision adds plan documents only):

- AArch64 hash classifier:
  `target/release/deps/compact_collections-d404e767d2ad2fad.s`,
  `classify_control_group` around line 10905. The full 16-byte path uses one
  `ldr q1` followed by NEON byte comparisons and reductions (`cmeq`, `uaddlp`).
- x86-64 hash classifier:
  `target/x86_64-apple-darwin/release/deps/compact_collections-7f92e73c0a64675a.s`,
  around line 9440. It uses `movdqu`, `pcmpeqb`, and `pmovmskb` for the full
  group path.
- AArch64 deque append:
  `target/release/deps/deque_vector_hot_paths-51eb23582e4c2911.s`,
  `push_back` around line 2147. The available-capacity path checks capacity
  once and stores directly with `str`; it has no per-element bounds-check
  panic path.
- x86-64 deque append:
  `target/x86_64-apple-darwin/release/deps/deque_vector_hot_paths-df1a95d6792dd4b3.s`,
  around line 1864. It uses a capacity branch and direct indexed `movl` store.

There are no `udiv`/`sdiv`/`idiv` instructions in the inspected collection
assembly files. Deque ring indexing uses one compare and subtract to wrap;
hash-table indices use the power-of-two mask. The profile's classifier cost in
A5 is small, consistent with the SIMD path not being the dominant cost there.
This was a targeted inspection of the profiled collection paths, not a full
disassembly audit of every crate or libc.

## Reproduction and artifacts

From this source commit, `scripts/profile_cpu.sh` records host metadata, builds
ordinary and frame-pointer release examples, runs two timing suites, profiles
each child directly, checks paired checksums, and exports inclusive/self
reports. On this host, `PROFILE_USE_SUDO=1` is needed for `perf`; the sysctl
was left unchanged.

```sh
PROFILE_OUT=/tmp/csl-v24-cpu-profile bash scripts/profile_cpu.sh metadata
PROFILE_OUT=/tmp/csl-v24-cpu-profile bash scripts/profile_cpu.sh build
PROFILE_OUT=/tmp/csl-v24-cpu-profile bash scripts/profile_cpu.sh baseline
PROFILE_OUT=/tmp/csl-v24-cpu-profile PROFILE_USE_SUDO=1 \
  bash scripts/profile_cpu.sh profile
PROFILE_OUT=/tmp/csl-v24-cpu-profile bash scripts/profile_cpu.sh reports
```

To profile selected scenarios only, set `PROFILE_SCENARIOS=A2,B6`, for
example. Profile repetition counts used here were A2/A4 `5,000`, A5 `3,000`,
B3 `250`, B5 `3,000`, B6 `60,000`, B8 `600`, and B10 `1,000` for each
variant. Every completed child reported the native/compact logical checksum.

The actual run wrote:

- Host/toolchain metadata: `/tmp/csl-v24-cpu-profile/host.txt`
- Ordinary and frame-pointer timing TSVs/logs:
  `/tmp/csl-v24-cpu-profile/baseline/`
- Raw perf captures (about 15 MiB total):
  `/tmp/csl-v24-cpu-profile/perf-data/`
- Direct-child logs, including checksums, sample counts, and `PHASE` timings:
  `/tmp/csl-v24-cpu-profile/profile-logs/`
- Symbolized inclusive and self reports:
  `/tmp/csl-v24-cpu-profile/reports/`
- Frame-pointer release binary:
  `/tmp/csl-v24-cpu-profile/target-profiled/release/examples/benchmark_compare-510c4a9c8b6a9422`

The ordinary release binary used for the measurements was the existing
no-telemetry release artifact
`/home/ubuntu/Projects/compact-std-library-rust/target/release/examples/benchmark_compare-c5f69250867ef18a`.
Its Cargo fingerprint had the same `default,json,serde,toml` features and no
`RUSTFLAGS`; revision `b9b7551` contains no production changes relative to the
pinned source. The script can instead build a fresh ordinary binary with
`BUILD_PLAIN=1` (the default).

## Limitations and next investigations

- Results are from one shared two-vCPU ARM VM. The sample event is software
  CPU time, not hardware cycles or instructions; CPU seconds are approximate.
- Captures used 499 Hz. The sample counts range from 1,013 to 13,512 for
  successful profiles; B10/native has no samples. Raw data and symbolized
  reports are kept in `/tmp`, not committed.
- Some glibc frames remain raw addresses because system-library debug symbols
  were unavailable. Rust application frames were symbolized.
- Native profiles include the benchmark `CountingAllocator` atomics. The
  harness setup also includes one warm-up and timer/accounting code in each
  child; profile totals are not exclusively the timed phase bodies.
- Inclusive values overlap with their callees. No inclusive percentages were
  summed as independent costs.

The strongest follow-up candidates are to measure whether the repeated
`read_header` validations in compact A2/A4/B3/B6 can be safely reused within
one operation, and to examine allocator lock/atomic cost in compact B10. Hash
probing deserves attention in B5/B8, while the A5 SIMD classifier is a lower
priority on this host. B10/native needs a valid sample capture before making a
native-vs-compact CPU comparison.
