#!/usr/bin/env python3
"""Summarize saved bpftrace and telemetry artifacts; never runs a workload."""

from __future__ import annotations

import argparse
import collections
import json
import re
from pathlib import Path


def normalize_frame(frame: str) -> str:
    frame = re.sub(r"\+\d+$", "", frame)
    return re.sub(r"::h[0-9a-fA-F]+$", "", frame)


def read_stacks(path: Path) -> dict[str, object]:
    leaves: collections.Counter[str] = collections.Counter()
    stacks: set[tuple[str, ...]] = set()
    total = 0
    unresolved = 0
    frames: list[str] | None = None

    for line in path.read_text().splitlines():
        if line.startswith("@["):
            frames = []
        elif frames is not None and line.startswith("]: "):
            count = int(line[3:])
            total += count
            stacks.add(tuple(frames))
            if frames:
                leaf = frames[0]
                if leaf.startswith("0x"):
                    unresolved += count
                    leaves["<raw address>"] += count
                else:
                    leaves[normalize_frame(leaf)] += count
            frames = None
        elif frames is not None and line.strip():
            frames.append(line.strip())

    return {
        "weighted_samples": total,
        "distinct_stacks": len(stacks),
        "distinct_leaf_functions": len(leaves),
        "unresolved_leaf_samples": unresolved,
        "resolved_leaf_samples": total - unresolved,
        "top_leaf_functions": leaves.most_common(20),
    }


def read_uprobes(path: Path) -> dict[str, dict[str, int]]:
    """Merge each 500 ms print/clear interval, ignoring per-thread start maps."""
    maps: dict[str, collections.Counter[str]] = collections.defaultdict(
        collections.Counter
    )
    current_histogram: str | None = None
    wanted = {
        "lock_calls",
        "lock_latency_us",
        "critical_section_us",
        "allocation_payload_bytes",
        "allocation_alignment",
        "release_batch_size",
        "release_locked_us",
        "merge_us",
    }

    for line in path.read_text().splitlines():
        scalar = re.match(r"^@(\w+):\s+(\d+)\s*$", line)
        if scalar:
            name, value = scalar.groups()
            if name in wanted:
                maps[name]["scalar"] += int(value)
            current_histogram = None
            continue

        header = re.match(r"^@(\w+):\s*$", line)
        if header:
            name = header.group(1)
            current_histogram = name if name in wanted else None
            continue

        keyed = re.match(r"^@(\w+)\[([^]]+)\]:\s*(\d+)\s*$", line)
        if keyed:
            name, key, value = keyed.groups()
            if name in wanted:
                maps[name][key] += int(value)
            current_histogram = None
            continue

        if current_histogram is not None:
            bucket = re.match(r"^\s*(\[[^|]*?(?:\]|\)))\s+(\d+)\s+\|", line)
            if bucket:
                key, value = bucket.groups()
                maps[current_histogram][key] += int(value)
            else:
                current_histogram = None

    return {name: dict(values) for name, values in sorted(maps.items())}


def read_telemetry(path: Path) -> dict[str, object]:
    selected_meta: dict[str, list[list[str]]] = collections.defaultdict(list)
    phases: list[dict[str, object]] = []

    for line in path.read_text().splitlines():
        fields = line.split("\t")
        if fields[0] == "META" and len(fields) >= 5:
            key = fields[1]
            if key in {
                "peak_rss_kb",
                "allocator_summary",
                "allocator_phase_profile",
                "block_size_bucket",
            }:
                selected_meta[key].append(fields[2:])
        elif fields[0] == "PHASE" and len(fields) >= 8:
            phases.append(
                {
                    "scenario": fields[1],
                    "variant": fields[2],
                    "phase": fields[3],
                    "runs": int(fields[4]),
                    "median_ns": int(fields[5]),
                    "p95_ns": int(fields[6]),
                }
            )

    return {"selected_meta": dict(selected_meta), "phases": phases}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path)
    args = parser.parse_args()

    result: dict[str, object] = {"artifact_dir": str(args.artifact_dir)}
    for scenario in ("A2", "B8", "B10"):
        result[scenario] = {
            "user_stacks": read_stacks(args.artifact_dir / f"{scenario}.user-stacks.txt"),
            "uprobes": read_uprobes(
                args.artifact_dir / f"{scenario}.allocator-uprobes.txt"
            ),
            "telemetry": read_telemetry(
                args.artifact_dir / f"{scenario}.telemetry.log"
            ),
        }

    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
