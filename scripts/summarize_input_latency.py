#!/usr/bin/env python3
"""Summarize SPEC_CHUM_INPUT_LATENCY probe output by mode and stage."""

from __future__ import annotations

import argparse
import math
import re
import statistics
from pathlib import Path


RECORD = re.compile(
    r"^\S+ session=(?P<session>\S+) sample=(?P<sample>\d+) mode=(?P<mode>\S+) "
    r"stage=(?P<stage>\S+)(?: elapsed_ms=(?P<elapsed>[\d.]+))?(?: .*)?$"
)
FINAL_STAGE = {"flat": "flat_draw", "living_room": "living_room_layer_refresh"}


def percentile95(values: list[float]) -> float:
    ordered = sorted(values)
    return ordered[max(math.ceil(0.95 * len(ordered)) - 1, 0)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "log",
        nargs="?",
        type=Path,
        default=Path("/tmp/spec-input-latency.log"),
        help="probe log path (default: /tmp/spec-input-latency.log)",
    )
    parser.add_argument("--minimum", type=int, default=10, help="minimum completed samples per mode")
    args = parser.parse_args()

    samples: dict[tuple[str, str], tuple[str, dict[str, float]]] = {}
    dropped: set[tuple[str, str]] = set()
    for line in args.log.read_text(encoding="utf-8").splitlines():
        if " dropped=overlap" in line:
            match = re.search(r"session=(\S+) sample=(\d+)", line)
            if match:
                dropped.add((match.group(1), match.group(2)))
            continue
        match = RECORD.match(line)
        if not match:
            continue
        sample = (match.group("session"), match.group("sample"))
        mode = match.group("mode")
        if sample not in samples:
            samples[sample] = (mode, {})
        elapsed = match.group("elapsed")
        if elapsed is not None:
            samples[sample][1][match.group("stage")] = float(elapsed)

    enough_samples = True
    sessions = {session for session, _ in samples}
    print(f"sessions={len(sessions)}")
    for mode, final_stage in FINAL_STAGE.items():
        complete_samples = {
            sample: values
            for sample, (sample_mode, values) in samples.items()
            if sample_mode == mode and sample not in dropped and final_stage in values
        }
        values = [stages[final_stage] for stages in complete_samples.values()]
        print(f"{mode}: completed={len(values)} minimum={args.minimum}")
        if len(values) < args.minimum:
            enough_samples = False
            print("  insufficient completed samples")
            continue
        print(f"  key_down_to_{final_stage}: median={statistics.median(values):.2f}ms p95={percentile95(values):.2f}ms")
        stage_names = set().union(*(stages.keys() for stages in complete_samples.values()))
        for stage in sorted(stage_names):
            stage_values = [sample_stages[stage] for sample_stages in complete_samples.values() if stage in sample_stages]
            print(
                f"  {stage}: n={len(stage_values)} "
                f"median={statistics.median(stage_values):.2f}ms "
                f"p95={percentile95(stage_values):.2f}ms"
            )

    if dropped:
        print(f"dropped_overlaps={len(dropped)}")
    return 0 if enough_samples else 1


if __name__ == "__main__":
    raise SystemExit(main())
