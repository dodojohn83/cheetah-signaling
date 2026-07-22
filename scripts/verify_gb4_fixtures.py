#!/usr/bin/env python3
"""Validate GB28181 fixture provenance metadata.

Checks that every fixture data file under `testdata/gb28181/**` has a companion
`<name>.meta.toml` and that the metadata contains the required fields defined in
`dev-docs/004_gb28181-improve/90_reference_provenance_and_license.md`.

Exit non-zero on any validation failure.
"""

import sys
from pathlib import Path

import toml


REQUIRED_FIELDS = {
    "source",
    "standard",
    "profile",
    "expected",
    "desensitization",
    "license",
}

OPTIONAL_FIELDS = {
    "source_project",
    "source_commit",
    "manufacturer",
    "model",
    "firmware",
}

ALLOWED_SOURCES = {"synthetic", "real-device", "reference-peer"}
ALLOWED_STANDARDS = {"GB/T 28181-2022", "GB/T 28181-2016"}


def validate_fixture(meta_path: Path) -> list[str]:
    errors: list[str] = []
    try:
        data = toml.loads(meta_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"{meta_path}: failed to parse TOML: {exc}"]

    for field in REQUIRED_FIELDS:
        if field not in data:
            errors.append(f"{meta_path}: missing required field '{field}'")

    if "source" in data and data["source"] not in ALLOWED_SOURCES:
        errors.append(
            f"{meta_path}: source must be one of {ALLOWED_SOURCES}, got {data['source']!r}"
        )

    if "standard" in data and data["standard"] not in ALLOWED_STANDARDS:
        errors.append(
            f"{meta_path}: standard must be one of {ALLOWED_STANDARDS}, got {data['standard']!r}"
        )

    unknown = set(data.keys()) - REQUIRED_FIELDS - OPTIONAL_FIELDS
    if unknown:
        errors.append(f"{meta_path}: unknown fields {sorted(unknown)}")

    return errors


def fixture_name(path: Path) -> str:
    """Return the fixture base name for a data or metadata file.

    `name.meta.toml` -> `name`; `name.txt` / `name.xml` / `name.expected` -> `name`.
    """
    if path.name.endswith(".meta.toml"):
        return path.name[: -len(".meta.toml")]
    if len(path.suffixes) >= 2 and path.suffixes[-2] == ".meta":
        return path.stem[: -len(".meta")]
    return path.stem


def main() -> int:
    repo = Path(__file__).resolve().parent.parent
    fixtures_dir = repo / "testdata" / "gb28181"

    if not fixtures_dir.exists():
        print(f"ERROR: fixtures dir not found: {fixtures_dir}", file=sys.stderr)
        return 1

    all_paths = list(fixtures_dir.rglob("*"))
    data_files = [
        p
        for p in all_paths
        if p.is_file()
        and not p.name.endswith(".meta.toml")
        and p.name != "README.md"
    ]
    meta_files = [p for p in all_paths if p.is_file() and p.name.endswith(".meta.toml")]

    all_errors: list[str] = []

    if not meta_files:
        all_errors.append(
            f"{fixtures_dir}: no .meta.toml files found; provenance metadata is required"
        )

    # Group data files by directory and fixture base name so the reverse check is
    # extension-agnostic and handles dotted names correctly.
    data_name_set: set[tuple[Path, str]] = {
        (p.parent, fixture_name(p)) for p in data_files
    }

    for data_path in data_files:
        name = fixture_name(data_path)
        meta_path = data_path.with_name(f"{name}.meta.toml")
        if not meta_path.exists():
            all_errors.append(
                f"{data_path}: missing required provenance metadata {meta_path.relative_to(repo)}"
            )

    for meta_path in meta_files:
        name = fixture_name(meta_path)
        if (meta_path.parent, name) not in data_name_set:
            all_errors.append(
                f"{meta_path}: no matching data file found for fixture '{name}'"
            )
        all_errors.extend(validate_fixture(meta_path))

    if all_errors:
        for error in all_errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(
        f"OK: {len(data_files)} fixture data files and {len(meta_files)} metadata files validated."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
