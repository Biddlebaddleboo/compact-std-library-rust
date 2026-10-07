# Benchmarks

These are V2.2.0 development measurements, not performance guarantees. Run the
benchmark on the target environment before making capacity or latency
decisions.

## Command

```bash
cargo run --release -p compact_std --example runtime_bench
```

The checked reference implementation used by the benchmark is a simple
bit-by-bit loop used only for comparison.

## Packed-field sample

One local run used 2,000,000 operations per packed case:

| Field | Read fast / reference | Write fast / reference |
| --- | ---: | ---: |
| Boolean | 4.74 / 5.08 ms | 10.71 / 7.38 ms |
| 3-bit | 5.36 / 10.18 ms | 11.41 / 12.86 ms |
| Cross-byte 9-bit | 18.42 / 27.78 ms | 29.38 / 29.47 ms |
| Aligned 16-bit | 18.36 / 48.47 ms | 28.85 / 50.29 ms |
| Aligned 32-bit | 13.16 / 95.81 ms | 27.51 / 99.09 ms |

The boolean write fast path was slower than the reference loop in this sample
and should not be treated as an optimized guarantee.

## Allocation sample

In the same run:

- growing a `CompactVec<u32>` to 65,536 elements took 583 microseconds;
- final arena use was 262,160 bytes including allocator overhead;
- traversal took 9.12 microseconds;
- 10,000 tail allocations took 175 microseconds;
- 10,000 allocate/drop/reuse rounds took 163 microseconds.

These numbers are single-run local observations. CPU, optimization level,
allocator fragmentation, element type, backing size, and target architecture
can materially change results.

## Compact byte payload sample

One release-mode run compared 20,000 construction/read/drop rounds per
payload. The candidate wrapper sizes were 24 bytes for inline capacities 12,
16, and 20; capacity 24 increased the wrapper to 32 bytes. The implementation
uses 20 inline bytes because it avoids arena allocation for 17–20 byte payloads
without increasing wrapper size.

| Payload | `CompactBytes` (20 inline) | `Vec<u8>` |
| --- | ---: | ---: |
| 8 bytes | 283 µs | 427 µs |
| 16 bytes | 252 µs | 442 µs |
| 20 bytes | 347 µs | 433 µs |
| 24 bytes | 1,573 µs | 431 µs |
| 64 bytes | 3,007 µs | 435 µs |

This is one local optimized run, not a performance guarantee. Compact inline
payloads avoid native allocation cost, while heap-backed compact buffers carry
allocator metadata and can lose to `Vec<u8>` for short-lived standalone
payloads. Measure the complete workload before replacing temporary native byte
vectors.

## V2.2 workload suite

Run the reproducible workload suite with:

```bash
cargo run --release -p compact_std --example workloads_bench --features json,toml
```

It measures compact `Vec` growth/traversal, `VecDeque`, a 1,000-entry bounded
log ring, a 1,024-fragment chunk transfer, `HashMap` and `HashSet`, `String`,
64 KiB of `Bytes`, 1,000 `PathBuf` values, TOML configuration parsing,
`format!` and `collect` rewrites, scratch allocations, freeze, and frozen graph
reads. The config case uses paths and strings; the frozen case traverses a
4,096-name catalog.

Each line reports logical payload bytes, arena use at the end of the workload,
the observed arena high-water while owners are live, free bytes after cleanup,
and runtime. `arena_overhead_estimate` is high-water arena use minus logical
payload; it includes collection capacity, allocator headers, and alignment
padding. The scratch line reports its nested arena high-water separately
because the parent releases the scratch backing when the scope ends.

`native_alloc_events` and `native_alloc_bytes` come from a counting global
allocator. They include the fixed arena backing for each workload and any
native allocations made by parsing or freezing; a `realloc` counts as one event
and records its requested replacement size. The fragmentation case releases
alternating owned blocks, reports the 128 deliberately separated free ranges
and their recovered bytes, then allocates a replacement payload. These counters
are instrumentation for repeatable local comparisons, not compatibility or
performance guarantees.

Measurements vary with CPU, compiler, allocator, and workload state. The
executable prints the full counters; run it on the deployment target before
choosing capacities or making latency claims.

One run on Linux aarch64 with Rust 1.95.0 reported:

| Workload | Payload | Arena high-water | Runtime |
| --- | ---: | ---: | ---: |
| `Vec` growth/traversal | 80,000 B | 131,088 B | 290 µs |
| `VecDeque` FIFO | 8,000 B | 8,208 B | 25.8 µs |
| Bounded 1,000-entry log ring | 39,000 B | 80,016 B | 221 µs |
| 1,024-fragment chunk transfer | 524,288 B | 565,280 B | 1.18 ms |
| `HashMap` + `HashSet` | 16,000 B | 45,120 B | 115 µs |
| `String` append | 26,000 B | 26,640 B | 58.8 µs |
| 64 KiB of `Bytes` | 65,536 B | 65,552 B | 121 µs |
| 1,000 `PathBuf` values | 40,000 B | 80,016 B | 176 µs |
| TOML service config | 99 B | 304 B | 190 µs |
| Scratch vectors/formatting | 2,068 B | 2,144 B nested arena | 6.2 µs |
| `format!` + `collect` rewrites | 20,025 B | 68,800 B | 185 µs |
| Alternating-block fragmentation | 65,536 B | 106,528 B | 421 µs |
| Freeze 4,096-name catalog | 40,987 B | 98,360 B source; 106,576 B frozen | 39.8 ms |
| Frozen catalog traversal | 40,987 B | 106,576 B frozen | 62.4 µs |

This is one local sample. In the executable output, `arena_current` is sampled
after each workload's owners have been dropped, so it can be zero while the
high-water still records the storage used during the operation.

## Performance risks to monitor

The current free-range search is first-fit and linear in the number of free
ranges. Highly fragmented arenas can therefore increase allocation search
cost.

Small owned allocations also pay fixed allocator metadata overhead. Workloads
with very many tiny independent allocations should measure total metadata
cost, not only payload density.
