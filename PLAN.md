# V2.4 Profile-Driven Performance Optimization

Repository: Biddlebaddleboo/compact-std-library-rust
Branch: main
Pinned profiling baseline: b0b956bc7ae1402c24f3fed1e1a0763e3caab8f4
Production baseline: fc0a58bf08aa3bffc577ec2da8151d262b6a5f53

## Objective and benchmarks

Fix the actionable, measured bottlenecks from PROFILE_SUMMARY.md, PROFILE_CPU.md, PROFILE_COLLECTIONS.md and PROFILE_ALLOCATOR.md without modifying V2.4 retained layouts.

Current compact/native slowdown: A2 1.32–1.44x; A4 5.54–6.14x; A5 3.93–4.11x; B6 3.49–3.65x; B8 2.54–2.59x; B10 1.68–2.32x (noisy). B3 and B5 are regression sentinels. These ratios are measurements, not promised speedups.

## Verified findings

- `read_header` is prominent in A2/A4/B6 sampled self time.
- B6 indexed updates (61.84 vs 11.36 µs) dominate its remaining gap; retain and clone are close to native.
- A4 pushes/pops resolve backing storage individually.
- A5 still has 7.13x build, 8.84x hit lookup, 7.21x churn gaps with an identical diagnostic FNV builder; hashing alone is much closer; the SIMD classifier is only ~2–3% of sampled CPU.
- A5 probes are shallow; churn does not repeatedly rehash.
- B8 combines hash access, allocation, pending reuse and release. Its active pending scan averages 3.25 candidates, so there is no evidence for a new pending index.
- B10 prior uprobes distorted execution ~25–27x; reliable contention attribution is still missing.
- Miri run 37717178933 failed at the intrusive allocator invariants stage.

## Frozen contracts

CageAllocation<T>, Option<CageAllocation<T>>, CompactBox<T>, CompactVec<T>: 4 bytes. CompactVecDeque<T>: 12 bytes. AllocationHeader: 16 bytes. Frozen descriptors: 8 bytes. Keep one cage, 32-bit relative owners, correct borrowing/FFI, no permanent native pointers, no relaxed offset validation, exact allocation accounting.

No software prefetching, new inline assembly, V2.5/V3 redesign, or insecure hash defaults.

## Mandatory validation prerequisite

Inspect failed GitHub Miri run 37717178933 to identify exact failure and reproduce on pinned main. Decide whether test flake, concurrency interference, telemetry issue, environment problem or real correctness bug. Fix established cause and add deterministic regression coverage. Do not suppress assertions or skip tests. Establish passing Miri before changing unsafe allocator internals. If fix changes architecture, reconcile all relevant workstream guidance centrally.

## Workstreams, ownership and parallel safety

PLAN_RESOLUTION.md owns production files `crates/compact_collections/src/deque.rs` and `vec.rs`; focus A4/B6. Read cage internals only.

PLAN_HASH.md owns `crates/compact_collections/src/hash_map.rs` and, only as justified, `hash_control.rs`; focus A5 and hash-related B8.

PLAN_ALLOCATOR.md owns `crates/compact_backend_std/src/cage.rs`; focus A2/B8/B10 after Miri prerequisite.

Three isolated branches/worktrees may start independently for non-conflicting work; allocator unsafe edits wait for Miri triage. Agree cross-crate APIs centrally first; avoid shared production edits.

Orchestrator exclusively owns shared benchmark `crates/compact_std/examples/benchmark_compare/{main.rs,measure.rs,scenarios.rs}`, `BENCHMARKS.md`, `ARCHITECTURE.md`, `SAFETY.md`. Executors can use independent temporary diagnostic harnesses and provide handoffs. No concurrent shared-file edits.

## Integration

1. Verify latest main, reconcile architecture.
2. Diagnose/fix Miri prerequisite and reestablish baseline.
3. Record uninstrumented native and compact release medians, p95, phase timings, RSS and checksums.
4. Run bounded workstreams in isolated worktrees; each reports changed files, commit SHA, tests, measured results, deviations, open assumptions.
5. Integrate resolution, then hash, then allocator, rerunning affected tests/benchmarks each step.
6. Run targeted A2/A4/A5/B6/B8/B10 and B3/B5 regression checks; full 16-scenario suite twice.
7. Review live memory, peak RSS, allocator invariants, error/panic/concurrency correctness and generated assembly.
8. Independently check final diff, resolve plan contradictions centrally, delete all PLAN*.md, commit implementation without planning artifacts.

## Acceptance

Test every profiled hotspot but keep only repeatable improvements with acceptable complexity, safety and no material regressions. Revert unsuccessful experiments and document why. B10 allocator changes require non-intrusive evidence first. Never mistake inclusive samples or high-overhead telemetry for removable production CPU time.

## Validation

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target x86_64-apple-darwin -p compact_collections --tests
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-opt-run1.tsv
cargo run --release -p compact_std --example benchmark_compare --features json,toml -- --output /tmp/v24-opt-run2.tsv
```

Run full .github/workflows/miri.yml equivalent, including the previously failing intrusive allocator invariants. Compare matching logical checksums. Retained bytes, high-water and process RSS are distinct metrics.

## Final checklist

- [ ] Miri failure diagnosed and corrected, not hidden
- [ ] Frozen owner/deque/header/frozen-descriptor sizes unchanged
- [ ] No persistent pointer, prefetch, new assembly or weakened hashing security
- [ ] Borrow/aliasing, initialization, panic/drop, error, concurrency and allocation accounting proven
- [ ] B3/B5 gains preserved, B10 contention regression checked
- [ ] All scenario checksums, two full suites, Miri, tests, Clippy and portability pass
- [ ] Only measured, justified improvements retained
- [ ] Final diff independently reviewed; PLAN*.md removed before implementation commit

Execution handoff: Verify latest main. Read PLAN.md first. Reproduce and resolve Miri failure. Run workstreams in isolated worktrees with clear ownership; integrate in documented order, benchmark and validate, review diff, delete PLAN*.md and commit.
