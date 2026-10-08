# PLAN_RESOLUTION.md — Cage Resolution and Collection Mutation

## Objective
Remove redundant cage-header resolution in A4 and B6 while preserving offset-only ownership and safe borrowing.

## Scope
Own `crates/compact_collections/src/deque.rs` (CompactVecDeque::{push_back,push_front,pop_front,pop_back,physical_index,reserve}) and `crates/compact_collections/src/vec.rs` (CompactVec::{as_mut_slice,retain,try_clone_copy}, IndexMut impl). Read-only dependency: cage.rs CageAllocation::{as_mut_slice,uninit_capacity_mut}, read_header. Benchmarks: scenarios.rs::{deque_churn,order_book} are centrally owned.

## Verified facts
A4 focused push/pop mutation 1,925.50 µs compact / 459.76 µs native. read_header 38.56% of compact self samples. B6 indexed quote updates 61.84 µs compact / 11.36 µs native; retain/snapshot near native; read_header 30.85% of compact self samples.

## Phase 1: B6 batch access experiment
First modify an isolated diagnostic benchmark, not production, to compare indexed access versus one `CompactVec::as_mut_slice()` borrow per bid/ask update batch or round. Match order, indices, checksums, workload. Never retain slice across retain, reallocation, cloning, growth or other invalidation. If existing API suffices, document usage rather than add an unnecessary API. If justified, expose safe borrow-scoped batch access that prevents invalidation while borrowed.

## Phase 2: deque writable view
Experiment with a safe closure- or operation-scoped writable ring view; its native pointers are temporary and exclusive to an active &mut borrow. View tracks head, len, capacity, wraparound and initialization. Growth either forbidden while borrowed or performed after view ends and normal reserve path is reacquired. Test batched push_back/pop_front, push_front/pop_back, full-capacity FIFO, wrapped buffers. No benchmark-only unsafe bypass or new retained deque fields.

## Phase 3: redundant reads
Inspect optimized code for repeated read_header per operation. Reuse already-validated borrow-scoped views without bypassing required validation of external/unsafe offsets. Preserve 4-byte vec, 12-byte deque, 16-byte header.

## Correctness
Native differential tests: empty/full and mixed directional deque operations, wrap, growth, iterator order, early return/panic, elements with panicking destructors, exactly-once drop, ZST if supported. Vector tests: batched update ordering, retain after updates, clone, panic/unwind, aliasing and allocation replacement. Run Miri and all collection tests.

## Benchmarks and acceptance
A4 mutation, B6 indexed update, B6 end-to-end medians/p95 and absolute µs, memory ratios, checksums. Keep only real API/use-pattern gains; discard experiments that improve only synthetic harnesses. Handoff: exact changed symbols/files, SHA, test/Miri results, metrics, generated code, rejected approaches, unresolved questions.
