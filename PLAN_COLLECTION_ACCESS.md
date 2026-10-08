# PLAN_COLLECTION_ACCESS.md — Borrow-scoped collection batching

## Scope
Own crates/compact_collections/src/deque.rs and vec.rs plus dedicated collection tests. Inspect CompactVecDeque::{with_view,push_back,pop_front,push_front,pop_back,reserve}, CompactVec::{as_mut_slice,push,retain,try_clone_copy}. Read A4/B4/B6 benchmark scenarios; shared harness files are orchestrator-owned.

## Verified facts
with_view gives borrow-scoped ring access without retained native pointer or persistent 12-byte layout change; it disallows growth and requires reserve. Batched A4 diagnostic was ~0.157 ms versus ordinary compact 1.292 ms, but ordinary production benchmark was deliberately unchanged for fair per-operation comparison. B6 can use existing as_mut_slice per update round; B4 construction dominates.

## Phase 1: fair A4 tests
Compare (a) ordinary compact/native, (b) both sides using comparably batched access, and (c) compact batch/ordinary compact as a separate API study. Vary batch size, wrap frequency, spare capacity, element size, push/pop pattern and growth pre-reserve; distinguish true per-operation improvements from different caller algorithms. Preserve shared benchmark fairness.

## Phase 2: B6 vector
Test a borrow of CompactVec::as_mut_slice for each update round versus per-index IndexMut, matching updates and snapshots; don't hold slices across retain, growth, clone replacement. Measure separate build/update/retain/snapshot phases. Prefer documenting existing safe API to adding unnecessary wrappers.

## Phase 3: further caller-pattern opportunities
Inspect other loops with repeated storage resolution, but create new borrowing APIs only if a realistic workload and a material sampled hotspot justify them. Document usage, errors, unwind and reserve requirements. No generic batching framework absent evidence.

## Correctness and acceptance
Differential VecDeque/Vec behavior under wrap, empty/full, growth, ZST, errors, panic/unwind, drop-once, aliasing and scope ends; Miri on new unsafe paths. Preserve 4-byte vec/12-byte deque and no native pointer in retained owner; no material regression in ordinary APIs. Report SHA, touched symbols, medians/p95 per distinct comparison, memory, tests and rejected designs.
