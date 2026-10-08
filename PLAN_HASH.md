# PLAN_HASH.md — A5 hash probing and mutation

## Verified baseline
Main 4a57dc7 A5 hash map/set churn ~5.0 ms, 4.25–4.32x native. Separate cage control/entry allocations; 16-byte control groups use contiguous direct SIMD classification and bounded scratch for wrapped/partial groups. Randomized SipHash-2-4 remains default. CompactHashMap::rehash uses a temporary native Vec<usize> destinations so hashing and allocation finish before element moves.

## Implementation scope
Own crates/compact_collections/src/hash_map.rs (CompactHashMap::{hash,find_slot,find_slot_in,ensure_insert_capacity,rehash}, classify_control_group, first_empty_slot) and crates/compact_collections/src/hash_control.rs (scalar classifier, NEON, SSE2), with related tests. Read benchmark scenario hash_churn and compact set wrappers.

## Profiling and design
Measure hash calculation separately from control-group probe and equality; distinguish scalar keys and short/long strings, adversarial collisions and custom hashers. Record groups visited, full candidate comparisons, tombstones, probe distances, rehash triggers and allocations without polluting timed production runs.

Investigate first-group fast path and avoid repeated control/entry resolutions while preserving first tombstone/empty behavior and all bounds. Do not read outside physical table, including wrap/partial SIMD groups. Retain scalar oracle, Miri compatibility and existing NEON/SSE2 fallback logic.

Investigate downsizing/removing rehash native Vec<usize> scratch, e.g. compact temporary index representation or safe two-pass strategy. Crucial invariant: all fallible hashing/allocations must occur before destructive move; avoid user Hash/Eq code after transfers begin if it can panic. Keep scratch if alternatives endanger unwind safety or performance.

Investigate whether A5's remove/reinsert pattern triggers excess tombstone cleanup/rehash; change thresholds only with diverse workloads and correctness proof. Consider alternate hash policy only after documenting randomized hash-flood resistance; never silently weaken secure default for scalar-key benchmark gains.

## Tests and security
Scalar versus SIMD across every wrap position and control state; collisions, tombstone-first preference, near full, rehash and allocation failures; deterministic operation-state differential model vs std::collections::HashMap, custom pathological Hasher/BuildHasher and Hash/Eq panic injection; no lost/double-dropped keys or values. Miri and concurrent collection tests where applicable.

## Acceptance/handoff
Compare A5 build, mutation, lookup, drop medians/p95 and retained memory versus native and 4a57dc7. Keep only benefits across realistic workloads with unchanged control format, retained sizes, randomized collision defense and panic safety. Report SHA, files, tests, perf breakdown, exceptions and unresolved risks. No prefetch, inline asm or new object fields.
