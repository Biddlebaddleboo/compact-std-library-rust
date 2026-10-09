# PLAN_COLLECTION_ACCESS.md — Ordinary Deque/Vector Access

## Objective / scope
Speed up *ordinary* API operations, not merely optional batching. Own `crates/compact_collections/src/deque.rs` and `vec.rs`, dedicated tests. Start `CompactVecDeque::{push_back,pop_front,push_front,pop_back,with_view,reserve}`; `CompactVec::{as_mut_slice,push,retain}` and Index/IndexMut. Read backend `CageAllocation<T>::as_mut_slice`, `read_header`, `validate_typed_header` and `OWNERSHIP_DECISION.md` without editing cage.rs (allocator-owned).

## Evidence
Ordinary A4 ~3.3x native; `with_view` batch far faster but opt-in. B6 per-index updates costly while single borrowed slice performs near native. Header checks currently inline; generic cross-crate resolved-view API was deferred for no proven incremental gain.

## Implementation experiments
1. Separate A4 steady-state push/pop, growth, construction, wrapped/unwrapped and ZST; inspect header reads, validation, bounds, ring index, slices, initialization/drop in generated code. Prototype single-operation simplifications without retained pointer or unsafe validation removal.
2. If more than one collection repeats *measured* resolution work, propose a narrow lifetime-scoped backend primitive; get central allocator-owner approval and integrate after interface lands. Borrow exclusivity, typed extent validation, no stale pointer, growth inhibition and unwind must be proven. Avoid generic public types solely for organization.
3. Vector: independently prototype fewer resolutions in indexed access/retain/push/extend/clone, use one validated borrow per bulk op and maintain initialized-prefix guards and panic/drop ordering. Ordinary semantics and std-style APIs remain available.
4. Retain `CompactVecDeque::with_view` and `CompactVec::as_mut_slice`; compare ordinary native/compact and equivalent batch native/compact *separately*. No benchmark-only rewrite for unfair speedup.

## Tests and acceptance
Differential Vec/VecDeque sequences, wrap/growth/replacement/exhaustion, ZST, partial initialization, panicking constructors/destructors, exact once drop, borrow-lifetime/aliasing and Miri. Prefer unchanged four-byte vec/twelve-byte deque, no heap metadata; count code size and stack-view costs if applicable. Measure A4/B6, B3/B5 sentinels, full scenario impacts, retained/RSS. Accept repeatable ordinary-API gain without serious regression and ≤+2% retained/≤+5% RSS; reject redundant non-beneficial abstraction. Handoff touched symbols, SHA, tests, ratios, absolute times, memory, API implications.
