#!/usr/bin/env python3
"""Verify 002 checkbox registry matches source files and 003 mappings."""

import json
import re
import sys
from pathlib import Path

REGISTRY_PATH = Path("target/002_checkbox_registry.json")
SOURCE_DIR = Path("dev-docs/002_vibe_coding_plan")


def parse_source_checkboxes():
    counts = {}
    for md in sorted(SOURCE_DIR.glob("*.md")):
        if md.name == "README.md":
            chapter = "README"
        else:
            chapter = md.stem.split("_")[0]
        text = md.read_text(encoding="utf-8")
        boxes = []
        for line in text.splitlines():
            if re.match(r"^\s*- \[.\]\s+", line):
                boxes.append(line)
        counts[chapter] = {
            "file": md.name,
            "count": len(boxes),
            "checkboxes": boxes,
        }
    return counts


def main():
    if not REGISTRY_PATH.exists():
        print(f"Missing registry: {REGISTRY_PATH}", file=sys.stderr)
        print("Run the generator first.", file=sys.stderr)
        return 1

    registry = json.loads(REGISTRY_PATH.read_text(encoding="utf-8"))
    source = parse_source_checkboxes()

    errors = []
    reg_chapters = registry.get("chapters", {})
    total_registry = sum(c["checkbox_count"] for c in reg_chapters.values())
    total_source = sum(c["count"] for c in source.values())

    if total_registry != total_source:
        errors.append(
            f"registry total {total_registry} != source total {total_source}"
        )

    for chapter, data in source.items():
        reg = reg_chapters.get(chapter)
        if reg is None:
            errors.append(f"chapter {chapter} missing from registry")
            continue
        if reg["checkbox_count"] != data["count"]:
            errors.append(
                f"chapter {chapter} registry {reg['checkbox_count']} != source {data['count']}"
            )
        seen = set()
        for box in reg["checkboxes"]:
            cb_id = box["id"]
            if cb_id in seen:
                errors.append(f"duplicate id {cb_id}")
            seen.add(cb_id)
            expected = f"002-{chapter}-"
            if not cb_id.startswith(expected):
                errors.append(f"id {cb_id} does not start with {expected}")

    if errors:
        print("AUDIT FAILED:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1

    print("AUDIT PASSED")
    print(f"  files: {len(source)}")
    print(f"  total checkboxes: {total_source}")
    print(f"  checked: {registry['total_checked']}")
    print(f"  unchecked: {registry['total_unchecked']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
