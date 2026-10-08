# PLAN_DEQUE.md — Borrow-Scoped Batched Ring Operations

## Scope
Own crates/compact_collections/src/deque.rs and dedicated tests. Start CompactVecDeque::{push_back,push_front,pop_front,pop_back,reserve,physical_index}; read A4 focused benchmarks.

## Verified facts
Current operations individually resolve/validate storage. Frozen 12-byte deque cannot persist a native pointer. Profile CPU lies mainly within push_back and pop_front.

## Experiment 1
Separately measure construction/growth, front/back pushes/pops, wrapped versus contiguous FIFO and full starting state. Inspect emitted header checks and ring index instructions.

## Experiment 2: batch view
Prototype exclusive &mut-borrow/closure scoped ring mutation view resolving storage once for multiple no-growth operations. Native pointers may be retained only inside borrow lifetime, never in deque state. Track head/len/capacity and initialized slots, support wraparound and exact transfer. Pre-reserve before view; capacity errors must have documented atomicity and valid post-error state. Avoid benchmark-only bypass.

## Public API gate
Offer safe batch API only if realistic workloads demonstrate repeatable gains versus ordinary per-operation methods, without significant single-op regression.

## Tests
Differential VecDeque sequences, empty/full/wrap/growth/front/back, ZST if supported, panic/early return, destructor panics, exact drop count, attempted growth during borrow (compile-time or API guard), Miri. Keep 12-byte layout and no persistent native pointer.

## Handoff
List new types/functions, files, SHA, tests, median/p95 and absolute A4 phase improvement, rejected designs and compatibility notes.
