#!/usr/bin/env bash
set -euo pipefail

# Reproducible V2.4 child-process profiling. Run only after reserving a quiet
# measurement window on the host; `profile` executes timed workloads.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUT=${PROFILE_OUT:-/tmp/csl-v24-cpu-profile}
FEATURES=json,toml
PLAIN_TARGET="$OUT/target-plain"
PROFILE_TARGET="$OUT/target-profiled"
find_example() {
    find "$1/release/examples" -maxdepth 1 -type f \
        -name 'benchmark_compare-*' -perm -111 -print -quit 2>/dev/null || true
}

PLAIN_BIN=${PLAIN_BIN:-$(find_example "$PLAIN_TARGET")}
PROFILE_BIN=${PROFILE_BIN:-$(find_example "$PROFILE_TARGET")}
SCENARIOS=(A2 A4 A5 B6 B8 B3 B5 B10)

usage() {
    cat <<'EOF'
Usage: scripts/profile_cpu.sh {metadata|build|baseline|profile|reports}

Commands:
  metadata  Record host and toolchain details (read-only workload-wise).
  build     Build ordinary release and frame-pointer/debug-symbol binaries.
  baseline  Run two suites with ordinary and frame-pointer release binaries.
  profile   Sample each native/compact child directly with perf.
  reports   Export inclusive/self perf reports from saved samples.

The profile command requires PROFILE_USE_SUDO=1 on hosts where perf access is
restricted. Set PROFILE_OUT to store artifacts outside the default /tmp path.
EOF
}

profile_repetitions() {
    case "$1" in
        A2|A4) printf '%s\n' 5000 ;;
        A5) printf '%s\n' 3000 ;;
        B3) printf '%s\n' 250 ;;
        B5) printf '%s\n' 3000 ;;
        B6) printf '%s\n' 60000 ;;
        B8) printf '%s\n' 600 ;;
        B10) printf '%s\n' 1000 ;;
        *) printf 'no profile repetition count for %s\n' "$1" >&2; return 2 ;;
    esac
}

metadata() {
    mkdir -p "$OUT"
    {
        printf 'Recorded UTC: '
        date -u +'%Y-%m-%dT%H:%M:%SZ'
        printf '\n--- source ---\n'
        git -C "$ROOT" rev-parse HEAD
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
        printf 'profile flags: -C debuginfo=1 -C force-frame-pointers=yes\n'
        printf 'sample event: cpu-clock:u; frequency: %s Hz\n' "${PERF_FREQUENCY:-499}"
    } > "$OUT/host.txt"
    cat "$OUT/host.txt"
}

build() {
    mkdir -p "$OUT"
    if [[ ${BUILD_PLAIN:-1} == 1 ]]; then
        CARGO_TARGET_DIR="$PLAIN_TARGET" cargo build --locked --release \
            -p compact_std --example benchmark_compare --features "$FEATURES"
    fi
    RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C debuginfo=1 -C force-frame-pointers=yes" \
        CARGO_TARGET_DIR="$PROFILE_TARGET" cargo build --locked --release \
        -p compact_std --example benchmark_compare --features "$FEATURES"
}

baseline() {
    test -x "$PLAIN_BIN" || { echo "missing $PLAIN_BIN; run build first" >&2; return 2; }
    test -x "$PROFILE_BIN" || { echo "missing $PROFILE_BIN; run build first" >&2; return 2; }
    mkdir -p "$OUT/baseline"
    for run in 1 2; do
        for scenario in "${SCENARIOS[@]}"; do
            "$PLAIN_BIN" --scenario "$scenario" \
                --output "$OUT/baseline/run${run}-${scenario}.tsv" \
                > "$OUT/baseline/run${run}-${scenario}.log" 2>&1
            "$PROFILE_BIN" --scenario "$scenario" \
                --output "$OUT/baseline/frame-pointer-run${run}-${scenario}.tsv" \
                > "$OUT/baseline/frame-pointer-run${run}-${scenario}.log" 2>&1
        done
    done
}

profile() {
    test -x "$PROFILE_BIN" || { echo "missing $PROFILE_BIN; run build first" >&2; return 2; }
    mkdir -p "$OUT/perf-data" "$OUT/profile-logs"
    local -a perf_cmd=(perf)
    if [[ ${PROFILE_USE_SUDO:-0} == 1 ]]; then
        perf_cmd=(sudo -n perf)
    fi
    local frequency=${PERF_FREQUENCY:-499}
    local -a selected_scenarios=("${SCENARIOS[@]}")
    if [[ -n ${PROFILE_SCENARIOS:-} ]]; then
        IFS=',' read -r -a selected_scenarios <<< "$PROFILE_SCENARIOS"
    fi
    for scenario in "${selected_scenarios[@]}"; do
        local repetitions
        repetitions=$(profile_repetitions "$scenario")
        local native_checksum= compact_checksum=
        for variant in native compact; do
            local data="$OUT/perf-data/${scenario}-${variant}.data"
            local log="$OUT/profile-logs/${scenario}-${variant}.log"
            "${perf_cmd[@]}" record -e cpu-clock:u -F "$frequency" -g \
                --call-graph fp -o "$data" -- \
                "$PROFILE_BIN" --child "$scenario" "$variant" "$repetitions" \
                > "$log" 2>&1
            if [[ ${PROFILE_USE_SUDO:-0} == 1 ]]; then
                sudo -n chown "$(id -u):$(id -g)" "$data"
            fi
            local checksum
            checksum=$(awk -F '\t' '$1 == "META" && $2 == "checksum" {print $5}' "$log")
            [[ -n $checksum ]] || { echo "no checksum in $log" >&2; return 1; }
            if [[ $variant == native ]]; then
                native_checksum=$checksum
            else
                compact_checksum=$checksum
            fi
        done
        [[ $native_checksum == "$compact_checksum" ]] || {
            echo "$scenario checksum mismatch: native=$native_checksum compact=$compact_checksum" >&2
            return 1
        }
    done
}

reports() {
    mkdir -p "$OUT/reports"
    for data in "$OUT"/perf-data/*.data; do
        [[ -e $data ]] || { echo "no perf data under $OUT/perf-data" >&2; return 2; }
        local base=${data##*/}
        base=${base%.data}
        if ! grep -Eq 'Captured and wrote .*\([0-9]+ samples\)' \
            "$OUT/profile-logs/${base}.log"; then
            echo "skip ${base}: perf recorded no samples"
            continue
        fi
        # Default perf report shows both Children (inclusive) and Self
        # (exclusive). The second export records Self alone for clarity.
        perf report --stdio -i "$data" --percent-limit 0.5 \
            > "$OUT/reports/${base}-inclusive-and-self.txt"
        perf report --stdio -i "$data" --no-children --percent-limit 0.5 \
            > "$OUT/reports/${base}-self-only.txt"
    done
}

case "${1:-}" in
    metadata) metadata ;;
    build) build ;;
    baseline) baseline ;;
    profile) profile ;;
    reports) reports ;;
    *) usage >&2; exit 2 ;;
esac
