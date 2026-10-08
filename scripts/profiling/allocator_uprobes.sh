#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
Usage: ALLOCATOR_PROFILE_WINDOW=coordinated PROFILE_BINARY=/absolute/path/to/benchmark_compare \
  bash scripts/profiling/allocator_uprobes.sh A2|B8|B10

PROFILE_BINARY must be the release benchmark_compare binary built without
allocator-telemetry or allocator-policy features. The generated uprobes record
lock-call latency, allocation payload/alignment, release batch length, and
release/merge function duration. A transaction-drop probe also attempts to
estimate critical-section duration, but the pairing can undercount; validate
that histogram before interpreting it. This script does not build or modify
the benchmark or allocator.
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
    A2) repetitions=120 ;;
    B8) repetitions=12 ;;
    B10) repetitions=80 ;;
    *) usage; exit 2 ;;
esac

binary=${PROFILE_BINARY:-target/release/examples/benchmark_compare}
if [[ ! -x "$binary" ]]; then
    echo "PROFILE_BINARY is missing or not executable: $binary" >&2
    exit 2
fi
binary=$(realpath "$binary")
if ! command -v bpftrace >/dev/null 2>&1 || ! command -v nm >/dev/null 2>&1; then
    echo "bpftrace and nm are required for allocator uprobes." >&2
    exit 2
fi

out_dir=${ALLOCATOR_PROFILE_OUTPUT_DIR:-"/tmp/compact-allocator-profile-$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out_dir"
out_dir=$(realpath "$out_dir")

find_symbol() {
    local prefix=$1
    nm --format=posix "$binary" 2>/dev/null |
        awk -v prefix="$prefix" 'BEGIN { found = 0 } !found && $1 ~ ("^" prefix) { print $1; found = 1 }'
}

lock_symbol=$(find_symbol '_ZN19compact_backend_std4cage4lock')
allocate_symbol=$(find_symbol '_ZN19compact_backend_std4cage14allocate_block')
release_symbol=$(find_symbol '_ZN19compact_backend_std4cage19release_many_locked')
merge_symbol=$(find_symbol '_ZN19compact_backend_std4cage24merge_free_ranges_locked')
drop_symbols=$(nm --format=posix "$binary" 2>/dev/null |
    awk '$1 ~ /^_ZN4core3ptr68drop_in_place/ && $1 ~ /AllocatorTransaction/ { print $1 }' |
    sort -u)

for symbol in "$lock_symbol" "$allocate_symbol" "$release_symbol" "$merge_symbol"; do
    if [[ -z "$symbol" ]]; then
        echo "Required allocator symbol is absent from $binary; keep the profile artifact and report this limitation." >&2
        exit 2
    fi
done
if [[ -z "$drop_symbols" ]]; then
    echo "AllocatorTransaction drop glue is absent from $binary; critical-section duration cannot be paired." >&2
    exit 2
fi

trace_script="$out_dir/$scenario.allocator-uprobes.bt"
trace_output="$out_dir/$scenario.allocator-uprobes.txt"
run_output="$out_dir/$scenario.uprobes-run.log"
command_line=$(printf '%q --child %q compact %q' "$binary" "$scenario" "$repetitions")

{
    cat <<EOF
uprobe:$binary:$lock_symbol /pid == cpid/ {
    @lock_start[tid] = nsecs;
    @lock_calls = count();
}
uretprobe:$binary:$lock_symbol /pid == cpid && @lock_start[tid]/ {
    @lock_latency_us = hist((nsecs - @lock_start[tid]) / 1000);
    @hold_start[tid] = nsecs;
    delete(@lock_start[tid]);
}
uprobe:$binary:$allocate_symbol /pid == cpid/ {
    @allocation_payload_bytes[arg2] = count();
    @allocation_alignment[arg3] = count();
}
uprobe:$binary:$release_symbol /pid == cpid/ {
    @release_start[tid] = nsecs;
    @release_batch_size[arg3] = count();
}
uretprobe:$binary:$release_symbol /pid == cpid && @release_start[tid]/ {
    @release_locked_us = hist((nsecs - @release_start[tid]) / 1000);
    delete(@release_start[tid]);
}
uprobe:$binary:$merge_symbol /pid == cpid/ {
    @merge_start[tid] = nsecs;
}
uretprobe:$binary:$merge_symbol /pid == cpid && @merge_start[tid]/ {
    @merge_us = hist((nsecs - @merge_start[tid]) / 1000);
    delete(@merge_start[tid]);
}
uprobe:$binary:*AllocatorTransaction* /pid == cpid && @hold_start[tid]/ {
    @critical_section_us = hist((nsecs - @hold_start[tid]) / 1000);
    delete(@hold_start[tid]);
}
EOF
    cat <<'EOF'
interval:ms:500 {
    print(@lock_calls); clear(@lock_calls);
    print(@lock_latency_us); clear(@lock_latency_us);
    print(@critical_section_us); clear(@critical_section_us);
    print(@allocation_payload_bytes); clear(@allocation_payload_bytes);
    print(@allocation_alignment); clear(@allocation_alignment);
    print(@release_batch_size); clear(@release_batch_size);
    print(@release_locked_us); clear(@release_locked_us);
    print(@merge_us); clear(@merge_us);
}
EOF
} > "$trace_script"

{
    date -u '+captured_at_utc=%Y-%m-%dT%H:%M:%SZ'
    printf 'scenario=%s\nrepetitions=%s\nbinary=%s\n' "$scenario" "$repetitions" "$binary"
    git rev-parse HEAD
    uname -a
    rustc -Vv
    bpftrace --version
    printf 'lock_symbol=%s\nallocate_block_symbol=%s\nrelease_many_locked_symbol=%s\nmerge_symbol=%s\n' \
        "$lock_symbol" "$allocate_symbol" "$release_symbol" "$merge_symbol"
    printf 'allocator_transaction_drop_symbols=%s\n' "$drop_symbols"
    sha256sum "$binary"
} > "$out_dir/$scenario.uprobes-environment.txt" 2>&1

if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" > "$run_output" 2>&1
else
    if ! command -v sudo >/dev/null 2>&1; then
        echo "Run this script as root; perf_event_paranoid=4 blocks unprivileged BPF probes." >&2
        exit 2
    fi
    sudo -n bpftrace -B line -o "$trace_output" -c "$command_line" "$trace_script" > "$run_output" 2>&1
fi

printf 'uprobes=%s\nrun_log=%s\nenvironment=%s\nscript=%s\n' \
    "$trace_output" "$run_output" "$out_dir/$scenario.uprobes-environment.txt" "$trace_script"
