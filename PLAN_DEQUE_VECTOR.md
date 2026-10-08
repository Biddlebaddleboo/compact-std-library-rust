# PLAN_DEQUE_VECTOR.md — A4 deque and B6 compact vector

## Verified baseline
At main 4a57dc7, A4 deque is ~5.65–6.16x native (about 2.0–2.26 ms); B6 order-book ~3.68–3.70x native (~0.14 ms). Deque push_back and push_front already avoid reserve on no-growth paths but still resolve header/storage per operation. CompactVec::retain has single-slice no-drop compaction; drop-bearing T uses Vec<T> native staging with restoration guards. PriceLevel: Copy; B6 already uses public retain and try_clone_copy.

## Implementation scope
Own crates/compact_collections/src/deque.rs (CompactVecDeque::{push_back,push_front,pop_front,pop_back,reserve,physical_index}) and crates/compact_collections/src/vec.rs (CompactVec::{retain,try_clone,try_clone_copy}, RetainNoDropGuard, RetainDropGuard), plus relevant dedicated tests. Read-only benchmarks: crates/compact_std/examples/benchmark_compare/scenarios.rs, deque_churn, order_book, PriceLevel.

## A4
Measure each part of steady-state push/pop: cage header resolution, capacity test, ring index, memory access and metadata updates. Evaluate one-resolution push/pop and minimized checked indexing/wrap logic while retaining safe public API and 12-byte deque. Explore a genuinely useful batch mutation API only when realistic callers benefit; never a benchmark-only bypass. Keep growth/reserve slow path authoritative. Avoid a retained capacity field.

## B6
Split quote updates, retain, clone/snapshot rebuild, snapshot copy, reads and teardown. Reduce repeated indexing resolution by using one borrowed mutable slice over a batch of updates, without holding invalidated references across resize. Retain Copy bulk clone, evaluate fewer needless initialized-header writes, temporary allocations and compaction passes. For needs_drop<T>(), measure staging overhead; only switch to a guarded in-place approach if precise predicate order, destructor order, panic recovery and single-drop invariants are maintained. Do not assume CompactValue implies Copy and do not ship unstable specialization.

## Tests
Deque empty/full, front/back, wrap, growth, repeated alternating churn, iterator order, ZST if supported and drop-once; native VecDeque differential randomized sequences. Vector retain all/none/alternating, Copy/non-Copy, predicate panic, destructor panic, survivor stable order, no invalid stale references and native Vec::retain differential checks. Miri targeted tests.

## Validation/acceptance
Baseline versus target A4/B6 per-phase medians/p95 and end-to-end, retained ratios and checksums. B6 must report absolute microseconds as well as ratios. Revert complexity without significant measured benefit. No layout/pointer changes, prefetch or new assembly. Report SHA, files, tests, measurements, deviations and risks.
