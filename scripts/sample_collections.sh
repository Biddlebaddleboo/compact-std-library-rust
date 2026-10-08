#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile_out="${PROFILE_OUT:-/tmp/csl-collection-profile}"
stat_runs="${PERF_STAT_RUNS:-30}"
record_runs="${PERF_RECORD_RUNS:-500}"
benchmark="$repo_root/target/release/examples/benchmark_compare"

mkdir -p "$profile_out"
if ! sudo -n perf stat -e cycles,instructions -- true >/dev/null 2>&1; then
    printf 'sudo perf access is unavailable; no samples were collected.\n' >&2
    exit 2
fi

for scenario in A4 A5 B6; do
    for variant in native compact; do
        output="$profile_out/${scenario}-${variant}"
        /usr/bin/time -v -o "${output}.profile-baseline-time.txt" \
            "$benchmark" --child "$scenario" "$variant" "$record_runs" \
            >"${output}.profile-baseline.tsv"
        sudo -n perf stat --repeat 5 \
            -e cycles,instructions,branches,branch-misses,cache-misses \
            -o "${output}.perf-stat.txt" \
            -- "$benchmark" --child "$scenario" "$variant" "$stat_runs" \
            >"${output}.perf-stat-benchmark.tsv"
        /usr/bin/time -v -o "${output}.perf-record-time.txt" \
            sudo -n perf record --freq 997 --call-graph dwarf,8192 \
            -o "${output}.perf.data" \
            -- "$benchmark" --child "$scenario" "$variant" "$record_runs" \
            >"${output}.perf-record-benchmark.tsv"
        sudo -n perf report --stdio --children --sort symbol \
            -i "${output}.perf.data" >"${output}.perf-report.txt"
    done
done

printf 'Wrote sampled profiles to %s\n' "$profile_out"
