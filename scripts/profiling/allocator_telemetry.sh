#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: ALLOCATOR_PROFILE_WINDOW=coordinated TELEMETRY_BINARY=/absolute/path/to/benchmark_compare \
  bash scripts/profiling/allocator_telemetry.sh A2|B8|B10

TELEMETRY_BINARY must be the release benchmark_compare binary built with
json,toml,allocator-telemetry and no allocator-policy feature. This script
does not build. Telemetry timings are diagnostic only and are not comparable
to production release timings.
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
    A2) repetitions=15 ;;
    B8) repetitions=9 ;;
    B10) repetitions=7 ;;
    *) usage; exit 2 ;;
esac

binary=${TELEMETRY_BINARY:-target/release/examples/benchmark_compare-telemetry}
if [[ ! -x "$binary" ]]; then
    echo "TELEMETRY_BINARY is missing or not executable: $binary" >&2
    exit 2
fi
binary=$(realpath "$binary")

out_dir=${ALLOCATOR_PROFILE_OUTPUT_DIR:-"/tmp/compact-allocator-profile-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out_dir"
out_dir=$(realpath "$out_dir")

{
    date -u '+captured_at_utc=%Y-%m-%dT%H:%M:%SZ'
    printf 'scenario=%s\nrepetitions=%s\nbinary=%s\n' "$scenario" "$repetitions" "$binary"
    git rev-parse HEAD
    uname -a
    rustc -Vv
    sha256sum "$binary"
} > "$out_dir/$scenario.telemetry-environment.txt" 2>&1

"$binary" --child "$scenario" compact "$repetitions" > "$out_dir/$scenario.telemetry.log" 2>&1
printf 'telemetry=%s\nenvironment=%s\n' \
    "$out_dir/$scenario.telemetry.log" "$out_dir/$scenario.telemetry-environment.txt"
