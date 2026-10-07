# Benchmark status

V2.3 benchmark runs were intentionally skipped for this completion. There are
no V2.3 timing, throughput, or memory-use results recorded here, and the
workspace makes no performance comparison or regression claim.

The benchmark examples are kept for a future measurement pass:

```sh
cargo run --release --example runtime_bench -p compact_std
cargo run --release --example workloads_bench -p compact_std --features json,toml
```

They cover runtime allocation and common collection, serialization, scratch,
and frozen-graph workloads. Run them on the target platform and record the
toolchain, platform, workload settings, and complete output before publishing
results. These commands are documentation only; they were not run as part of
the V2.3 completion.
