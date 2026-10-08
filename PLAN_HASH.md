# PLAN_HASH.md — Hash Table Access and Probing

## Objective
Reduce repeated table allocation resolution, insertion and lookup overhead in A5 and B8 without changing hash security or retained object size.

## Scope
Own `crates/compact_collections/src/hash_map.rs`: CompactHashMap::{insert,hash,find_slot,find_slot_in,ensure_insert_capacity,rehash}, classify_control_group and first_empty_slot. Read `hash_control.rs` scalar/NEON/SSE2; edit only if justified. Centrally-owned benchmark references: scenarios.rs::{hash_churn,cache_churn}.

## Verified facts
A5 about 4x native; identical diagnostic FNV builder still yields 7.13x build, 8.84x hit lookup, 7.21x churn. Hash computation much closer, SIMD classifier ~2–3% compact CPU; shallow probes and no churn rehash. Default randomized hashing must remain secure; FNV is diagnostic only.

## Phase 1: one-resolution table access
Within a single operation, resolve and validate control and entry cage slices once, along with mask/capacity; pass short-lived borrowed views to internal probe helpers. Carefully structure borrowing for insert/remove mutations and equality callbacks. Do not retain native pointers in map state. Preserve offset/header validation when entering an operation.

## Phase 2: insert/lookup
Measure hashing, control loads, equality checks, first group, slot search, replacement, tombstone and metadata. Evaluate first-group fast path or reduced repeated lookup only if benchmark supports it. Preserve EMPTY/FULL/TOMBSTONE, tombstone-first preference, equal-key replacement, iteration semantics and safe drop behavior.

## Phase 3: B8 attribution
Use isolated nonintrusive capture to separate lookup/removal, value drop, construction, insertion and batched release without altering logical operation order. Quantify improvements attributable to map optimizations, then hand residual allocator cost to PLAN_ALLOCATOR.md.

## Phase 4: optional rehash
Investigate native Vec<usize> destination staging only if rehash costs materially contribute to a relevant workload. Preserve all fallible allocations/hash calls before destructive transfer, especially panic safety. Avoid table-layout redesign under frozen V2.4 unless explicitly proven compatible.

## Tests/security
Native HashMap differential randomized operation sequences, collision-heavy keys, tombstones, remove/reinsert, growth, exhausted allocation, Hash/Eq panics, exact drops, SIMD/scalar on partial and wrapped groups. Default randomized collision resistance stays unchanged. Run Miri and relevant tests.

## Acceptance
A5 build/mutation/lookup and B8 phase medians/p95, absolute costs and RSS/retained ratios. Reject gains that regress B5 or B8, weaken security, or grow retained memory. Handoff files/SHA, measurements, Miri/tests, safety findings and remaining bottlenecks.
