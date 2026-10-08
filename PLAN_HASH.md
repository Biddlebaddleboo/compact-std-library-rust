# PLAN_HASH.md — Lookup, Probing and Fingerprint Experiments

## Scope
Own crates/compact_collections/src/hash_map.rs and hash_control.rs, dedicated hash tests. Start CompactHashMap::{find_slot_in,get,get_key_value,get_mut,insert,remove_entry,rehash}, classify_control_group and first_empty_slot; read A5/B8/B5 benchmark scenarios.

## Verified facts
find_slot_in tracks first tombstone for insertion while get/lookup does not need an insertion slot; each occupied candidate checks full key equality without a stored hash fingerprint. Profiles show probe and classifier costs, but any proposed optimization remains experimental.

## Experiment 1: lookup-only probe
Implement internal read-only slot lookup returning index or absence, excluding insertion tombstone tracking. Use get/get_key_value/get_mut/remove_entry where semantics permit; maintain early stop at true empty and correct wrapped/partial scans. A/B benchmark hit/miss, short/long probes, A5/B8/B5. Preserve panic safety and borrowing.

## Experiment 2: probe loop
Measure mask, lane masks, group-width computation, branch structure, classifier and first-group paths. Inspect AArch64/x86-64 assembly and CPU samples. Implement only proven reductions without out-of-bounds SIMD loads, changed probe termination or duplicated equality comparisons.

## Experiment 3: fingerprint representation
In isolated branch, prototype short hash fingerprints in occupied control bytes with distinct EMPTY/TOMBSTONE markers to reduce full key equality. Must preserve 4-byte owners, validate table metadata memory, probing/collision behavior and randomized hash-flood defense. Compare memory and CPU across scalar and string keys, adversarial collisions, loads and B5/B8. Treat as gated redesign; reject if capacity or memory tradeoff unacceptable. No insecure default hash.

## Optional hashing/SIMD
Profile SipHasher24::finish and NEON/SSE2 classify separately; change only if measurable. Preserve randomized secure default, scalar oracle, partial/wrap correctness.

## Tests/acceptance
Differential randomized HashMap ops, lookup hit/miss, collisions, tombstones, rehash/growth, Hash/Eq panic, drop-once, allocation errors, SIMD/scalar parity and Miri. Keep repeatable wins in A5/B8 without B5 regression. Handoff changes, SHA, timings, RSS, tests and rejected variants.
