# PLAN_HASH.md — Structural Hash-Table Optimization

## Scope and facts
Own `crates/compact_collections/src/hash_map.rs`, `hash_control.rs`, hash-specific tests. Start `CompactHashMap::{find_slot_in,find_index_in,insert,get,get_mut,remove_entry,rehash}`, `classify_control_group`, `first_empty_slot`; inspect scalar/NEON/SSE2 classifiers. Lookup-only probe already exists. Prior 7-bit fingerprint and EMPTY-only SIMD classifier gave insufficient integrated wins; insertion and probing remain A5/B8/B5 CPU costs. Randomized collision-resistant default hashing must remain.

## Experiments
1. Measure hash computation, initial/group probes, equality counts, tombstones, replacement/remove, rehash/growth, iteration, allocator share. Use realistic short/long and adversarial-collision keys and whole-workload A5/B8/B5 phase attribution.
2. Isolate candidate insertion-probe and first-group paths, repeated-classifier avoidance, control access/wrapped scan improvements, tombstone and capacity policies; inspect AArch64 assembly and scalar/NEON/SSE2 behavior. Do not rerun rejected classifier/fingerprint without a genuinely different hypothesis.
3. Only if locality profiling supports, prototype control/entry alternative storage layout; quantify per-entry metadata, allocation count, cache misses, iteration/rehash/destructors, retained bytes and peak RSS.
4. If hash computation demonstrably dominates, compare only collision-resistant randomized alternatives under adversarial keys; never substitute insecure deterministic hashing to win a benchmark.

## Safety/tests/acceptance
Preserve FULL/EMPTY/TOMBSTONE, stop at first EMPTY, MaybeUninit guarantees, Hash/Eq panic safety, collision resistance, exact drop counts, compatible scalar/SIMD behavior. Differential std HashMap random sequences, tombstones/wrap, rehash, invalid control bytes, Miri and cross-target checks. A/B paired timing and memory for A5/B8/B5 plus B3 and all-16 final suite. Reject micro-only gains or material B5/p95 regressions. Default ≤+2% retained, ≤+5% peak RSS. Report exact symbols, SHA, tests, absolute/runtime deltas and decisions.
