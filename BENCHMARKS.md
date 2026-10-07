# Benchmarks

These are V2.1.0 development measurements, not performance guarantees. Run the
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
| Boolean | 4.73 / 5.03 ms | 10.72 / 7.95 ms |
| 3-bit | 5.37 / 10.36 ms | 11.40 / 12.89 ms |
| Cross-byte 9-bit | 18.47 / 28.35 ms | 29.00 / 29.46 ms |
| Aligned 16-bit | 18.48 / 48.58 ms | 29.17 / 50.50 ms |
| Aligned 32-bit | 12.73 / 95.95 ms | 27.49 / 98.50 ms |

The boolean write fast path was slower than the reference loop in this sample
and should not be treated as an optimized guarantee.

## Allocation sample

In the same run:

- growing a `CompactVec<u32>` to 65,536 elements took 576 microseconds;
- final arena use was 262,160 bytes including allocator overhead;
- traversal took 8.88 microseconds;
- 10,000 tail allocations took 187 microseconds;
- 10,000 allocate/drop/reuse rounds took 147 microseconds.

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
| 8 bytes | 240 µs | 437 µs |
| 16 bytes | 250 µs | 430 µs |
| 20 bytes | 329 µs | 442 µs |
| 24 bytes | 1,540 µs | 440 µs |
| 64 bytes | 3,024 µs | 435 µs |

This is one local optimized run, not a performance guarantee. Compact inline
payloads avoid native allocation cost, while heap-backed compact buffers carry
allocator metadata and can lose to `Vec<u8>` for short-lived standalone
payloads. Measure the complete workload before replacing temporary native byte
vectors.

## Performance risks to monitor

The current free-range search is first-fit and linear in the number of free
ranges. Highly fragmented arenas can therefore increase allocation search
cost.

Small owned allocations also pay fixed allocator metadata overhead. Workloads
with very many tiny independent allocations should measure total metadata
cost, not only payload density.
