#!/usr/bin/env python3
"""Verify 002 checkbox registry matches source files and 003 mappings."""

import json
import re
import subprocess
import sys
from pathlib import Path

REGISTRY_PATH = Path("target/002_checkbox_registry.json")
SOURCE_DIR = Path("dev-docs/002_vibe_coding_plan")


def ensure_registry():
    if not REGISTRY_PATH.exists():
        print("Registry missing; running scripts/generate_002_registry.py...", file=sys.stderr)
        result = subprocess.run(
            [sys.executable, "scripts/generate_002_registry.py"], capture_output=True, text=True
        )
        if result.returncode != 0:
            print(result.stderr, file=sys.stderr)
            return False
    return True


def parse_source_checkboxes():
    counts = {}
    for md in sorted(SOURCE_DIR.glob("*.md")):
        chapter = "README" if md.name == "README.md" else md.stem.split("_")[0]
        text = md.read_text(encoding="utf-8")
        boxes = []
        checked = 0
        for line in text.splitlines():
            m = re.match(r"^\s*- \[(.)\]\s+", line)
            if not m:
                continue
            boxes.append(line)
            if m.group(1) in "xX":
                checked += 1
        counts[chapter] = {
            "file": md.name,
            "count": len(boxes),
            "checked": checked,
            "unchecked": len(boxes) - checked,
            "checkboxes": boxes,
        }
    return counts


def main():
    if not ensure_registry():
        return 1

    registry = json.loads(REGISTRY_PATH.read_text(encoding="utf-8"))
    source = parse_source_checkboxes()

    errors = []
    reg_chapters = registry.get("chapters", {})
    total_registry = sum(c["checkbox_count"] for c in reg_chapters.values())
    total_source = sum(c["count"] for c in source.values())
    total_checked_source = sum(c["checked"] for c in source.values())
    total_unchecked_source = sum(c["unchecked"] for c in source.values())

    if total_registry != total_source:
        errors.append(f"registry total {total_registry} != source total {total_source}")

    if registry.get("total_checked") != total_checked_source:
        errors.append(
            f"registry total_checked {registry.get('total_checked')} != source {total_checked_source}"
        )

    if registry.get("total_unchecked") != total_unchecked_source:
        errors.append(
            f"registry total_unchecked {registry.get('total_unchecked')} != source {total_unchecked_source}"
        )

    if registry.get("total_checkboxes") != registry.get("total_checked", 0) + registry.get(
        "total_unchecked", 0
    ):
        errors.append(
            "registry total_checkboxes != total_checked + total_unchecked"
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
        if reg["checked_count"] != data["checked"]:
            errors.append(
                f"chapter {chapter} registry checked_count {reg['checked_count']} != source {data['checked']}"
            )
        if reg["unchecked_count"] != data["unchecked"]:
            errors.append(
                f"chapter {chapter} registry unchecked_count {reg['unchecked_count']} != source {data['unchecked']}"
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
