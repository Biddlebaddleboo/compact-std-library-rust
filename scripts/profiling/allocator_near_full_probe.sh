#!/usr/bin/env bash
set -euo pipefail

if [[ "${ALLOCATOR_PROFILE_WINDOW:-}" != "coordinated" ]]; then
    echo "Refusing to build or run: root must coordinate the shared-host window first." >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
out_file=${ALLOCATOR_NEAR_FULL_OUTPUT:-/tmp/compact-allocator-profile-v2.4/near-full-probe.log}
mkdir -p "$(dirname "$out_file")"

{
    date -u '+captured_at_utc=%Y-%m-%dT%H:%M:%SZ'
    printf 'source_commit=%s\n' "$(git rev-parse HEAD)"
    printf 'kernel=%s\n' "$(uname -a)"
    printf 'rustc='; rustc -V
    printf 'cargo='; cargo -V
    printf 'command=cargo run --release --locked -p compact_std --example allocator_near_full_probe --no-default-features\n'
} > "$out_file"

cd "$repo_root"
cargo run --release --locked -p compact_std \
    --example allocator_near_full_probe --no-default-features >> "$out_file" 2>&1

cat "$out_file"
