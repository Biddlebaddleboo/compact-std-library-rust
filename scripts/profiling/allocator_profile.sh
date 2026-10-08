#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: ALLOCATOR_PROFILE_WINDOW=coordinated PROFILE_BINARY=/absolute/path/to/benchmark_compare \
  bash scripts/profiling/allocator_profile.sh A2|B8|B10

PROFILE_BINARY must be the release benchmark_compare binary built without
allocator-telemetry or allocator-policy features. This script does not build.
The bpftrace profile runs the selected compact child for a bounded number of
repetitions and records user stacks at 99 Hz.
EOF
}

if [[ "${ALLOCATOR_PROFILE_WINDOW:-}" != "coordinated" ]]; then
    echo "Refusing to capture: root must coordinate the shared-host measurement window first." >&2
    exit 2
fi

if [[ $# -ne 1 ]]; then
    usage
    exit 2
fi

scenario=$1
case "$scenario" in
    A2) repetitions=800 ;;
    B8) repetitions=45 ;;
    B10) repetitions=500 ;;
    *) usage; exit 2 ;;
esac

binary=${PROFILE_BINARY:-target/release/examples/benchmark_compare}
if [[ ! -x "$binary" ]]; then
    echo "PROFILE_BINARY is missing or not executable: $binary" >&2
    exit 2
fi
binary=$(realpath "$binary")

if ! command -v bpftrace >/dev/null 2>&1; then
    echo "bpftrace is required for this user-stack probe." >&2
    exit 2
fi

out_dir=${ALLOCATOR_PROFILE_OUTPUT_DIR:-"/tmp/compact-allocator-profile-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out_dir"
out_dir=$(realpath "$out_dir")

{
    date -u '+captured_at_utc=%Y-%m-%dT%H:%M:%SZ'
    printf 'scenario=%s\nrepetitions=%s\nbinary=%s\n' "$scenario" "$repetitions" "$binary"
    git rev-parse HEAD
    uname -a
    rustc -Vv
    bpftrace --version
    printf 'perf_event_paranoid='
    cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true
    sha256sum "$binary"
} > "$out_dir/$scenario.environment.txt" 2>&1

command_line=$(printf '%q --child %q compact %q' "$binary" "$scenario" "$repetitions")
trace_script=$(dirname "$0")/allocator_sample.bt
trace_script=$(realpath "$trace_script")
trace_output="$out_dir/$scenario.user-stacks.txt"
run_output="$out_dir/$scenario.profile-run.log"

if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" > "$run_output" 2>&1
else
    if ! command -v sudo >/dev/null 2>&1; then
        echo "Run this script as root; perf_event_paranoid=4 blocks unprivileged sampling." >&2
        exit 2
    fi
    sudo -n bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" > "$run_output" 2>&1
fi

printf 'profile=%s\nrun_log=%s\nenvironment=%s\n' \
    "$trace_output" "$run_output" "$out_dir/$scenario.environment.txt"
