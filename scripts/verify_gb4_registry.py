#!/usr/bin/env python3
"""Verify GB4 task ID registry consistency.

Cross-checks that:
- All `GB4-*` IDs used as task checkboxes in dev-docs/004_gb28181-improve are unique.
- All checkbox-defined IDs are covered by the ranges/groups in
  91_003_requirement_registry.md.
- All relative markdown links in the docs point to existing files.
- Total unique checkbox-defined task count is 68.

Exit non-zero on mismatch.
"""

import re
import sys
from pathlib import Path

ID_RE = re.compile(r"GB4-[A-Z]{2,5}-[0-9]{3}")
PREFIX_RE = re.compile(r"GB4-[A-Z]{2,5}")
CHECKBOX_RE = re.compile(r"^\s*(?:-\s*\[\s*[xX ]\s*\]|\*\s*\[\s*[xX ]\s*\])")
RANGE_RE = re.compile(
    r"(GB4-[A-Z]{2,5})-([0-9]{3})\.\.([0-9]{3})"
)


def main() -> int:
    repo = Path(__file__).resolve().parent.parent
    docs_dir = repo / "dev-docs" / "004_gb28181-improve"
    registry = docs_dir / "91_003_requirement_registry.md"

    if not docs_dir.exists():
        print(f"ERROR: docs dir not found: {docs_dir}", file=sys.stderr)
        return 1

    checkbox_ids: set[str] = set()
    files = list(docs_dir.rglob("*.md"))
    for path in files:
        text = path.read_text(encoding="utf-8")
        for line in text.splitlines():
            if CHECKBOX_RE.match(line):
                for task_id in ID_RE.findall(line):
                    if task_id in checkbox_ids:
                        print(
                            f"ERROR: task ID {task_id} appears in multiple checkbox definitions",
                            file=sys.stderr,
                        )
                        return 1
                    checkbox_ids.add(task_id)

    if not registry.exists():
        print(f"ERROR: registry not found: {registry}", file=sys.stderr)
        return 1

    registry_text = registry.read_text(encoding="utf-8")

    # Build the explicit set of IDs declared in the registry by expanding ranges.
    registry_ids: set[str] = set(ID_RE.findall(registry_text))
    # Expand `GB4-XXX-001..005` patterns.
    for match in RANGE_RE.finditer(registry_text):
        prefix = match.group(1)
        start = int(match.group(2))
        end = int(match.group(3))
        for i in range(start, end + 1):
            registry_ids.add(f"{prefix}-{i:03d}")

    # Bare group references like `GB4-REF` or `GB4-TST` cover all IDs with that prefix
    # that are also defined as checkboxes in the phase docs.
    prefix_references = set(PREFIX_RE.findall(registry_text))
    for task_id in checkbox_ids:
        prefix = PREFIX_RE.match(task_id)
        if prefix and prefix.group(0) in prefix_references:
            registry_ids.add(task_id)

    only_in_checkbox = sorted(checkbox_ids - registry_ids)
    only_in_registry = sorted(registry_ids - checkbox_ids)

    if only_in_checkbox:
        print(
            f"ERROR: checkbox IDs not covered by registry: {only_in_checkbox}",
            file=sys.stderr,
        )
        return 1
    if only_in_registry:
        print(
            f"ERROR: registry IDs not defined as checkboxes: {only_in_registry}",
            file=sys.stderr,
        )
        return 1

    expected_count = 68
    if len(checkbox_ids) != expected_count:
        print(
            f"ERROR: expected {expected_count} unique checkbox-defined GB4 IDs, found {len(checkbox_ids)}",
            file=sys.stderr,
        )
        return 1

    # Verify relative markdown links in the docs directory point to existing files.
    link_re = re.compile(r"\]\(([^)]+\.md)\)")
    for path in files:
        text = path.read_text(encoding="utf-8")
        for match in link_re.finditer(text):
            link = match.group(1)
            link_path = link.split("#", 1)[0]
            if link_path.startswith(("http://", "https://", "/")):
                continue
            target = (path.parent / link_path).resolve()
            if not target.exists():
                print(f"ERROR: broken link in {path}: {link}", file=sys.stderr)
                return 1

    print(
        f"OK: {len(checkbox_ids)} unique GB4 task IDs, registry cross-check passed, links valid."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
