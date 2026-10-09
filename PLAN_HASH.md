# PLAN_HASH.md — Hash Layout and Probing Experiments

## Objective / scope
Reduce A5/B8 whole-workload overhead while protecting B5. Own `crates/compact_collections/src/{hash_map.rs,hash_control.rs}` and dedicated tests. Start `CompactHashMap::{find_slot_in,find_index_in,insert,get,get_mut,remove_entry,rehash}`, `classify_control_group`, `first_empty_slot`, scalar/SSE2/NEON control classifiers.

## Verified facts
Lookup-only probe exists. Insertion/classifier overhead prominent across A5/B8/B5. Round 4 7-bit fingerprints had negligible whole-workload gain and were rejected. Default process-randomized hash has collision resistance; retain.

## Targeted isolated prototypes
First profile hash cost vs group classification, probe lengths, candidate equality, tombstones, rehash, allocation and iteration for short/long/colliding keys. Prototype independently: insertion probe simplification, first-group path, lane mask/branch pruning, group width and wrapped scans, capacity/tombstone policy. Compare SIMD/scalar cross-target results; inspect emitted code. Only if locality counters justify: alternative control/entry layout, quantifying metadata per entry, reserved capacity, iteration, drop and rehash cost. Analyze alternate secure hash only if hasher accounts for substantial workload time. Do **not** repeat identical rejected fingerprint scheme without new hypothesis.

## Safety/security / validation
Maintain first-EMPTY stopping invariant, FULL/EMPTY/TOMBSTONE distinctions, valid MaybeUninit access, Hash/Eq panic invariants, randomized collision resistance. Native HashMap randomized differential tests, tombstone/full/wrap, rehash, hash flooding, destructive panics, correct drops, Miri and SIMD/scalar equivalence. Compare A5/B8/B5 and all 16 final scenarios; metadata, retained bytes, RSS (≤+2%/≤+5% default). Reject microbench-only wins; report measured candidates, diff/SHA and exact acceptance or rejection.
