#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
OUT=${MEMORY_PROFILE_OUT:-/tmp/csl-v25-memory-profile}
BUILD_ROOT=${MEMORY_PROFILE_BUILD_ROOT:-$OUT}
RUNS=${MEMORY_PROFILE_RUNS:-5}
BIN=${MEMORY_PROFILE_BIN:-}

[[ "$RUNS" =~ ^[1-9][0-9]*$ ]] || {
    echo "MEMORY_PROFILE_RUNS must be a positive integer" >&2
    exit 2
}

mkdir -p "$OUT"
if [[ -z "$BIN" ]]; then
    mkdir -p "$BUILD_ROOT"
    env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
        CARGO_TARGET_DIR="$BUILD_ROOT/target" cargo build --locked --release \
        --message-format=json --no-default-features -p compact_std \
        --example memory_profile > "$OUT/build.jsonl"
    BIN=$(python3 - "$OUT/build.jsonl" <<'PY'
import json
import sys

for line in open(sys.argv[1]):
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    target = message.get("target", {})
    if (
        message.get("reason") == "compiler-artifact"
        and target.get("name") == "memory_profile"
        and "example" in target.get("kind", [])
    ):
        executable = message.get("executable")
        if executable:
            print(executable)
            break
else:
    raise SystemExit("cargo did not report a memory_profile executable")
PY
    )
fi

[[ -x "$BIN" ]] || {
    echo "memory probe executable is missing or not executable: $BIN" >&2
    exit 2
}

{
    printf 'Recorded UTC: '
    date -u +'%Y-%m-%dT%H:%M:%SZ'
    printf 'source='; git -C "$ROOT" rev-parse HEAD
    printf 'run_count=%s\n' "$RUNS"
    printf 'host='; uname -a
    printf 'toolchain='; rustc -V
    printf 'binary='; sha256sum "$BIN"
    printf 'features=--no-default-features\n'
    printf 'allocator-telemetry=disabled\n'
    printf 'probe=128 MiB cage, 512 KiB live payload, 200 ms quiescence\n'
} > "$OUT/host.txt"

mkdir -p "$OUT/runs"
for ((run = 1; run <= RUNS; run++)); do
    "$BIN" > "$OUT/runs/run$(printf '%02d' "$run").tsv"
done

cat "$OUT/host.txt"
printf 'Memory snapshots written to %s/runs\n' "$OUT"
