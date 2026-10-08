# PLAN_HASH_PROBING.md — A5/B8 probe, fingerprints, classifier

## Implementation scope
Own crates/compact_collections/src/hash_map.rs and hash_control.rs plus dedicated hash tests. Inspect CompactHashMap::{find_index_in,find_slot_in,insert,get,get_mut,remove_entry,rehash}, classify_control_group, first_empty_slot, scalar/NEON/SSE2 classification. Read A5/B8/B5 scenarios only.

## Verified facts
Round 3 introduced lookup-only find_index_in and achieved ~3–4% A5 improvement. Control bytes encode occupancy/tombstones, not hash fingerprints. find_slot_in and classify_control_group remain sampled CPU costs. Default hasher is randomized and must stay collision resistant.

## Phase 1: current probe attribution
Profile hits/misses, insert/replace/remove, tombstone churn, load factor and hash collisions on short/long/scalar keys; count control groups, equality calls, branches and cache effects. Compare unchanged native and compact algorithms with identical checksums.

## Phase 2: minimal probe optimizations
Isolated first-group, lane mask, width computation, wrapped versus contiguous group, unnecessary branch/load and SIMD dispatch experiments. Check emitted AArch64/x86-64 assembly, missing instruction support and scalar parity. Do not change probe termination at first EMPTY, tombstone insertion semantics, or Hash/Eq panic safety.

## Phase 3: gated fingerprint prototype
Prototype metadata using a short fragment of randomized hash in FULL controls with distinct EMPTY/TOMBSTONE markers; skip full key comparison on fingerprint mismatch. Explicitly account for metadata format, allocation/retained memory, load factor, tombstone replacement, rehash, initialization/cleanup and existing compat assumptions. Compare short, long and adversarial keys, B5/B8, scalar/NEON/SSE2. Keep isolated unless compelling overall wins, no security reduction and acceptable memory overhead. No silent default switch to FNV.

## Phase 4: B8 integration
Separate map lookup/remove/insert from allocator release and verify benefit to B8 after each accepted change. Avoid changing allocator policy in this workstream.

## Tests and handoff
Native HashMap differential random operations, all tombstone/full/wrapped cases, hash/eq panics, collision flood, allocation failure, drop-once, rehash, scalar/NEON/SSE2 parity and Miri. Report changed files, SHA, probe/equality counts, timings, RSS/retained memory, tests and rejected experiments.
