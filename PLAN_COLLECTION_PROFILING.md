# PLAN_COLLECTION_PROFILING.md — A4/A5/B6 hot-path diagnosis

## Objective
Measure and localize remaining collection overhead without implementing optimizations. Baseline at `fc0a58b`: A4 5.05–5.55x, A5 3.96–4.00x, B6 3.51–3.52x native.

## A4 deque
Start at `crates/compact_collections/src/deque.rs`, particularly `CompactVecDeque::{push_back,push_front,pop_front,pop_back,physical_index,reserve}` and the `scenarios.rs::deque_churn` benchmark.

Measure independent costs of storage/header resolution, index/wrap calculation, writes/reads, and metadata updates. Compare push-only, pop-only, alternating churn and wrapped/contiguous cases with matched populations and capacities. Use sampling and focused no-behavior-change diagnostic probes; do not assume offset resolution dominates.

## A5 hash
Start at `crates/compact_collections/src/hash_map.rs` and `hash_control.rs`. Inspect `hash`, `find_slot_in`, `classify_control_group`, `first_empty_slot`, `ensure_insert_capacity`, `rehash`, scalar control classifier, NEON and SSE2.

Separate SipHash calculation, probing, equality comparisons, control classification, rehash allocations and tombstone costs. Include scalar keys, short/long string keys, collision patterns and custom hashers with equivalent native semantics. Preserve randomized default hashing and control-byte invariants. Report groups probed, full comparisons, tombstone rate and rehash frequency without polluting the timed build.

## B6 order book
Start at `crates/compact_collections/src/vec.rs` (`CompactVec::retain`, `try_clone_copy`, indexing, slice access) and `scenarios.rs::order_book`.

B6 end-to-end is approximately 141 microseconds. Repeat enough to resolve quote updates, retain, snapshot rebuilding, snapshot copying, reads and drop. Compare matched native operations, and identify whether indexed accesses, retain, allocation or copy dominate. Do not assume retain remains the bottleneck, and do not overinterpret microsecond noise.

## Correctness and methodology
Compare native/compact at the same sizes, work volume and checksums; run sampling separately from uninstrumented timing. Report exact functions, inclusive/exclusive CPU cost, phase-level medians/p95, assembly findings, retained bytes and peak RSS where available. No public API/layout changes, prefetch, new assembly, or benchmark-only fast paths.

## Write scope and handoff
Produce `PROFILE_COLLECTIONS.md`, with reproducible commands, ranked collection hotspots and evidence-backed candidates. Production source is read-only during profiling; shared benchmark harness modifications require central ownership. Report any scripts, changed files, SHA, tests, assumptions and unresolved questions.
