# PLAN.md — V2.4 native-Rust benchmark and freeze

## Baseline

Repository: `Biddlebaddleboo/compact-std-library-rust`  
Branch: `main`  
Verified baseline commit: `e14034eaa41a31239722fdf5c268a4f3f7cd5a2e`  
Version: `2.4.0`

V2.4 architecture is considered complete at this baseline.

This task is not another V2.4 architecture iteration. Its purpose is to:

1. replace the current compact-only, one-shot benchmark examples with a controlled **native Rust vs V2.4** benchmark suite;
2. add realistic application-shaped scenarios in addition to container microbenchmarks;
3. measure both CPU and memory behavior using identical logical workloads;
4. document the results honestly;
5. make the resulting benchmark/validation state the **frozen V2.4 baseline**.

Do not benchmark against V2.2 or V2.3 as a primary comparison. Historical versions may be mentioned only as history, not as the performance reference.

## Primary comparison

Every benchmark must compare:

```text
baseline:
    ordinary Rust std types and normal Serde construction

versus:

V2.4:
    compact_std cage-backed equivalents
```

Examples:

```text
std::Vec<T>              vs CompactVec<T>
std::Box<T>              vs CompactBox<T>
std::String              vs CompactString
std::VecDeque<T>         vs CompactVecDeque<T>
std::HashMap/HashSet     vs compact equivalents
std::PathBuf             vs CompactPathBuf
ordinary nested structs  vs compact nested structs
ordinary immutable data  vs FrozenGraph
serde_json -> std model  vs compact_std::json -> compact model
toml -> std model        vs compact_std::toml -> compact model
```

Both sides must perform the same logical work on the same deterministic dataset.

Do not use V2.2 benchmark results as a success criterion.

---

# Verified repository facts

At `e14034e...`:

- `BENCHMARKS.md` contains only one release-mode V2.4 run and explicitly states it is not a comparative allocator benchmark.
- `crates/compact_std/examples/runtime_bench.rs` measures only compact owner sizes, compact vector growth, and short `CompactBytes`.
- `crates/compact_std/examples/workloads_bench.rs` measures only V2.4 implementations.
- Current examples use `Instant` directly and run each workload once.
- `compact_std` already has optional `json` and `toml` features.
- `compact_serde` already depends on `serde`, `serde_json`, and `toml`; use the same parser versions for native comparison rather than introducing competing parser implementations.
- V2.4 currently has a 16-byte common allocation header, four-byte `CompactVec`/`CompactBox`, intrusive cage free-list metadata, and eight-byte frozen descriptors.
- V2.4 Miri is green at the baseline commit.
- `PLAN.md` is absent at the V2.4 implementation baseline.

---

# Implementation scope

Inspect first:

- `crates/compact_std/Cargo.toml`
- `crates/compact_std/examples/runtime_bench.rs`
- `crates/compact_std/examples/workloads_bench.rs`
- `BENCHMARKS.md`
- `README.md`
- `ARCHITECTURE.md`
- `crates/compact_backend_std/src/cage.rs` only if benchmark diagnostics cannot be obtained from existing runtime APIs.

Proposed benchmark harness:

```text
crates/compact_std/examples/benchmark_compare/
    main.rs
    measure.rs
    models.rs
    datasets.rs
    scenarios.rs
```

Add an explicit `[[example]]` entry in `crates/compact_std/Cargo.toml`:

```text
name = "benchmark_compare"
path = "examples/benchmark_compare/main.rs"
required-features = ["json", "toml"]
```

The existing `runtime_bench.rs` and `workloads_bench.rs` may either:

- be deleted after their useful cases are incorporated into the paired suite; or
- become thin compatibility/developer examples that clearly defer authoritative performance results to `benchmark_compare`.

Do not maintain two competing authoritative benchmark methodologies.

---

# Benchmark harness design

## No heavy benchmark framework requirement

Do not add Criterion or another large dependency unless the std-only harness proves inadequate.

Prefer a small deterministic harness using:

- `std::time::Instant`;
- `std::hint::black_box`;
- repeated iterations;
- a warm-up phase;
- median and percentile calculation;
- a custom counting global allocator for benchmark-process memory accounting.

Keep dependencies minimal.

`serde` derive may be added as a dev dependency where needed for native benchmark models, using the same Serde ecosystem already present in the workspace.

## Process isolation

The top-level benchmark command should run each `(scenario, variant)` in a fresh child process by invoking the benchmark executable itself with internal arguments.

Conceptual interface:

```text
benchmark_compare
    orchestrator mode

benchmark_compare --child <scenario> native
benchmark_compare --child <scenario> compact
```

Reasons:

- native and compact allocator counters start from clean state;
- cage initialization does not contaminate native measurements;
- retained allocations from one scenario cannot affect another;
- process-level memory metrics are more meaningful;
- failure of one scenario can be reported independently.

The orchestrator must produce one consolidated report.

Do not rely on randomized execution order.

## Repetitions

For each timed workload:

- perform at least one untimed warm-up;
- run enough measured repetitions for stable medians;
- default target: 15–30 measured repetitions for short scenarios;
- allow fewer repetitions for deliberately large/long-running scenarios;
- never report only the single fastest run.

Report at minimum:

```text
median
p95
min
max
number of measured runs
```

Do not average away major outliers without reporting them.

The harness may automatically scale iteration count to avoid extremely short sub-microsecond samples, but the logical dataset must remain identical between native and V2.4.

---

# Memory measurement

CPU time alone is insufficient.

## Native baseline allocator accounting

In `measure.rs`, implement a benchmark-only global allocator wrapper around `std::alloc::System`.

Track with atomics:

```text
allocation calls
deallocation calls
requested bytes allocated
current requested live bytes
peak requested live bytes
```

Use the `Layout` size passed to `alloc`, `alloc_zeroed`, `realloc`, and `dealloc`.

The measurement is requested allocator bytes, not malloc implementation metadata. Label it accurately.

Reset counters immediately before each measured scenario body.

The measurement code itself must avoid corrupting the counters through recursive allocation.

## Compact accounting

For V2.4 report separately:

```text
cage retained live bytes
cage peak/high-water bytes where observable
native auxiliary allocation calls/bytes
native auxiliary peak bytes
```

Initialize the cage before resetting the global allocator counters so the configured cage reservation is not incorrectly reported as per-workload native auxiliary allocation.

Native temporary allocations used by:

- Serde parsers;
- frozen builders;
- formatting;
- explicit native conversions;

must still be counted as auxiliary native work.

## Allocator diagnostics

Prefer existing APIs.

If the realistic churn/fragmentation benchmark cannot be measured adequately with current runtime APIs, the only allowed library change is a read-only diagnostic surface in `compact_backend_std`, for example a proposed:

```rust
#[doc(hidden)]
pub struct AllocatorStats {
    pub live_bytes: u32,
    pub high_water_cursor: u32,
    pub free_bytes: u32,
    pub free_blocks: u32,
    pub largest_free_block: u32,
}
```

and:

```rust
#[doc(hidden)]
pub fn allocator_stats() -> Result<AllocatorStats>;
```

Exact naming may differ.

This diagnostic must:

- take the existing allocator lock;
- inspect state without mutation;
- add no fields to retained owners or allocation headers;
- add no allocator bookkeeping solely for benchmarking;
- not change allocation policy;
- not change layout;
- not change V2.4 safety invariants.

If existing state is enough, do not add this API.

## Process memory

On Linux, optionally record `/proc/self/status` metrics such as `VmHWM`/peak RSS for each child process.

Treat this as an OS-specific supplemental metric, not the canonical portable allocator measurement.

Do not add platform-specific dependencies solely for RSS.

---

# Correctness before timing

Every native/compact pair must compute and compare a deterministic checksum or logical result.

A benchmark result is invalid if the native and compact variants do not produce equivalent output.

Examples:

```text
same parsed record count
same IDs
same aggregate sum
same lookup results
same queue ordering
same final cache state
same number of emitted/logical events
same filesystem lookup result
same order-book best bid/ask and aggregate quantity
same frozen catalog traversal checksum
```

Do not benchmark deliberately different algorithms merely because one is easier in compact types.

---

# Scenario set

The final benchmark suite must contain both microbenchmarks and realistic workloads.

## Group A — core container microbenchmarks

These are diagnostic, not the headline results.

### A1. Vector build/traverse/churn

Native:

```text
std::Vec<u32>
```

Compact:

```text
CompactVec<u32>
```

Measure:

- build 100k elements;
- traversal checksum;
- repeated reserve/grow;
- truncate/regrow;
- drop.

Report wrapper size and retained bytes.

### A2. Box/object allocation

Allocate and destroy many small fixed-size objects using:

```text
Box<Record>
CompactBox<Record>
```

Use a cage-safe record containing only scalars.

Measure per-object retained overhead and allocation/drop throughput.

### A3. String/bytes distribution

Use a realistic deterministic length distribution:

```text
0–12 B
13–32 B
33–128 B
1 KiB
64 KiB
```

Compare `String`/`Vec<u8>` to `CompactString`/`CompactBytes`.

Do not benchmark only one favorable SSO length.

### A4. VecDeque

FIFO workload with wrapping, pop/push churn and growth.

### A5. HashMap/HashSet

Use:

- inserts;
- successful lookups;
- misses;
- updates;
- removals;
- churn.

Use the normal randomized hashers on both sides.

### A6. Paths

Build and query thousands of realistic path strings with mixed component lengths.

---

# Group B — realistic application scenarios

These are the primary benchmark results.

## B1. JSON API response

Generate a deterministic nested JSON payload representing an API result with roughly 10k records.

Each record should include a representative mixture:

```text
numeric ID
short name/string
status enum/string
boolean flags
optional fields
small vector of tags
nested metadata
timestamp-like integer/string
```

Native model:

- ordinary `String`, `Vec`, `Option`, and std structs;
- deserialize with the same `serde_json` version used by the workspace.

Compact model:

- `CompactString`, `CompactVec`, compact nested structs where supported;
- direct `compact_std::json` construction.

Measure separately:

```text
parse/build
retained memory after parse
traversal/query checksum
drop/cleanup
```

Do not include payload generation in timed parse results.

## B2. TOML service configuration

Create a substantially larger real configuration than the current three-field example.

Include:

```text
service name
endpoints
filesystem paths
labels
feature flags
retry/timeout numeric settings
nested worker definitions
routing entries
```

Compare native TOML-to-std model with compact direct construction.

Measure parse, retained memory, repeated reads, and drop.

## B3. HTTP/request metadata batch

Model tens of thousands of request metadata records:

```text
method
path
host
status
content length
several short headers
request/trace IDs
small tag lists
```

This intentionally stresses many small strings and nested collections.

Operations:

- construct batch;
- scan by status;
- lookup selected headers/fields;
- aggregate byte counts;
- drop.

## B4. Logging/event buffer

Model a service retaining a bounded event history.

Record:

```text
timestamp
severity
component
short message
small numeric context
optional request ID
```

Use native `VecDeque`/strings versus compact queue/string representation.

Simulate long-running append/evict churn.

Measure:

- steady-state per-event update time;
- retained bytes at fixed logical capacity;
- allocation count;
- fragmentation/high-water behavior on compact side.

## B5. Mobility/dispatch state

Create a realistic in-memory dispatch workload with records representing:

```text
offer/trip ID
pickup/dropoff zone IDs
coordinates as scalars
estimated distance/time
provider/status
short address/label strings
optional rider/driver metadata
small route/tag arrays
```

Operations:

- load active offers;
- update statuses;
- replace expired offers;
- lookup by ID;
- scan by zone/status;
- maintain recent history.

Use deterministic synthetic data; no private or external production data.

This scenario should exercise mixed short-lived and retained objects rather than one container type.

## B6. Market-data/order-book state

Model one or more trading-pair snapshots.

Record price levels:

```text
price
quantity
order count/flags
venue/symbol metadata
```

Operations:

- build initial bid/ask levels;
- apply deterministic quote updates;
- remove empty levels;
- calculate best bid/ask;
- calculate aggregate depth;
- rebuild a snapshot periodically.

Use `BTreeMap` only if both implementations require ordered-map semantics; otherwise use equivalent vectors/maps on both sides.

Do not compare different algorithms.

## B7. Filesystem/index catalog

Generate roughly 100k deterministic file records containing:

```text
PathBuf/path
size
mtime-like scalar
file type
extension/category
optional hash/metadata
```

Operations:

- build catalog;
- query selected paths;
- scan by extension/category;
- aggregate sizes;
- update a subset;
- drop.

This is a headline memory-density workload.

## B8. Cache/session churn

Maintain a fixed logical cache/session population while repeatedly:

- inserting new entries;
- expiring old entries;
- changing payload sizes;
- removing entries;
- reusing freed storage.

Run enough cycles to reach a steady state.

Report compact allocator:

```text
live bytes
high-water cursor
free bytes
free-block count
largest free block
fragmentation ratio where derivable
```

This scenario is specifically intended to expose pathological first-fit fragmentation.

Do not change the allocator merely because some fragmentation exists. Architecture changes require a clearly severe/pathological result.

## B9. Immutable/frozen catalog

Build an application-shaped immutable catalog containing at least:

```text
tens of thousands of records
names/strings
numeric IDs
small lists of related IDs
categories/flags
nested frozen descriptors
```

Native baseline should be a normal owned immutable Rust model such as vectors, strings, boxed slices, or equivalent conventional structures.

V2.4 side uses `FrozenBuilder`/`FrozenGraph`.

Measure:

```text
construction/freeze time
temporary native allocation peak
final retained memory
sequential traversal
random deterministic lookup/traversal
parallel read traversal
```

This is the primary benchmark for `FrozenGraph`.

## B10. Concurrent worker state

Use multiple threads operating on independent logical datasets while sharing the process allocator/cage.

Compare equivalent native and compact workloads.

Exercise:

- per-thread build;
- read/traverse;
- allocate/drop churn.

Do not deliberately share mutable compact owners in ways the public API does not support.

Report throughput and memory, and confirm allocator correctness after the compact run.

---

# Dataset generation

Implement deterministic generators in `datasets.rs`.

Requirements:

- fixed seeds/constants;
- no network;
- no wall-clock-dependent input;
- no external service;
- no random crate required unless already justified;
- generated datasets identical for native and compact variants.

A tiny deterministic PRNG implemented locally is acceptable if useful, but simpler arithmetic/index-based generation is preferred.

Do not time dataset generation unless a scenario is explicitly testing generation.

For parse benchmarks, generate/prepare serialized input before starting the timer.

---

# Timing phases

Do not collapse entire scenarios into one number when phases matter.

Where relevant report:

```text
build / parse
steady-state mutation
lookup / traversal
freeze / conversion
drop / cleanup
```

For real-world scenarios, also report an end-to-end median.

This makes it possible to see whether V2.4 trades slower construction for faster/similar traversal, or vice versa.

---

# Output format

The orchestrator must print and optionally write a machine-readable result.

Required human-readable table columns:

```text
scenario
phase
native median
V2.4 median
V2.4/native time ratio
native retained/requested bytes
V2.4 cage retained bytes
V2.4 native auxiliary bytes
memory ratio
```

Include p95 where space permits or in detailed sections.

Add a machine-readable format without adding a serialization dependency if practical, for example deterministic CSV/TSV.

If JSON output is simpler because `serde_json` is already enabled for the example, it is acceptable.

The committed `BENCHMARKS.md` must summarize results rather than paste enormous raw logs.

---

# Fairness rules

The benchmark suite must obey these rules.

1. Same logical dataset.
2. Same logical result.
3. Same parser version for native and compact Serde tests.
4. Release mode for reported results.
5. Same machine and toolchain within each published comparison.
6. No V2.2/V2.3 comparison presented as the primary result.
7. No native baseline deliberately written poorly.
8. No compact side given a different algorithm solely to win.
9. No timing of input generation unless both sides include it by design.
10. No one-run conclusions.
11. No hiding scenarios where V2.4 loses.
12. Memory metrics must clearly distinguish requested allocator bytes, cage live bytes, temporary native bytes, and OS RSS when available.

---

# Interpretation

The benchmark report must not require V2.4 to beat native Rust in every metric.

The purpose is to quantify the trade.

For each scenario classify the result descriptively, for example:

```text
major memory win / small CPU cost
major memory win / neutral CPU
memory and CPU win
minor memory win / substantial CPU cost
pathological regression requiring investigation
```

Do not establish arbitrary pass/fail percentages before measuring.

However, investigate before freezing if any realistic scenario shows:

- apparent correctness mismatch;
- allocator corruption;
- unbounded memory growth;
- fragmentation that grows without stabilizing under fixed logical population;
- orders-of-magnitude CPU regression;
- unexpectedly huge native auxiliary allocation that defeats the memory purpose.

Normal moderate tradeoffs are documentation results, not reasons to redesign V2.4.

---

# V2.4 freeze rule

After the benchmark suite is complete and validation remains green, V2.4 is frozen.

The freeze means:

- no further V2.4 representation redesign;
- no new allocation model;
- no owner/header format changes;
- no new pointer model;
- no opportunistic unsafe rewrite;
- no assembly optimization added merely because a microbenchmark could improve;
- no compatibility-breaking API changes.

Allowed after freeze:

```text
correctness/security fixes
documentation corrections
test improvements
benchmark harness fixes
clearly non-breaking maintenance
```

Any substantial memory-model, pointer-model, std/toolchain, or dependency-wide compaction work becomes **V3**.

If benchmarks reveal a severe correctness or pathological-performance defect before freeze, fix only the smallest necessary V2.4 surface, rerun the full comparison, and document the deviation.

---

# Validation

Before publishing benchmark results, rerun:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Miri must remain green through the existing GitHub workflow.

Build/run benchmark suite:

```sh
cargo run --release -p compact_std --example benchmark_compare --features json,toml
```

Provide child-mode commands only as internal/debugging interfaces, not as the normal user workflow.

Run the full benchmark on one stable machine without other deliberately heavy workloads.

Record:

```text
OS
kernel
architecture
CPU model
logical CPU count
rustc version
cargo version
build flags
commit SHA
benchmark repetition count
```

---

# Documentation updates

Update `BENCHMARKS.md` to become the authoritative V2.4 benchmark report.

It must include:

1. methodology;
2. hardware/software environment;
3. microbenchmark summary;
4. real-world scenario summary;
5. native-vs-V2.4 time ratios;
6. native-vs-V2.4 memory ratios;
7. frozen-graph comparison;
8. long-running fragmentation/churn result;
9. limitations;
10. explicit V2.4 freeze statement.

Update `README.md` only enough to point to the native comparison report and state that V2.4 architecture is frozen after the benchmark/validation pass.

Update `ARCHITECTURE.md` only to add the freeze status if no architecture changes are required.

Do not rewrite the architecture based on benchmark preferences.

---

# Non-goals

This task does not:

- benchmark V2.2 as the primary baseline;
- redesign the V2.4 allocator;
- introduce V3;
- modify pointer width;
- add assembly optimizations;
- add a new hashing algorithm;
- replace Serde;
- optimize every benchmark loss;
- add network benchmarks requiring external services;
- use production/private datasets;
- attempt perfect malloc-overhead accounting;
- claim RSS is identical to retained logical memory.

---

# Final-diff checklist

Before declaring the benchmark/freeze pass complete:

- [ ] every headline scenario has native and V2.4 variants;
- [ ] native and compact outputs are logically verified;
- [ ] runs use fresh processes;
- [ ] warm-up and repeated measurements are used;
- [ ] median and p95 are reported;
- [ ] memory accounting distinguishes native requested bytes from cage bytes;
- [ ] V2.4 auxiliary native allocations are visible;
- [ ] real-world scenarios substantially outnumber the old synthetic examples;
- [ ] JSON and TOML comparisons use the same parser ecosystem/version;
- [ ] frozen comparison uses a conventional native immutable model, not V2.2;
- [ ] fixed-population churn checks fragmentation stability;
- [ ] concurrent allocator scenario is included;
- [ ] no benchmark relies on external network/services;
- [ ] no architecture change was made unless required by a severe measured defect;
- [ ] full tests/clippy/Miri remain green;
- [ ] `BENCHMARKS.md` records exact environment and commit;
- [ ] V2.4 freeze is documented;
- [ ] `PLAN.md` is deleted before the implementation commit.

---

# Execution handoff

Implement this plan exactly from the latest `main`.

1. Verify the V2.4 architecture has not advanced before starting.
2. Read this plan completely.
3. Build the benchmark measurement/process-isolation framework first.
4. Add microbenchmarks only as diagnostics.
5. Add all realistic scenarios with paired native/compact implementations.
6. Verify logical equivalence before trusting timings.
7. Run the suite repeatedly on one machine/toolchain.
8. Investigate only correctness failures or genuinely pathological results; do not redesign V2.4 to win normal benchmarks.
9. Run the full validation suite after any necessary fix.
10. Update `BENCHMARKS.md` with the final native-vs-V2.4 results.
11. Mark V2.4 architecture frozen in documentation.
12. Review the final diff against this plan.
13. Delete `PLAN.md`.
14. Commit the benchmark/freeze implementation without planning files remaining.

Executor handoff should begin with:

> Implement PLAN.md exactly. Verify latest main first. Benchmark native Rust against V2.4 only. Preserve V2.4 architecture unless correctness or a genuinely pathological benchmark requires the smallest possible fix. Run the full validation suite, publish the benchmark report, freeze V2.4, delete PLAN.md, and commit.
