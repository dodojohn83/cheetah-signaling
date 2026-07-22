#!/usr/bin/env python3
"""Verify that protobuf Rust codegen is reproducible from a clean target.

This script implements the BAS-002 requirement that codegen from an empty target
is executable and produces byte-identical artifacts across two runs.

It builds `cheetah-signal-contracts` twice in separate `CARGO_TARGET_DIR`
directories and compares the generated `.rs` files in the `OUT_DIR` of each run.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def run(cmd: list[str], cwd: Path, extra_env: dict | None = None) -> int:
    merged = {**dict(os.environ), **(extra_env or {})}
    print(f"$ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=cwd, env=merged).returncode


def find_out_dir(target_dir: Path) -> Path | None:
    build_dir = target_dir / "debug" / "build"
    if not build_dir.exists():
        return None
    for entry in build_dir.iterdir():
        if entry.is_dir() and entry.name.startswith("cheetah-signal-contracts-"):
            out = entry / "out"
            if out.exists():
                return out
    return None


def collect_files(out_dir: Path) -> dict[Path, bytes]:
    files: dict[Path, bytes] = {}
    for path in out_dir.rglob("*"):
        if path.is_file():
            rel = path.relative_to(out_dir)
            files[rel] = path.read_bytes()
    return files


def main() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        target_a = tmp_path / "a"
        target_b = tmp_path / "b"

        for label, target_dir in [("run-1", target_a), ("run-2", target_b)]:
            print(f"\n=== {label} ===", flush=True)
            rc = run(
                ["cargo", "check", "-p", "cheetah-signal-contracts"],
                REPO,
                {"CARGO_TARGET_DIR": str(target_dir)},
            )
            if rc != 0:
                print(f"{label} failed with exit code {rc}", file=sys.stderr)
                return rc

        out_a = find_out_dir(target_a)
        out_b = find_out_dir(target_b)

        if out_a is None or out_b is None:
            print("Could not locate generated OUT_DIR in one of the runs", file=sys.stderr)
            return 1

        files_a = collect_files(out_a)
        files_b = collect_files(out_b)

        if set(files_a.keys()) != set(files_b.keys()):
            only_a = set(files_a.keys()) - set(files_b.keys())
            only_b = set(files_b.keys()) - set(files_a.keys())
            print("Generated file sets differ:", file=sys.stderr)
            for p in sorted(only_a):
                print(f"  only in run-1: {p}", file=sys.stderr)
            for p in sorted(only_b):
                print(f"  only in run-2: {p}", file=sys.stderr)
            return 1

        mismatches: list[Path] = []
        for rel in sorted(files_a.keys()):
            if files_a[rel] != files_b[rel]:
                mismatches.append(rel)

        if mismatches:
            print("Generated file contents differ:", file=sys.stderr)
            for rel in mismatches:
                print(f"  {rel}", file=sys.stderr)
            return 1

        print(f"\nOK: {len(files_a)} generated file(s) are byte-identical across two clean builds.")
        print(f"OUT_DIR run-1: {out_a}")
        print(f"OUT_DIR run-2: {out_b}")
        return 0


if __name__ == "__main__":
    sys.exit(main())
