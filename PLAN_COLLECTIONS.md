# PLAN_COLLECTIONS.md — Deque, hash map and vector remaining bottlenecks

## Scope
Own crates/compact_collections/src/deque.rs, vec.rs, hash_map.rs and hash_control.rs only if profiling justifies classifier edits, plus collection-specific tests. Read-only benchmark_compare/scenarios.rs and crates/compact_std/examples/collection_profile.rs (shared harness work centrally owned). Consult latest PROFILE_V2_4_ROUND2.md measurements before retaining changes.

Starting symbols:
- CompactVecDeque::{push_back,push_front,pop_front,pop_back,reserve,physical_index}
- CompactVec::{push,as_mut_slice,retain,try_clone_copy}, Index/IndexMut
- CompactHashMap::{insert,get,get_mut,remove,remove_entry,find_slot,find_slot_in,rehash}
- classify_control_group, hash_control scalar/NEON/SSE2 paths

## A4 — deque
Profile separate construction, growth and steady push/pop costs; last measured end-to-end 4.4x native. Test temporary &mut-borrow-scoped batch ring view for multiple operations after proving real caller utility. Do not store native pointer inside 12-byte deque. Define capacity/initialization/head/len and wrap invariants, prohibit growth while view borrows backing storage or end view before reserve; ensure panic/early exit/drop-once/empty/full handling. Contrast single operations and batched usage at matched work volumes; do not ship a benchmark-only bypass.

## A5 — hash
After single-resolution insert/get_mut/remove_entry, freshly profile remaining get/hit/miss/insert/remove access patterns, duplicate probing, equality, control and entries loads, first-group fast path, tombstones, load factor and instruction count. Preserve randomized hashing, tombstone-first semantics, no destructive moves ahead of fallible hash/eq and safe lookup borrow lifetimes. Only touch hash_control SIMD when measured expensive. Never silently switch to insecure FNV; use it for diagnostics only.

## B6 — order book
Measure already available `CompactVec::as_mut_slice()` once per update round versus IndexMut per element while respecting no borrow across retain, growth or allocation replacement. Measure B6 construction separately from update/retain/snapshot. Prefer documenting existing efficient API over inventing unnecessary public APIs. Match updates, checksums and order exactly.

## B8 — cache
Phase-isolate hash lookup, remove, object construction, insertion, value destruction, release batching. Share remaining allocator residual after integrating hash changes. Preserve collection semantics and deterministic checksums.

## Tests and handoff
Differential tests against std collections, deque wrap/full/growth/directional operations, vector drop/panic/update-slice lifetime, map hash/eq panic and collisions, tombstone/reinsert, SIMD scalar equivalence and Miri unsafe access coverage. Keep frozen compact layouts, no retained native pointers. Report exact symbols/changed files, commit SHA, before/after medians/p95 absolute units, peak RSS, retained memory, failed experiments, tests and deviations; avoid B3/B5 and B8 regressions.
