# PLAN.md — V2.5 Framework-Wide Architectural Optimization
Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Verified initial baseline: 9ff38cdfc6daffc75eb39fcc8047d12cd9d180f8

## Objective
V2.5 is an evidence-driven architectural optimization release. Existing architectural contracts are optimization targets, **not** defaults to preserve. Optimize CPU efficiency, latency, throughput, concurrency and memory across the whole framework, including workloads already faster than native Rust. Accept contract changes whenever repeatable, worthwhile gains justify complexity and modest memory costs; safety remains mandatory.

## Verified facts
V2.4 has cage-relative 4-byte ownership and vectors, 12-byte deque, 16-byte allocation header, 8-byte frozen descriptor. `CompactVecDeque::with_view` and `CompactVec::as_mut_slice` amortize validation during borrows. Hash lookup has a specialized probe; Round 4 seven-bit fingerprint prototype delivered negligible integrated gain and was rejected. Global allocator mutex remains a B10 hotspot. `crates/compact_backend_std/src/allocator_model.rs` is test-only/sequential, not an implementation or proof of concurrent safety. Round 4 added profiling harness and collection differential tests; A2/A4/A5/B8/B10 remain slower in ordinary APIs, B3/B5 faster. Round 4 left the five PLAN*.md files on main pending architecture handoff.

## Architectural policy
Open to redesign: allocation synchronization/reservations, global/TLS ownership, cage offset resolution, headers and validation, borrow-scoped storage, collection metadata/backing stores, hash probing/control bytes, reclamation/recycling/fragmentation, internal interfaces and lifetimes. Do not redesign without a measured hypothesis or retain a costly contract solely for compatibility. Keep safe Rust guarantees, collision-resistant randomized default hashing, correct initialization/destruction, remote-free safety and accurate accounting. Do not mislabel distinct algorithms as like-for-like benchmarks.

## Memory budget (relative to pinned V2.4 on identical workloads)
Retained bytes default ≤+2%, peak RSS default ≤+5%, owner sizes preferably unchanged, metadata preferably unchanged; bounded lazy and reclaimable TLS reservations, idle return close to baseline, no significant fragmentation regression. Measure absolute and relative deltas and separate virtual cage reservation, committed pages, allocator retained bytes, metadata and RSS. Tiny/noisy baselines require absolute-byte context. Exceptions require explicit approval with benefit and exact cost; no unbounded growth.

## Scope ownership
- PLAN_PROFILING.md: standalone profiling tools, baseline capture and opportunity matrix; read-only production code.
- PLAN_ALLOCATOR.md: `crates/compact_backend_std/src/cage.rs`, allocator model/tests, synchronization, release, accounting, allocator-side access primitives.
- PLAN_OWNERSHIP.md: define shared handle-resolution/validation/borrow contract and proof obligations; owns design document, not production cage.rs, which allocator integrates.
- PLAN_COLLECTIONS.md: `crates/compact_collections/src/{vec.rs,deque.rs,hash_map.rs,hash_control.rs}` and associated tests. Other collection files only if profiling warrants.
- PLAN_MEMORY.md: memory/layout/compatibility inventory, budget tests and report; production layout edits by owning workstreams.
- PLAN_VALIDATION.md: integration verification and final report.
Orchestrator owns shared `benchmark_compare` scenarios, `benchmark_profile.rs`, global docs/version changes, ownership interfaces and merge conflicts. Avoid simultaneous edits to shared symbols.

## Dependencies, parallel safety and integration
1. Verify latest main, reconcile changed code/history/CI; read PLAN.md first.
2. Profile all 16 scenarios (including already fast) and memory on a reproducible, accounting-free V2.4 baseline; separate instrumented allocation statistics; rank hotspots by absolute time, cross-framework impact, improvement confidence, complexity and memory cost.
3. In parallel isolated worktrees, allocator develops **state contract/model**, ownership defines safe resolved-view API, collections profile localized bottlenecks, memory establishes baseline. No unsafe TLS production implementation before central safety gate.
4. Orchestrator approves allocator/ownership interfaces, memory exceptions and architecture decisions. Contract consumers implement only after interface approval and upstream integration. Isolate competing prototypes and measure each against baseline before combining.
5. Integrate allocator, agreed ownership primitives, collection consumers, then benchmarks/docs. Reprofile shifted bottlenecks, regressions and memory.
6. Run cargo fmt --all -- --check; cargo check --workspace --all-features --locked; cargo test --workspace --all-features --locked; cargo clippy --workspace --all-targets --all-features --locked -- -D warnings; full Miri/harness workflows; available cross-target checks; stress/differential tests and repeated 16-scenario release comparisons.
7. Independent final diff and test review. Record accepted/rejected experiments, absolute latency, medians/p95, RSS, retention, fragmentation, complexity and migrations. Remove **all** PLAN*.md before final implementation commit.

## Non-goals
Wholesale rewrite absent evidence; unchecked public APIs; hash-flood regression; benchmark-only shortcuts; unbounded TLS caches; assuming lower instruction count equals runtime improvement; V3 compiler ABI projects.

## Execution handoff
Bounded executors report exact touched files/symbols, commit SHA, tests, performance/memory comparisons, deviations and unresolved assumptions. Scope expansion only for moved symbols, compilation, safety, compatibility or demonstrated dependencies. Central orchestrator resolves contradictions; no executor redesigns interfaces silently.
