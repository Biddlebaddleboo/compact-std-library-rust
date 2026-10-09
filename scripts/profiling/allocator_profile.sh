#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

usage() {
    cat >&2 <<'EOF'
Usage: ALLOCATOR_PROFILE_WINDOW=coordinated PROFILE_BINARY=/absolute/path/to/benchmark_profile-HASH \
  bash scripts/profiling/allocator_profile.sh A2|B8|B10 [native|compact]

PROFILE_BINARY must be the hashed, frame-pointer benchmark_profile release
artifact built without allocator-telemetry. This binary uses System directly,
does not install the counting allocator, and does not snapshot allocator state
inside the measured workload windows. This script does not build.
EOF
}

if [[ ${ALLOCATOR_PROFILE_WINDOW:-} != coordinated ]]; then
    echo "Refusing to capture: root must coordinate the shared-host measurement window first." >&2
    exit 2
fi

if [[ $# -lt 1 || $# -gt 2 ]]; then
    usage
    exit 2
fi

scenario=$1
variant=${2:-compact}
case "$scenario" in
    A2) repetitions=800 ;;
    B8) repetitions=45 ;;
    B10) repetitions=500 ;;
    *) usage; exit 2 ;;
esac
case "$variant" in
    native|compact) ;;
    *) usage; exit 2 ;;
esac

binary=${PROFILE_BINARY:-}
if [[ ! -x "$binary" ]]; then
    echo "PROFILE_BINARY is missing or not executable: $binary" >&2
    exit 2
fi
if [[ $(basename "$binary") != benchmark_profile-* ]]; then
    echo "PROFILE_BINARY must be the hashed Cargo benchmark_profile artifact." >&2
    exit 2
fi
binary=$(realpath "$binary")
seconds=${PROFILE_SECONDS:-5}
[[ "$seconds" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
    echo "PROFILE_SECONDS must be a non-negative number" >&2
    exit 2
}

if ! command -v bpftrace >/dev/null 2>&1; then
    echo "bpftrace is required for this user-stack probe." >&2
    exit 2
fi

out_dir=${ALLOCATOR_PROFILE_OUTPUT_DIR:-"/tmp/compact-allocator-profile-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out_dir"
out_dir=$(realpath "$out_dir")

{
    date -u '+captured_at_utc=%Y-%m-%dT%H:%M:%SZ'
    printf 'scenario=%s\nvariant=%s\nrepetitions=%s\nseconds=%s\nbinary=%s\n' \
        "$scenario" "$variant" "$repetitions" "$seconds" "$binary"
    git -C "$repo_root" rev-parse HEAD
    uname -a
    rustc -Vv
    cargo -V
    bpftrace --version
    printf 'perf_event_paranoid='
    cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || true
    printf 'sample=99 Hz user stacks; pid filter includes all threads in the workload process\n'
    sha256sum "$binary"
} > "$out_dir/$scenario-$variant.environment.txt" 2>&1

command_line=$(printf '%q --window %q %q %q --seconds %q' \
    "$binary" "$scenario" "$variant" "$repetitions" "$seconds")
trace_script=$(dirname "$0")/allocator_sample.bt
trace_script=$(realpath "$trace_script")
trace_output="$out_dir/$scenario-$variant.user-stacks.txt"
run_output="$out_dir/$scenario-$variant.profile-run.log"

if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" \
        > "$run_output" 2>&1
else
    if ! command -v sudo >/dev/null 2>&1; then
        echo "Run this script as root; perf_event_paranoid blocks unprivileged sampling." >&2
        exit 2
    fi
    sudo -n bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" \
        > "$run_output" 2>&1
    sudo -n chown "$(id -u):$(id -g)" "$trace_output"
fi

mapfile -t checksums < <(
    awk -F '\t' '$1 == "META" && $2 == "checksum" {print $5}' "$run_output" | sort -u
)
if (( ${#checksums[@]} != 1 )); then
    echo "expected one stable checksum in $run_output; found ${#checksums[@]}" >&2
    exit 1
fi
printf '%s\n' "${checksums[0]}" > "$out_dir/$scenario-$variant.checksum.txt"

printf 'profile=%s\nrun_log=%s\nenvironment=%s\nchecksum=%s\n' \
    "$trace_output" "$run_output" "$out_dir/$scenario-$variant.environment.txt" \
    "${checksums[0]}"
