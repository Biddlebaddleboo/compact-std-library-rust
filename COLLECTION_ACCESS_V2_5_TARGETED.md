# V2.5 deque/vector access experiment

**Decision: Rejected.** The safe branchless ring-index prototype increased
ordinary A4 FIFO mutation time and instruction count. No production change is
retained. Source baseline: `b3ca878`.

## Candidate

`CompactVecDeque::physical_index` was prototyped with wrapping addition and
subtraction plus a mask to select the wrapped or unwrapped index. The intent
was to remove the conditional from the hot A4 `push_back` path. For its callers,
`head < capacity` and `logical < capacity`; therefore the selected value stays
in range, and wrapping arithmetic makes the unselected intermediate safe on
32-bit targets. The experiment changed no APIs or owner layouts. The only
production symbol touched was `CompactVecDeque::physical_index` in
`crates/compact_collections/src/deque.rs`; that source change was reverted.

## Timing results

I built baseline and candidate release binaries from separate worktrees and
target directories. For A4 and B6, I ran nine alternating baseline/candidate
pairs. Each run used `benchmark_compare --runs 9 --mode profile --order
alternate`; checksums matched across all 36 suite invocations. Values below are
the median of each binary's nine per-phase medians; ranges show those nine
medians. The paired-ratio column is the median of candidate/base ratios within
each pair.

| Scenario / phase | Baseline median (range) | Candidate median (range) | Paired change |
| --- | ---: | ---: | ---: |
| A4 compact mutation | 1,248,530 ns (1,242,609–1,259,849) | 1,265,290 ns (1,260,529–1,269,249) | +1.43% |
| A4 compact end to end | 1,288,730 ns (1,285,490–1,300,570) | 1,307,010 ns (1,305,809–1,310,850) | +1.42% |
| B6 compact updates, retain, clone | 68,041 ns (67,921–68,121) | 67,921 ns (67,720–68,281) | −0.18% |
| B6 compact end to end | 89,120 ns (89,000–89,241) | 89,000 ns (88,800–89,321) | −0.09% |

The A4 candidate range did not overlap the baseline range. B6 exercises vector
indexing and retain/copy paths but does not use the deque helper; its small
difference is measurement noise.

## Hardware counters and memory

Nine alternating `perf stat` pairs ran the accounting-free
`benchmark_profile --window <scenario> compact 9 --seconds 0` command. These
whole-process counts include startup and the scenario runner.

| Scenario / metric | Baseline median | Candidate median | Paired change |
| --- | ---: | ---: | ---: |
| A4 cycles | 40,798,598 | 41,232,067 | +1.06% |
| A4 instructions | 134,782,949 | 137,372,421 | +1.93% |
| A4 process peak RSS | 2,788 KB | 2,788 KB | 0% |
| B6 cycles | 4,697,550 | 4,672,663 | −0.42% |
| B6 instructions | 10,797,179 | 10,840,773 | +0.24% |
| B6 process peak RSS | 2,696 KB | 2,764 KB | +2.5% |

Accounting-enabled A4 and B6 runs showed identical cage counters for baseline
and candidate. A4 mutation had a 32,768-byte live delta, 98,344-byte high-water
cursor, and one 32,784-byte free block. B6 updates had a −6,336-byte live delta,
65,576-byte high-water cursor, and one 22,720-byte free block. Accounting-mode
process peak RSS was 2,788 KB for A4 and 2,892 KB for B6 on both binaries. The
profile-mode B6 RSS difference did not reproduce in accounting mode, and B6
does not call the changed deque helper. End-to-end cage live delta returned to
zero in both scenarios. The existing layout test passed:
`CompactVec<u64>` remains 4 bytes and
`CompactVecDeque<u64>` remains 12 bytes.

No B3/B5 follow-up guard was run because this candidate showed no gain and was
rejected at A4.

## Correctness checks

- `cargo test -p compact_collections --test collection_batch_access --test deque_vector_hot_paths --test deque_view`: 20 passed.
- `cargo test -p compact_collections --test cage_collections`: 22 passed, including owner layout assertions.
- `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p compact_collections --test deque_vector_hot_paths --test deque_view --test collection_batch_access`: 20 passed.
- Profile-mode A4/B6 checksums matched in every baseline and candidate pair.

The existing B6 probe and `collection_batch_access` tests also confirm that
callers can batch per-index updates through `CompactVec::as_mut_slice` with
equivalent native-slice semantics. No new vector API or collection layout is
needed for that access pattern.
