# V2.5 targeted implementation result

## Outcome

The targeted plan was pinned to `8bfe6f7`; implementation work started from
`b3ca878`, which adds the plan files but no runtime source changes. One
production optimization passed review and the V2.5 gates: private,
allocator-issued owner reads now retain generic header checks while skipping a
redundant typed-payload extent check. The four-byte owner, 16-byte header,
12-byte deque, public APIs, and raw-offset validation remain unchanged.

The accepted fast path is integrated at `bdeccd9`; real over-aligned
allocation/resize/pending-reuse coverage was added at `c941e9`.

The owner-affine chunk allocator remains **Blocked** before unsafe
implementation. Its phase-one state model now rejects mutator-to-mutator chunk
transfer and models a remote-release pin across owner retirement, but production
offset provenance, registry synchronization, bounded pending-release storage,
TLS/reaper recovery, coherent statistics, and chunk memory caps are still open.

The temporary `PLAN*.md` files are removed in this final implementation commit.
The workstream results remain in [PROFILE_V2_5_BASELINE.md](PROFILE_V2_5_BASELINE.md),
[PROFILE_V2_5_MEMORY.md](PROFILE_V2_5_MEMORY.md),
[ALLOCATOR_PHASE1_V2_5.md](ALLOCATOR_PHASE1_V2_5.md),
[ALLOCATOR_OWNER_HEADER_V2_5_RESULTS.md](ALLOCATOR_OWNER_HEADER_V2_5_RESULTS.md),
[HASH_V2_5.md](HASH_V2_5.md), and
[COLLECTION_ACCESS_V2_5_TARGETED.md](COLLECTION_ACCESS_V2_5_TARGETED.md).

## Accepted owner-header fast path

`read_owner_header<T>` accepts a reference to the private `CageAllocation<T>`
owner and extracts its non-zero offset. It retains `read_header`'s cage,
allocation-block, and initialization checks, and relies on the payload-fit
invariant established by the allocator's sole owner constructor and checked
resize path. Owner fields are private; there is no safe owner reconstruction,
`Clone`, or `Copy` path. The source audit covers both header initialization
paths, resize writes, and initialized-length guards.

The fast path is limited to private owner accessors (`header`, `resolved`,
`resolved_mut`, and `try_resize`). `read_typed_header<T>` still performs the
full typed extent validation for typed raw-offset resolution and the
`validate_owned` diagnostic. `resolve_bytes_unchecked` keeps its generic
header and byte-length checks, and drop keeps the type-erased release checks.
The new integration coverage uses a real over-aligned owner through fresh
allocation, growth, shrink, and pending reuse. Header-layout and owner-layout
assertions remain in force.

### Compact timing results

The full comparison used two alternating accounting-free suites from the
pinned baseline and candidate binaries. Values are per scenario repetition;
with two suites, p95 is the larger observation. Every native/compact checksum
matched.

| Scenario | Compact baseline median / p95 (ms) | Candidate median / p95 (ms) | Median change |
| --- | ---: | ---: | ---: |
| A1 | 0.099 / 0.102 | 0.099 / 0.100 | +0.46% |
| A2 | 1.952 / 1.968 | 1.790 / 1.794 | −8.29% |
| A3 | 1.270 / 1.421 | 1.066 / 1.127 | −16.10% |
| A4 | 1.392 / 1.393 | 1.159 / 1.162 | −16.78% |
| A5 | 3.395 / 3.415 | 3.371 / 3.484 | −0.69% |
| A6 | 1.453 / 1.455 | 1.467 / 1.508 | +0.98% |
| B1 | 18.620 / 18.874 | 18.072 / 18.098 | −2.94% |
| B2 | 1.177 / 1.178 | 1.181 / 1.188 | +0.31% |
| B3 | 22.936 / 22.944 | 22.797 / 22.831 | −0.61% |
| B4 | 9.292 / 9.322 | 9.388 / 9.519 | +1.03% |
| B5 | 7.556 / 7.578 | 7.469 / 7.507 | −1.16% |
| B6 | 0.116 / 0.116 | 0.105 / 0.106 | −9.16% |
| B7 | 20.902 / 21.023 | 20.335 / 20.524 | −2.71% |
| B8 | 28.349 / 28.500 | 27.408 / 27.520 | −3.32% |
| B9 | 7.678 / 8.023 | 7.664 / 7.997 | −0.19% |
| B10 | 3.256 / 3.434 | 2.476 / 3.079 | −23.94% |

The focused nine-pair runs also improved A2 by 8.99%, A4 by 17.73%, and B6
by 11.24%. B3 and B5, already compact wins, improved by 1.06% and 3.73% in
that focused run. Their full-suite shifts were below 1.2%.

Three-repeat compact-only hardware counters support the A2/A4/B6 changes:

| Scenario | Cycles | Instructions | Branches | Branch misses | Cache misses |
| --- | ---: | ---: | ---: | ---: | ---: |
| A2 | −9.41% | −14.48% | −9.14% | −13.45% | +3.47% |
| A4 | −18.08% | −17.27% | −11.06% | −90.86% | −8.01% |
| B6 | −11.69% | −13.58% | −8.04% | −5.71% | −1.81% |
| B10 | −0.37% | −3.69% | −3.04% | +5.56% | +2.72% |

B10 timing remains noisy. Its two-suite full-profile median improved 23.94%,
but a separate nine-pair zero-second run regressed 10.56%, and nine paired
0.5-second windows improved 10.18%. The 0.5-second compact counters were
nearly flat (−0.09% cycles, −2.08% instructions); native counters varied by
16–20% between builds. Treat B10's candidate effect as inconclusive and keep
it under observation on a multicore host.

### Memory and compatibility

All sixteen scenarios had identical candidate/baseline peak requested and
cage high-water bytes. Every compact scenario returned to zero retained cage
bytes. Two-run profile RSS medians differed by −132 to +10 KiB. A single
measure-mode capture ranged from −60 to +132 KiB, with the largest percentage
increase 4.57% on the very small B6 baseline; this did not reproduce in the
profile-mode RSS or cage measurements. No owner, header, deque, frozen
descriptor, serialized format, or public API layout changed. No memory-budget
exception is requested.

## Rejected and blocked experiments

| Workstream | Decision | Evidence |
| --- | --- | --- |
| Hash classifier | Rejected | The EMPTY-only SIMD classifier changed compact medians by −1.14% A5, −0.30% B5, and −0.43% B8; B8 p95 worsened 4.07%. Retained/high-water bytes were identical, B8 RSS rose 4 KiB, and cycles/instructions did not improve meaningfully in A5/B5. Randomized hashing and control layout remain unchanged. |
| Branchless deque index | Rejected | Nine paired A4 runs regressed compact mutation 1.43%, end-to-end 1.42%, and instructions 1.93%; RSS and allocator accounting were unchanged. B6 was effectively unchanged. |
| Owner-affine chunks | Blocked before production code | The sequential model is not a synchronization proof. Open gates include actual owner/release provenance, registry pin/removal ordering, bounded no-allocation remote releases, TLS/reaper/poison recovery, coherent stats, and a measured chunk/metadata budget. Phase-two unsafe code was not started. |
| Generic resolved-view type | Deferred | Existing borrowed slices, `CompactVec::as_mut_slice`, and `CompactVecDeque::with_view` already provide the measured batch behavior; no incremental benefit justified a new public cross-crate type. |

## Validation

Passed on the integrated source:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-features --locked`
- `cargo test --workspace --all-features --locked`, including owner-layout,
  raw typed-offset, and real aligned-owner resize/reuse coverage
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- Full `.github/workflows/miri.yml` coverage: 86 passed, 0 failed, including
  the new owner-header tests and aligned-owner integration path
- `cargo check --workspace --all-features --locked --target x86_64-apple-darwin`
- `.github/workflows/harness.yml` checksum parity: all 16 scenarios passed
- Nine alternating A/B pairs on A2/A4/B6/B3/B5; two alternating full 16-scenario
  profile suites; all checksum pairs matched

Clippy initially encountered `ENOSPC` while writing Cargo's query cache. The
authorized Cargo target-cache cleanup freed space; the strict Clippy rerun
passed. No sanitizer or physical multicore run was available on this host.
