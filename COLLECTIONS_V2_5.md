# V2.5 collection workstream

## Finding

The all-16 capture at `ec68fb7` keeps A4, A5, and B8 as measurable collection gaps. Its timing suite has two pairs, so p95 values below are descriptive only. Compact/native median latency was 1.397/0.427 ms for A4 (3.27x), 3.457/1.328 ms for A5 (2.60x), and 28.646/13.730 ms for B8 (2.09x). The corresponding compact CPU profiles contained 3K, 4K, and 7K samples, with zero lost samples.

| Scenario | Largest compact collection symbols | Other visible cost |
| --- | --- | --- |
| A4 FIFO churn | `CompactVecDeque::push_back` 58.82%; `pop_front` 37.40% | Repeated ring operations dominate. |
| A5 map/set churn | `CompactHashSet::insert` 25.30%; `CompactHashMap::insert` 25.19%; `classify_control_group` 13.21% | `SipHasher24::finish` accounts for 10.43%. |
| B8 fixed-population cache churn | `CompactHashMap::insert` 18.99%; `find_index_in` 10.88%; `classify_control_group` 6.83% | The inclusive `CompactBytes::from_slice` subtree is about 19%; allocation and release machinery also appears and belongs to the allocator workstream. |

The profiled mechanisms are visible in [deque.rs](crates/compact_collections/src/deque.rs): ordinary `push_back` and `pop_front` each call `CageAllocation::uninit_capacity_mut` for every operation. That resolves the backing allocation before computing/updating the ring index. The existing `CompactVecDeque::with_view` resolves storage once for an exclusive borrow and runs the same ring operations against the view. The hash profiles do not justify another hash-table layout experiment: the earlier seven-bit fingerprint prototype preserved the randomized hasher but delivered negligible end-to-end gains and was rejected. Do not weaken `CompactBuildHasher`'s randomized SipHash default.

## Focused A4 candidate experiment

Candidate: for bounded FIFO batches, reserve the known spare slot once and use the existing `CompactVecDeque::with_view` path for repeated `push_back`/`pop_front`. This is an opt-in caller pattern, not a proposed default-API or owner-layout change.

The checked-in `collection_batch_probe` was rerun on this branch with `--scenario A4 --runs 11`, a 4,096-element full initial ring, 80,000 operations, and a timed `reserve(1)` on the view path. All arms produced checksum `3199960000`.

| A4 shared-shape path | Median / p95, ns | Relative to compact per-op median |
| --- | ---: | ---: |
| Native per-op | 301,722 / 338,163 | — |
| Compact per-op | 1,208,809 / 1,773,613 | 1.00x |
| Compact `with_view`, including `reserve(1)` | 143,122 / 149,881 | 8.45x faster |

The view path was also measured in a temporary mode-isolated probe: nine `perf stat --repeat` process runs, each with 30 timed repetitions. Median-of-process medians were 1,205,729 ns for compact per-op and 143,081 ns for compact view; every run produced the same checksum. Raw aggregate counters from those paired captures:

| Path | Cycles | Instructions | Branches | Branch misses | Cache misses |
| --- | ---: | ---: | ---: | ---: | ---: |
| Compact per-op | 113,100,955 | 386,337,159 | 89,071,126 | 17,237 | 110,824 |
| Compact view | 17,436,876 | 52,339,288 | 5,008,350 | 13,916 | 47,283 |

Capture files: `/tmp/csl-v25-collections-a4-counter/compact-perop.{workload.tsv,perf-stat.csv}` and `/tmp/csl-v25-collections-a4-counter/compact-view.{workload.tsv,perf-stat.csv}`. The path-isolated counters are diagnostic; the all-16 A4 profile and scenario-level counters remain the suite authority.

This clears the isolated timing hypothesis: amortizing resolution cuts the measured loop by about 8.4x with identical behavior. It does **not** establish a framework-wide production win, because callers must know the maximum live length and reserve before borrowing. No production change is recommended in this workstream. The public `with_view` API already exposes this optimization, and the focused probe is evidence for callers with bounded batches.

## Semantics and memory constraints

- In the shared-shape probe, the ordinary path grows on its first push; the view path times `reserve(1)` before opening the view. Both grow a 4,096-slot ring to 8,192 slots and peak at length 4,097. The candidate adds no retained allocation or owner metadata relative to that same reservation; no exact-path RSS/retained-byte sample was taken.
- `with_view` holds the deque's exclusive borrow, exposes only a borrow-scoped resolved slot slice, and stores no pointer in the owner. The twelve-byte deque layout remains unchanged.
- A view cannot grow. Callers must reserve sufficient capacity; pushing at capacity returns `AllocationExhausted` without changing the deque. `CompactVecDequeView::drop` writes its head/length back on return and unwind.
- Any future API/internal change must preserve wrapped-index behavior, ZST and drop behavior, allocation-failure semantics, panic recovery, and the existing Miri coverage (`deque_view` and `collection_batch_access`).

For A5/B8, keep the randomized hash seed, tombstone/probe order, panic-safe `Hash`/`Eq`, and entry initialization/drop behavior. B8's payload construction and release costs mean a hash-control change alone would not address its largest visible cost.

## Next measurement protocol

1. Keep the current production collection code and owner layouts unchanged until the central ownership/interface and memory gates are approved.
2. For each proposed bounded-batch call site, compare ordinary and view paths from the same pre-state and capacity, with growth cost included in both paths. Alternate at least 11 runs, require identical checksums, and record median/p95 plus cycles, instructions, and branch/cache misses.
3. Record retained bytes, high-water cursor, peak RSS, and final capacity for the exact same workload. Reject a path migration if it hides over-reservation, increases retained bytes beyond the 2% budget, or raises peak RSS beyond 5%.
4. Re-run differential and Miri coverage for wrapping, reserve/growth, exhausted capacity, ZST, drops, and unwind before integrating any new implementation surface. Re-run all 16 scenarios to ensure A5/B8 and already-fast workloads do not regress.

## Scope and state

Only this report is intended for the collection workstream. No production source or shared benchmark harness changes are proposed. The temporary counter example used for the isolated capture was removed.
