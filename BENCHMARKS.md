# V2.4 benchmark results

Measured on 2026-10-07 in one release-mode run per example. These are
environment-specific samples, not statistically repeated measurements or a
comparison against another allocator.

## Environment

- Ubuntu 24.04.4 LTS, Linux `6.17.0-1020-oracle`
- AArch64, ARM Neoverse-N1, 2 vCPUs
- `rustc 1.95.0 (59807616e 2026-04-14)` and Cargo 1.95.0
- Built with `cargo build --workspace --all-features --release --offline`

## Runtime benchmark

Command: `cargo run --release --offline -p compact_std --example runtime_bench`

```text
owner sizes: Vec<u32>=4B Box<u64>=4B CompactBytes=24B
vector growth + traversal: 4.366153ms; checksum=4999950000; used=524304B
64-byte values: 18.4685ms
```

## Collection and workload benchmark

Command: `cargo run --release --offline -p compact_std --example workloads_bench --features json,toml`

```text
Vec growth and traversal: elapsed=2.848861ms; live_cage_delta=262160B
VecDeque FIFO: elapsed=65.481µs; live_cage_delta=8208B
HashMap and HashSet: elapsed=1.565132ms; live_cage_delta=114752B
chunk assembly: elapsed=176.321µs; live_cage_delta=303120B
String append: elapsed=208.121µs; live_cage_delta=106512B
64 KiB Bytes: elapsed=5µs; live_cage_delta=65552B
PathBuf workload: elapsed=233.602µs; live_cage_delta=80016B
direct TOML config: elapsed=188.642µs; live_cage_delta=240B
scratch allocations: elapsed=10.28µs; live_cage_delta=65552B
frozen catalog: elapsed=15.16µs; live_cage_delta=16432B
final live cage bytes=0
```
