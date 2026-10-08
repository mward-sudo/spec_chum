#!/usr/bin/env python3
"""Check Spectrum Next inventory coverage and optionally verify pinned source files."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "docs" / "next"


def read_csv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as source:
        return list(csv.DictReader(source))


def parse_register_ids(path: Path) -> set[str]:
    ids: set[str] = set()
    for left in re.findall(r"^(?!//)([^\n]+)=>\s*", path.read_text(encoding="utf-8"), re.M):
        if re.match(r"0x[0-9a-fA-F]{2}", left):
            ids.update(f"{int(value, 16):02X}" for value in re.findall(r"0x([0-9a-fA-F]{2})", left))
    return ids


def parse_port_entries(path: Path) -> list[tuple[str, str, str, str]]:
    entries = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.match(r"^\|([* ])\|([* ])\|\|([^|]+)\|\s*(0x[0-9a-fA-F]+)?\s*\|([^|]+)\|", line)
        if match:
            read, write, pattern, address, description = match.groups()
            if not description.strip():
                continue
            access = ("R" if read == "*" else "") + ("W" if write == "*" else "")
            entries.append((pattern.strip(), (address or "").lower(), re.sub(r"\s+", " ", description.strip()), access))
    return entries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, help="directory containing nextreg.txt and ports.txt from the pinned source commit")
    args = parser.parse_args()

    manifest = json.loads((DATA / "manifest.json").read_text(encoding="utf-8"))
    registers = read_csv(DATA / "nextreg.csv")
    ports = read_csv(DATA / "ports.csv")
    errors: list[str] = []

    expected_regs = set(manifest["nextreg_ids"])
    actual_regs = [row["id"].upper() for row in registers]
    if len(actual_regs) != len(set(actual_regs)):
        errors.append("NextReg inventory contains duplicate IDs")
    if set(actual_regs) != expected_regs:
        errors.append(f"NextReg coverage differs: missing={sorted(expected_regs - set(actual_regs))}, extra={sorted(set(actual_regs) - expected_regs)}")
    for index, row in enumerate(registers, start=2):
        for field in ("title", "access", "status", "implementation", "documented_bits", "behavior_limits", "tests", "follow_up"):
            if not row.get(field, "").strip():
                errors.append(f"nextreg.csv:{index}: {field} is empty")
        if row.get("status") not in {"implemented", "partial", "intentionally_unsupported", "unimplemented", "reserved"}:
            errors.append(f"nextreg.csv:{index}: unknown status {row.get('status')!r}")

    expected_ports = manifest["port_entries"]
    actual_ports = [
        {"pattern": row["pattern"], "address": row["address"].lower(), "description": row["description"], "access": row["access"]}
        for row in ports
    ]
    if actual_ports != expected_ports:
        errors.append("ports.csv entries or order differ from the extracted source manifest (duplicate aliases are significant)")
    for index, row in enumerate(ports, start=2):
        for field in ("pattern", "address", "description", "access", "status", "implementation", "behavior_limits", "tests", "follow_up"):
            if not row.get(field, "").strip():
                if field == "address" and "floating bus" in row.get("description", "").lower():
                    continue
                errors.append(f"ports.csv:{index}: {field} is empty")
        if row.get("status") not in {"implemented", "partial", "intentionally_unsupported", "unimplemented", "reserved"}:
            errors.append(f"ports.csv:{index}: unknown status {row.get('status')!r}")

    if args.source_dir:
        for key, name in (("nextreg", "nextreg.txt"), ("ports", "ports.txt")):
            path = args.source_dir / name
            if not path.is_file():
                errors.append(f"missing pinned source file: {path}")
                continue
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            if digest != manifest["sources"][key]["sha256"]:
                errors.append(f"{name}: SHA256 {digest} does not match pinned {manifest['sources'][key]['sha256']}")
        reg_source = args.source_dir / "nextreg.txt"
        port_source = args.source_dir / "ports.txt"
        if reg_source.is_file() and parse_register_ids(reg_source) != expected_regs:
            errors.append("pinned NextReg source IDs differ from manifest")
        if port_source.is_file() and [
            {"pattern": pattern, "address": address, "description": description, "access": access}
            for pattern, address, description, access in parse_port_entries(port_source)
        ] != expected_ports:
            errors.append("pinned port source entries differ from manifest")

    if errors:
        print("Next hardware inventory check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"Next hardware inventory OK: {len(registers)} NextRegs, {len(ports)} port entries; source commit {manifest['sources']['commit']}")
    if not args.source_dir:
        print("Pinned source hashes not rechecked (pass --source-dir to verify them).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
