#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile_out="${PROFILE_OUT:-/tmp/csl-collection-profile}"
profile_runs="${PROFILE_RUNS:-30}"
benchmark_runs="${BENCHMARK_RUNS:-30}"

mkdir -p "$profile_out"
cargo build --manifest-path "$repo_root/Cargo.toml" --release -p compact_std \
    --features json,toml --example benchmark_compare --example collection_profile

benchmark="$repo_root/target/release/examples/benchmark_compare"
diagnostic="$repo_root/target/release/examples/collection_profile"

for scenario in A4 A5 B6; do
    for variant in native compact; do
        output="$profile_out/${scenario}-${variant}"
        /usr/bin/time -v -o "${output}.time.txt" \
            "$benchmark" --child "$scenario" "$variant" "$benchmark_runs" \
            >"${output}.benchmark.tsv"
        /usr/bin/time -v -o "${output}.diagnostic-time.txt" \
            "$diagnostic" --scenario "$scenario" --variant "$variant" --runs "$profile_runs" \
            >"${output}.diagnostic.tsv"
    done
done

printf 'Wrote collection timing and RSS artifacts to %s\n' "$profile_out"
