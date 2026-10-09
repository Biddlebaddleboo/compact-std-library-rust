#!/usr/bin/env bash
set -euo pipefail

# Reproducible V2.5 runs. Keep artifacts outside the repository and use the
# benchmark_profile binary for timing and sampling; it installs no counting
# global allocator and does not snapshot allocator state in timed windows.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${PROFILE_OUT:-/tmp/csl-v25-cpu-profile}
BUILD_ROOT=${PROFILE_BUILD_ROOT:-$OUT}
if [[ -n ${PROFILE_RUN_ID:-} ]]; then
    RUN_ID=$PROFILE_RUN_ID
elif [[ ( ${1:-} == reports || ${1:-} == summary ) && -f "$OUT/latest-profile-run" ]]; then
    IFS= read -r RUN_ID < "$OUT/latest-profile-run"
else
    RUN_ID="$(date -u +'%Y%m%dT%H%M%SZ')-$$"
fi
[[ "$RUN_ID" =~ ^[A-Za-z0-9_.-]+$ ]] || {
    echo "PROFILE_RUN_ID contains unsupported characters" >&2
    exit 2
}
RUN_DIR="$OUT/runs/$RUN_ID"
FEATURES=json,toml
PLAIN_TARGET="$BUILD_ROOT/target-plain"
PROFILE_TARGET="$BUILD_ROOT/target-profiled"
PLAIN_BIN=${PLAIN_BIN:-}
CHECK_BIN=${CHECK_BIN:-}
PROFILE_BIN=${PROFILE_BIN:-}
SCENARIOS=(A1 A2 A3 A4 A5 A6 B1 B2 B3 B4 B5 B6 B7 B8 B9 B10)

usage() {
    cat <<'EOF'
Usage: scripts/profile_cpu.sh {metadata|build|baseline|suite|noisy|stats|profile|counters|summary|reports}

Commands:
  metadata  Record host, toolchain, source, and available artifact hashes.
  build     Build telemetry-off benchmark_profile and checksum binaries.
  baseline  Run checksum parity, measure-mode stats, two full suites, and noisy cases.
  suite     Run SUITE_RUNS full 16-scenario accounting-free suites (default 2).
  noisy     Repeat selected scenarios in alternating order (default 9 runs).
  stats     Record allocation/accounting snapshots in measure mode; never use its times.
  profile   Sample all A1-A6/B1-B10 native/compact windows directly with perf.
  counters  Repeat accounting-free runs for available hardware counters.
  summary   Rebuild the checksum-verified summary from saved suite/noisy runs.
  reports   Export inclusive/self perf reports from saved samples.

Timing uses the hashed benchmark_profile executable. It has no counting
allocator and no per-phase allocator snapshots. The benchmark_compare binary
is used only for the logical-checksum self-check.

Set PROFILE_OUT to store artifacts outside the default /tmp path. Set
PROFILE_USE_SUDO=1 on hosts that require sudo -n perf.
Set CSL_B10_WORKERS=1..8 to scale B10; the default is two workers with
8,000 records per worker.
EOF
}

need_binary() {
    local binary=$1
    local label=$2
    if [[ ! -x "$binary" ]]; then
        echo "missing $label executable: $binary; run build first or set its environment variable" >&2
        return 2
    fi
}

load_artifacts() {
    local stored_plain=
    local stored_check=
    local stored_profile=
    if [[ -f "$OUT/artifacts.tsv" ]]; then
        IFS=$'\t' read -r stored_plain stored_check stored_profile < "$OUT/artifacts.tsv"
    fi
    PLAIN_BIN=${PLAIN_BIN:-$stored_plain}
    CHECK_BIN=${CHECK_BIN:-$stored_check}
    PROFILE_BIN=${PROFILE_BIN:-$stored_profile}
}

artifact_from_json() {
    python3 - "$1" "$2" <<'PY'
import json
import os
from pathlib import Path
import sys

build_log = Path(sys.argv[1])
example_name = sys.argv[2]
candidates = set()
for line in build_log.read_text().splitlines():
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    target = message.get("target", {})
    if (
        message.get("reason") == "compiler-artifact"
        and target.get("name") == example_name
        and "example" in target.get("kind", [])
        and message.get("executable")
    ):
        stable_path = Path(message["executable"])
        for path in stable_path.parent.glob(f"{example_name}-*"):
            if path.is_file() and os.path.samefile(path, stable_path):
                candidates.add(str(path))
if len(candidates) != 1:
    raise SystemExit(
        f"expected one hashed executable for {example_name}, found {sorted(candidates)}"
    )
print(next(iter(candidates)))
PY
}

profile_repetitions() {
    case "$1" in
        A1|A3|A6) printf '%s\n' 15 ;;
        A2|A4) printf '%s\n' 5000 ;;
        A5) printf '%s\n' 3000 ;;
        B1|B2|B4) printf '%s\n' 9 ;;
        B3) printf '%s\n' 250 ;;
        B5) printf '%s\n' 3000 ;;
        B6) printf '%s\n' 60000 ;;
        B7|B9) printf '%s\n' 7 ;;
        B8) printf '%s\n' 600 ;;
        B10) printf '%s\n' 1000 ;;
        *) printf 'no profile repetition count for %s\n' "$1" >&2; return 2 ;;
    esac
}

metadata() {
    mkdir -p "$RUN_DIR"
    {
        printf 'Recorded UTC: '
        date -u +'%Y-%m-%dT%H:%M:%SZ'
        printf '\n--- source ---\n'
        git -C "$ROOT" rev-parse HEAD
        printf 'run_id=%s\n' "$RUN_ID"
        git -C "$ROOT" status --short --branch
        printf '\n--- host ---\n'
        uname -a
        lscpu
        printf 'perf_event_paranoid='
        cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || printf 'unavailable\n'
        printf 'yama/ptrace_scope='
        cat /proc/sys/kernel/yama/ptrace_scope 2>/dev/null || printf 'unavailable\n'
        printf '\n--- toolchain ---\n'
        rustc -Vv
        cargo -V
        rustup show active-toolchain
        perf --version
        printf '\n--- benchmark contract ---\n'
        printf 'features: --no-default-features --features %s\n' "$FEATURES"
        printf 'build artifact root: %s\n' "$BUILD_ROOT"
        printf 'allocator-telemetry: disabled\n'
        printf 'timing: benchmark_profile; System global allocator; no per-phase allocator snapshots\n'
        printf 'sample note: whole-process samples can include one post-window allocator validation; elapsed_ns excludes it\n'
        printf 'plain release flags: Cargo release defaults; RUSTFLAGS=unset\n'
        printf 'profile release flags: -C debuginfo=1 -C force-frame-pointers=yes\n'
        printf 'sample event: cpu-clock:u; frequency: %s Hz\n' "${PERF_FREQUENCY:-499}"
        printf '\n--- hashed artifacts ---\n'
        for binary in "$PLAIN_BIN" "$CHECK_BIN" "$PROFILE_BIN"; do
            if [[ -x "$binary" ]]; then
                sha256sum "$binary"
            fi
        done
    } > "$RUN_DIR/host.txt"
    cat "$RUN_DIR/host.txt"
}

build() {
    mkdir -p "$OUT" "$BUILD_ROOT"
    if [[ ${BUILD_PLAIN:-1} == 1 ]]; then
        env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
            CARGO_TARGET_DIR="$PLAIN_TARGET" cargo build --locked --release \
            --message-format=json \
            --no-default-features -p compact_std --features "$FEATURES" \
            --example benchmark_compare --example benchmark_profile \
            > "$OUT/plain-build.jsonl"
        PLAIN_BIN=$(artifact_from_json "$OUT/plain-build.jsonl" benchmark_profile)
        CHECK_BIN=$(artifact_from_json "$OUT/plain-build.jsonl" benchmark_compare)
    fi
    if [[ ${BUILD_PROFILE:-1} == 1 ]]; then
        env -u CARGO_ENCODED_RUSTFLAGS \
            RUSTFLAGS="-C debuginfo=1 -C force-frame-pointers=yes" \
            CARGO_TARGET_DIR="$PROFILE_TARGET" cargo build --locked --release \
            --message-format=json \
            --no-default-features -p compact_std --features "$FEATURES" \
            --example benchmark_profile > "$OUT/profile-build.jsonl"
        PROFILE_BIN=$(artifact_from_json "$OUT/profile-build.jsonl" benchmark_profile)
    fi
    need_binary "$PLAIN_BIN" "plain benchmark_profile"
    need_binary "$CHECK_BIN" "plain benchmark_compare"
    need_binary "$PROFILE_BIN" "frame-pointer benchmark_profile"
    printf '%s\t%s\t%s\n' "$PLAIN_BIN" "$CHECK_BIN" "$PROFILE_BIN" > "$OUT/artifacts.tsv"
    metadata
}

run_profile_suite() {
    local output_prefix=$1
    local order=$2
    local scenario=${3:-}
    local -a arguments=(--seconds 0 --order "$order")
    mkdir -p "$(dirname "$output_prefix")"
    if [[ -n "$scenario" ]]; then
        arguments+=(--scenario "$scenario")
    fi
    "$PLAIN_BIN" "${arguments[@]}" --output "${output_prefix}.tsv" \
        > "${output_prefix}.log" 2>&1
    if ! grep -q 'Profile windows finished with matching native/compact checksums' \
        "${output_prefix}.log"; then
        cat "${output_prefix}.log" >&2
        echo "checksum parity failed in ${output_prefix}" >&2
        return 1
    fi
    if [[ -z "$scenario" ]] && ! grep -q '16 scenario(s)' "${output_prefix}.log"; then
        cat "${output_prefix}.log" >&2
        echo "expected a full 16-scenario suite in ${output_prefix}" >&2
        return 1
    fi
}

suite() {
    need_binary "$PLAIN_BIN" "plain benchmark_profile"
    local suite_runs=${SUITE_RUNS:-2}
    [[ "$suite_runs" =~ ^[1-9][0-9]*$ ]] || {
        echo "SUITE_RUNS must be a positive integer" >&2
        return 2
    }
    mkdir -p "$RUN_DIR/suite"
    for ((run = 1; run <= suite_runs; run++)); do
        local order=alternate
        if ((run % 2 == 0)); then
            order=alternate-reversed
        fi
        run_profile_suite "$RUN_DIR/suite/run${run}" "$order"
    done
    echo "Wrote $suite_runs accounting-free full suites to $RUN_DIR/suite"
    summarize
}

noisy() {
    need_binary "$PLAIN_BIN" "plain benchmark_profile"
    local noisy_runs=${NOISY_RUNS:-9}
    local scenario_list=${NOISY_SCENARIOS:-A2,A4,A5,B6,B8,B3,B5,B10}
    [[ "$noisy_runs" =~ ^[1-9][0-9]*$ ]] || {
        echo "NOISY_RUNS must be a positive integer" >&2
        return 2
    }
    local -a selected_scenarios=()
    IFS=',' read -r -a selected_scenarios <<< "$scenario_list"
    mkdir -p "$RUN_DIR/noisy"
    for ((run = 1; run <= noisy_runs; run++)); do
        local order=alternate
        if ((run % 2 == 0)); then
            order=alternate-reversed
        fi
        for scenario in "${selected_scenarios[@]}"; do
            run_profile_suite "$RUN_DIR/noisy/run${run}-${scenario}" "$order" "$scenario"
        done
    done
    echo "Wrote $noisy_runs repeated samples for ${#selected_scenarios[@]} scenarios to $RUN_DIR/noisy"
    summarize
}

summarize() {
    python3 - "$RUN_DIR" "$RUN_DIR/summary.tsv" <<'PY'
from collections import defaultdict
from math import ceil
from pathlib import Path
import statistics
import sys

root = Path(sys.argv[1])
destination = Path(sys.argv[2])
groups = defaultdict(lambda: {"elapsed": [], "rss": [], "checksums": []})

for scope in ("suite", "noisy"):
    for path in sorted((root / scope).glob("*.tsv")):
        for line in path.read_text().splitlines():
            fields = line.split("\t")
            if len(fields) < 5 or fields[0] != "META":
                continue
            if fields[1] == "profile_window":
                scenario, variant = fields[2], fields[3]
                values = dict(zip(fields[4::2], fields[5::2]))
                total = int(values["elapsed_ns"])
                runs = int(values["runs_per_window"]) * int(values["windows"])
                groups[(scope, scenario, variant)]["elapsed"].append(total / runs)
            elif fields[1] == "peak_rss_kb":
                groups[(scope, fields[2], fields[3])]["rss"].append(int(fields[4]))
            elif fields[1] == "checksum":
                groups[(scope, fields[2], fields[3])]["checksums"].append(fields[4])

if not groups:
    raise SystemExit("no benchmark metadata found to summarize")

checks = defaultdict(dict)
for (scope, scenario, variant), values in groups.items():
    unique = set(values["checksums"])
    if len(unique) != 1:
        raise SystemExit(f"checksum changed across {scope}/{scenario}/{variant}: {sorted(unique)}")
    checks[(scope, scenario)][variant] = next(iter(unique))
for key, variants in checks.items():
    if set(variants) != {"native", "compact"} or variants["native"] != variants["compact"]:
        raise SystemExit(f"native/compact checksum mismatch for {key}: {variants}")

def p95(values):
    ordered = sorted(values)
    return ordered[max(0, ceil(0.95 * len(ordered)) - 1)]

lines = [
    "scope\tscenario\tvariant\tn\tmedian_ns_per_scenario_run\tp95_ns_per_scenario_run\tmedian_peak_rss_kb\tp95_peak_rss_kb"
]
for key in sorted(groups):
    scope, scenario, variant = key
    values = groups[key]
    elapsed = values["elapsed"]
    rss = values["rss"]
    lines.append(
        "\t".join(
            [
                scope,
                scenario,
                variant,
                str(len(elapsed)),
                f"{statistics.median(elapsed):.1f}",
                f"{p95(elapsed):.1f}",
                f"{statistics.median(rss):.1f}" if rss else "unavailable",
                f"{p95(rss):.1f}" if rss else "unavailable",
            ]
        )
    )
destination.write_text("\n".join(lines) + "\n")
print(f"Checksum-verified medians and p95 written to {destination}")
PY
}

baseline() {
    need_binary "$CHECK_BIN" "plain benchmark_compare"
    mkdir -p "$RUN_DIR/baseline"
    "$CHECK_BIN" --self-check > "$RUN_DIR/baseline/self-check.log" 2>&1
    if ! grep -q '^Self-check passed:' "$RUN_DIR/baseline/self-check.log"; then
        cat "$RUN_DIR/baseline/self-check.log" >&2
        echo "benchmark checksum self-check failed" >&2
        return 1
    fi
    metadata
    stats
    suite
    noisy
    summarize
}

stats() {
    need_binary "$CHECK_BIN" "plain benchmark_compare"
    mkdir -p "$RUN_DIR"
    "$CHECK_BIN" --mode measure --order alternate \
        --output "$RUN_DIR/measure-stats.tsv" \
        > "$RUN_DIR/measure-stats.log" 2>&1
    if ! grep -q 'Logical result checksums matched for all 16 scenarios' \
        "$RUN_DIR/measure-stats.log"; then
        cat "$RUN_DIR/measure-stats.log" >&2
        echo "measure-mode checksum parity failed" >&2
        return 1
    fi
    echo "Wrote measure-mode allocation/accounting stats to $RUN_DIR/measure-stats.tsv"
}

profile() {
    need_binary "$PROFILE_BIN" "frame-pointer benchmark_profile"
    mkdir -p "$RUN_DIR/perf-data" "$RUN_DIR/profile-logs"
    local -a perf_cmd=(perf)
    if [[ ${PROFILE_USE_SUDO:-0} == 1 ]]; then
        if [[ -n ${CSL_B10_WORKERS:-} ]]; then
            perf_cmd=(sudo -n --preserve-env=CSL_B10_WORKERS perf)
        else
            perf_cmd=(sudo -n perf)
        fi
    fi
    local frequency=${PERF_FREQUENCY:-499}
    local seconds=${PROFILE_SECONDS:-3}
    local -a selected_scenarios=("${SCENARIOS[@]}")
    if [[ -n ${PROFILE_SCENARIOS:-} ]]; then
        IFS=',' read -r -a selected_scenarios <<< "$PROFILE_SCENARIOS"
    fi
    for scenario in "${selected_scenarios[@]}"; do
        local repetitions
        repetitions=$(profile_repetitions "$scenario")
        local native_checksum= compact_checksum=
        for variant in native compact; do
            local data="$RUN_DIR/perf-data/${scenario}-${variant}.data"
            local log="$RUN_DIR/profile-logs/${scenario}-${variant}.log"
            "${perf_cmd[@]}" record -e cpu-clock:u -F "$frequency" -g \
                --call-graph fp -o "$data" -- \
                "$PROFILE_BIN" --window "$scenario" "$variant" "$repetitions" \
                --seconds "$seconds" > "$log" 2>&1
            if [[ ${PROFILE_USE_SUDO:-0} == 1 ]]; then
                sudo -n chown "$(id -u):$(id -g)" "$data"
            fi
            local -a checksums=()
            mapfile -t checksums < <(
                awk -F '\t' '$1 == "META" && $2 == "checksum" {print $5}' "$log" | sort -u
            )
            if (( ${#checksums[@]} != 1 )); then
                echo "expected one stable checksum in $log; found ${#checksums[@]}" >&2
                return 1
            fi
            local checksum=${checksums[0]}
            if [[ $variant == native ]]; then
                native_checksum=$checksum
            else
                compact_checksum=$checksum
            fi
        done
        [[ "$native_checksum" == "$compact_checksum" ]] || {
            echo "$scenario checksum mismatch: native=$native_checksum compact=$compact_checksum" >&2
            return 1
        }
    done
    printf '%s\n' "$RUN_ID" > "$OUT/latest-profile-run"
    printf 'Perf artifacts for run %s are under %s\n' "$RUN_ID" "$RUN_DIR"
}

counters() {
    need_binary "$PLAIN_BIN" "plain benchmark_profile"
    mkdir -p "$RUN_DIR/counters" "$RUN_DIR/counter-probes"
    : > "$RUN_DIR/counter-status.tsv"
    local -a perf_cmd=(perf)
    if [[ ${PROFILE_USE_SUDO:-0} == 1 ]]; then
        if [[ -n ${CSL_B10_WORKERS:-} ]]; then
            perf_cmd=(sudo -n --preserve-env=CSL_B10_WORKERS perf)
        else
            perf_cmd=(sudo -n perf)
        fi
    fi
    local -a events=(cycles instructions branches branch-misses cache-misses)
    local -a supported_events=()
    for event in "${events[@]}"; do
        local probe="$RUN_DIR/counter-probes/${event}.txt"
        local probe_error="$RUN_DIR/counter-probes/${event}.stderr"
        if "${perf_cmd[@]}" stat --no-big-num -x, -e "$event" -o "$probe" \
            -- true >/dev/null 2>"$probe_error" \
            && ! grep -Eiq 'not supported|not counted|permission denied|error:' \
                "$probe" "$probe_error"; then
            supported_events+=("$event")
            printf '%s\tavailable\n' "$event" >> "$RUN_DIR/counter-status.tsv"
        else
            printf '%s\tunavailable\n' "$event" >> "$RUN_DIR/counter-status.tsv"
        fi
    done
    if (( ${#supported_events[@]} == 0 )); then
        echo "No requested hardware counters are available; see $RUN_DIR/counter-status.tsv"
        return 0
    fi
    local event_csv
    event_csv=$(IFS=,; printf '%s' "${supported_events[*]}")
    local repeats=${COUNTER_REPEAT:-5}
    [[ "$repeats" =~ ^[1-9][0-9]*$ ]] || {
        echo "COUNTER_REPEAT must be a positive integer" >&2
        return 2
    }
    local -a selected_scenarios=("${SCENARIOS[@]}")
    if [[ -n ${PROFILE_SCENARIOS:-} ]]; then
        IFS=',' read -r -a selected_scenarios <<< "$PROFILE_SCENARIOS"
    fi
    for scenario in "${selected_scenarios[@]}"; do
        local repetitions
        repetitions=$(profile_repetitions "$scenario")
        local native_checksum= compact_checksum=
        for variant in native compact; do
            local prefix="$RUN_DIR/counters/${scenario}-${variant}"
            "${perf_cmd[@]}" stat --no-big-num -x, --repeat "$repeats" \
                -e "$event_csv" -o "${prefix}.perf-stat.csv" -- \
                "$PLAIN_BIN" --window "$scenario" "$variant" "$repetitions" \
                --seconds 0 > "${prefix}.workload.log" 2>&1
            local -a checksums=()
            mapfile -t checksums < <(
                awk -F '\t' '$1 == "META" && $2 == "checksum" {print $5}' \
                    "${prefix}.workload.log" | sort -u
            )
            if (( ${#checksums[@]} != 1 )); then
                echo "expected one stable checksum in ${prefix}.workload.log; found ${#checksums[@]}" >&2
                return 1
            fi
            if [[ $variant == native ]]; then
                native_checksum=${checksums[0]}
            else
                compact_checksum=${checksums[0]}
            fi
        done
        [[ "$native_checksum" == "$compact_checksum" ]] || {
            echo "$scenario checksum mismatch: native=$native_checksum compact=$compact_checksum" >&2
            return 1
        }
    done
    printf 'Counter captures for %s scenarios are under %s/counters\n' \
        "${#selected_scenarios[@]}" "$RUN_DIR"
}

reports() {
    mkdir -p "$RUN_DIR/reports"
    for data in "$RUN_DIR"/perf-data/*.data; do
        [[ -e "$data" ]] || { echo "no perf data under $RUN_DIR/perf-data" >&2; return 2; }
        local base=${data##*/}
        base=${base%.data}
        if ! awk '/Captured and wrote/ && /[1-9][0-9]* samples/ { found = 1 } END { exit !found }' \
            "$RUN_DIR/profile-logs/${base}.log"; then
            echo "skip ${base}: perf recorded no samples"
            continue
        fi
        perf report --stdio -i "$data" --percent-limit 0.5 \
            > "$RUN_DIR/reports/${base}-inclusive-and-self.txt"
        perf report --stdio -i "$data" --no-children --percent-limit 0.5 \
            > "$RUN_DIR/reports/${base}-self-only.txt"
    done
}

load_artifacts
case "${1:-}" in
    metadata) metadata ;;
    build) build ;;
    baseline) baseline ;;
    suite) suite ;;
    noisy) noisy ;;
    stats) stats ;;
    profile) profile ;;
    counters) counters ;;
    summary) summarize ;;
    reports) reports ;;
    *) usage >&2; exit 2 ;;
esac
