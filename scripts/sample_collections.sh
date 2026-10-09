#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
profile_out=${PROFILE_OUT:-/tmp/csl-collection-profile-r4}
profile_binary=${PROFILE_BINARY:-}
time_runs=${PERF_TIME_RUNS:-30}
stat_runs=${PERF_STAT_RUNS:-30}
stat_repeat=${PERF_STAT_REPEAT:-5}
record_runs=${PERF_RECORD_RUNS:-500}
record_frequency=${PERF_RECORD_FREQUENCY:-997}
run_id=${PROFILE_RUN_ID:-$(date -u +'%Y%m%dT%H%M%SZ')-$$}
[[ "$run_id" =~ ^[A-Za-z0-9_.-]+$ ]] || {
    echo "PROFILE_RUN_ID contains unsupported characters" >&2
    exit 2
}
run_out="$profile_out/$run_id"

if [[ ! -x "$profile_binary" ]]; then
    echo "Set PROFILE_BINARY to the hashed benchmark_profile executable from profile_cpu.sh build." >&2
    exit 2
fi
if [[ $(basename "$profile_binary") != benchmark_profile-* ]]; then
    echo "PROFILE_BINARY must be the hashed Cargo benchmark_profile artifact." >&2
    exit 2
fi
profile_binary=$(realpath "$profile_binary")

for count in "$time_runs" "$stat_runs" "$stat_repeat" "$record_runs"; do
    [[ "$count" =~ ^[1-9][0-9]*$ ]] || {
        echo "run counts must be positive integers" >&2
        exit 2
    }
done

local_perf=(perf)
if [[ ${PERF_USE_SUDO:-1} == 1 ]]; then
    local_perf=(sudo -n perf)
fi
if ! "${local_perf[@]}" stat -e cpu-clock:u -- true >/dev/null 2>&1; then
    echo "perf access is unavailable; no collection samples were collected." >&2
    exit 2
fi

read_checksum() {
    local input=$1
    local -a values=()
    mapfile -t values < <(
        awk -F '\t' '$1 == "META" && $2 == "checksum" {print $5}' "$input" | sort -u
    )
    if (( ${#values[@]} != 1 )); then
        echo "expected one stable checksum in $input; found ${#values[@]}" >&2
        return 1
    fi
    printf '%s\n' "${values[0]}"
}

assert_checksum() {
    local expected=$1
    local actual=$2
    local label=$3
    if [[ "$expected" != "$actual" ]]; then
        echo "$label checksum mismatch: expected=$expected actual=$actual" >&2
        return 1
    fi
}

mkdir -p "$run_out/event-probes"
{
    printf 'recorded_utc='
    date -u +'%Y-%m-%dT%H:%M:%SZ'
    printf 'run_id=%s\n' "$run_id"
    printf 'source_commit='
    git -C "$repo_root" rev-parse HEAD
    uname -a
    lscpu
    rustc -Vv
    cargo -V
    perf --version
    printf 'profile_binary=%s\n' "$profile_binary"
    sha256sum "$profile_binary"
    printf 'timing_mode=benchmark_profile; no counting global allocator; no per-phase allocator snapshots\n'
    printf 'time_runs=%s stat_runs=%s stat_repeat=%s record_runs=%s\n' \
        "$time_runs" "$stat_runs" "$stat_repeat" "$record_runs"
} > "$run_out/environment.txt" 2>&1

events=(cycles instructions branches branch-misses cache-misses)
supported_events=()
for event in "${events[@]}"; do
    probe="$run_out/event-probes/$event.txt"
    probe_error="$run_out/event-probes/$event.stderr"
    if "${local_perf[@]}" stat --no-big-num -x, -e "$event" -o "$probe" \
        -- sleep 0.1 >/dev/null 2>"$probe_error" \
        && ! grep -Eiq 'not supported|permission denied|error:' "$probe" "$probe_error"; then
        supported_events+=("$event")
        printf '%s\tavailable\n' "$event" >> "$run_out/event-status.tsv"
    else
        printf '%s\tunavailable\n' "$event" >> "$run_out/event-status.tsv"
    fi
done

for scenario in A4 A5 B6; do
    native_checksum=
    compact_checksum=
    for variant in native compact; do
        prefix="$run_out/$scenario-$variant"
        /usr/bin/time -v -o "$prefix.time.txt" \
            "$profile_binary" --window "$scenario" "$variant" "$time_runs" \
            --seconds 0 > "$prefix.time.tsv" 2>&1
        time_checksum=$(read_checksum "$prefix.time.tsv")

        stat_checksum=$time_checksum
        if (( ${#supported_events[@]} > 0 )); then
            event_csv=$(IFS=,; printf '%s' "${supported_events[*]}")
            "${local_perf[@]}" stat --no-big-num --repeat "$stat_repeat" \
                -e "$event_csv" -o "$prefix.perf-stat.txt" -- \
                "$profile_binary" --window "$scenario" "$variant" "$stat_runs" \
                --seconds 0 > "$prefix.perf-stat.tsv" 2>&1
            stat_checksum=$(read_checksum "$prefix.perf-stat.tsv")
            assert_checksum "$time_checksum" "$stat_checksum" "$scenario/$variant perf stat"
        else
            printf 'no hardware counters supported; perf stat skipped\n' \
                > "$prefix.perf-stat.txt"
        fi

        data="$prefix.perf.data"
        "${local_perf[@]}" record --freq "$record_frequency" \
            --call-graph fp -o "$data" -- \
            "$profile_binary" --window "$scenario" "$variant" "$record_runs" \
            --seconds 0 > "$prefix.perf-record.tsv" 2>&1
        if [[ ${PERF_USE_SUDO:-1} == 1 ]]; then
            sudo -n chown "$(id -u):$(id -g)" "$data"
        fi
        record_checksum=$(read_checksum "$prefix.perf-record.tsv")
        assert_checksum "$time_checksum" "$record_checksum" "$scenario/$variant perf record"

        perf report --stdio --children --sort symbol -i "$data" \
            > "$prefix.perf-report.txt"
        if [[ $variant == native ]]; then
            native_checksum=$time_checksum
        else
            compact_checksum=$time_checksum
        fi
    done
    assert_checksum "$native_checksum" "$compact_checksum" "$scenario native/compact"
done

printf 'Wrote checksum-verified collection timing and profiles to %s\n' "$run_out"
