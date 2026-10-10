# PLAN_COLLECTIONS.md — Ordinary Deque and Vector Optimization

## Scope and verified facts
Own `crates/compact_collections/src/deque.rs`, `vec.rs`, dedicated tests. Inspect `CompactVecDeque::{push_back,pop_front,push_front,pop_back,with_view}`, `CompactVec::{push,retain,as_mut_slice}`, Index/IndexMut, growth/reserve and relevant accessors. Backend `cage.rs` is read-only; allocator owner implements any centrally approved backend primitive. Owner-header fast path already landed; ordinary A4 still slower, batch view fast, B6 borrowed-slice operations near native. Branchless ring-index prototype regressed and generic resolved-view API lacked incremental benefit.

## Phase 1 — reprofile current code
Separate A4 steady-state, full-ring growth, wrapped/unwrapped, bounds/ring arithmetic, slot reads/writes, initialized length and drop; B6 index, retain, clone/snapshot, growth, codegen. Do not reuse pre-header-optimization hotspot assumptions.

## Phase 2 — bounded code prototypes
For ordinary deque operations, investigate resolving once per operation, duplicated bounds/length checks, slot sequencing, common capacity-state paths and growth without repeating rejected branchless index. For Vec, isolate Index/IndexMut, push/extend, bulk per-borrow updates, retain and cloning/copying; favor one validated borrow per bulk op and initialized-prefix/panic-safe guards. Only propose a shared resolved-access backend API if more than one collection actually benefits; it must be allocator-owned and reviewed before consumer changes. No unchecked public accessor or permanent native pointer.

## Evaluation / handoff
Compare ordinary compact/native on same algorithm; optional batch versus comparably batched native separately. A4/B6 primary; A2/B3/B5 sentinels and full suite after integration. Native Vec/VecDeque randomized differential tests, wrap/growth/exhaustion, ZST, panic/destructor exact-once, borrow/alias/Miri. Preserve 4-byte Vec and 12-byte Deque preferred; default ≤+2% retained/≤+5% peak RSS. Reject changes that move overhead or worsen ordinary workloads. Report functions, source SHA, tests, performance, memory, APIs and accepted/rejected variants.
