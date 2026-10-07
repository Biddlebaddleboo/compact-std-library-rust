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

## Performance risks to monitor

The current free-range search is first-fit and linear in the number of free
ranges. Highly fragmented arenas can therefore increase allocation search
cost.

Small owned allocations also pay fixed allocator metadata overhead. Workloads
with very many tiny independent allocations should measure total metadata
cost, not only payload density.
