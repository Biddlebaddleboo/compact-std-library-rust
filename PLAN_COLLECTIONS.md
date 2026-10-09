# PLAN_COLLECTIONS.md — Contract-Aware Collection Optimization

## Scope
Own `crates/compact_collections/src/{vec.rs,deque.rs,hash_map.rs,hash_control.rs}` and dedicated tests. Start `CompactVec::{as_mut_slice,push,retain,try_clone_copy}`, `CompactVecDeque::{with_view,push_back,pop_front,push_front,pop_back,reserve}`, `CompactHashMap::{find_index_in,find_slot_in,insert,get,get_mut,remove_entry,rehash}`, `classify_control_group`, `first_empty_slot`. Extend to other collection files **only** for profiler-ranked substantial hotspots; report additions. Allocator and common harness are read-only dependencies.

## Current evidence
With-view A4 batched path ~0.143 ms versus ordinary compact ~1.190 ms in Round 4 focused study; B6 compact slice updates ~9.44 µs versus compact per-index ~39 µs. Standard A4 still incurs repeated header resolution; Round 4 7-bit fingerprint prototype slightly aided B8 lookup but barely improved whole scenario and was rejected.

## Investigation
Vec: growth, indexing, retain, bulk mutation/insert, clone/copy and destruction; assess shared resolved scopes, inline storage access and allocation policy.
Deque: wrap arithmetic, capacity/growth, bulk operations, iteration and view ergonomics; retain ordinary-versus-ordinary and batch-versus-comparably-batched comparisons.
Hash: probe group/layout/control bytes, memory locality, per-key hash cost, tombstones, load-factor/rehash, iteration and destructors. Test SIMD scalar/NEON/SSE2 equivalence; maintain randomized collision resistance and panic-safe Hash/Eq.
Other collections: based on *all 16* profiling including already-faster A3/B3/B5/B7/B9; identify reusable architecture where multiple consumers pay same overhead. Don't change all collections simply for symmetry.

## Constraints and tests
Memory budget per collection and library: retained ≤+2%, peak RSS ≤+5% baseline absent exception, unchanged compact owner footprint preferred; compare metadata/entry, fragmentation and code size. No unsafe aliasing or lost/double drops. Differential std collection randomized tests; ZST, wraps, exhausted growth, panic, rehash collisions, remote ownership, Miri and platform checks. Report accepted/rejected variants, exact symbols/diff/commit, metrics and API compatibility. Depend on centrally approved allocator/resolution contract before migrations.
