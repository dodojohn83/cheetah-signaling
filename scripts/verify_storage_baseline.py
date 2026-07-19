#!/usr/bin/env python3
"""Verify storage and migration baseline for SQLite and PostgreSQL adapters."""

import json
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REPORT_DIR = REPO / "target" / "reports"
REPORT_PATH = REPORT_DIR / "bas-005-storage-baseline.md"


def run(cmd, **kwargs):
    print(f"$ {' '.join(cmd)}", flush=True)
    start = time.time()
    result = subprocess.run(
        cmd,
        cwd=REPO,
        capture_output=True,
        text=True,
        **kwargs,
    )
    elapsed = time.time() - start
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    return result.returncode, elapsed, result.stdout, result.stderr


def list_migration_files():
    """Return list of .sql migration files under migrations/postgres and migrations/sqlite."""
    files = []
    for backend in ("postgres", "sqlite"):
        d = REPO / "migrations" / backend
        if d.exists():
            files.extend(sorted(d.glob("*.sql")))
    return files


def check_migrations_append_only(migrations):
    """Verify migration files follow the NNNN__name.sql convention."""
    errors = []
    for path in migrations:
        rel = path.relative_to(REPO)
        if not re.match(r"^\d{4}__[a-z0-9_]+\.sql$", path.name):
            errors.append(f"unexpected migration filename: {rel}")
    return errors


def main():
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    results = {
        "cargo_test_storage_tests": {"returncode": None, "elapsed": None, "stdout": ""},
        "cargo_test_storage_adapters": {"returncode": None, "elapsed": None, "stdout": ""},
        "migrations_append_only": [],
    }

    rc, elapsed, stdout, _ = run(
        ["cargo", "test", "-p", "cheetah-storage-tests", "--test", "sqlite", "--test", "postgres"]
    )
    results["cargo_test_storage_tests"] = {"returncode": rc, "elapsed": elapsed, "stdout": stdout}

    rc2, elapsed2, stdout2, _ = run(
        ["cargo", "test", "-p", "cheetah-storage-sqlite", "-p", "cheetah-storage-postgres"]
    )
    results["cargo_test_storage_adapters"] = {"returncode": rc2, "elapsed": elapsed2, "stdout": stdout2}

    migrations = list_migration_files()
    append_errors = check_migrations_append_only(migrations)
    results["migrations_append_only"] = append_errors
    results["migration_file_count"] = len(migrations)

    lines = [
        "# BAS-005: Storage and Migration Baseline\n\n",
        f"- SQLite/PostgreSQL contract suite return code: {results['cargo_test_storage_tests']['returncode']}\n",
        f"- SQLite/PostgreSQL contract suite elapsed: {results['cargo_test_storage_tests']['elapsed']:.2f}s\n",
        f"- Storage adapter tests return code: {results['cargo_test_storage_adapters']['returncode']}\n",
        f"- Storage adapter tests elapsed: {results['cargo_test_storage_adapters']['elapsed']:.2f}s\n",
        f"- Migration files scanned: {results['migration_file_count']}\n",
        f"- Migration append-only errors: {len(append_errors)}\n\n",
        "## Commands run\n\n",
        "```bash\n",
        "cargo test -p cheetah-storage-tests --test sqlite --test postgres\n",
        "cargo test -p cheetah-storage-sqlite -p cheetah-storage-postgres\n",
        "```\n\n",
        "## Contract suite submodules\n\n",
        "The shared contract suite in `crates/testing/cheetah-storage-tests/src/contract.rs` "
        "runs the same repository port tests against both the SQLite and PostgreSQL adapters. "
        "It covers: device, channel, operation, media, list, outbox, outbox retry, transaction, "
        "processed message, owner, ownership, node, webhook, step, and unicode.\n\n",
    ]

    if append_errors:
        lines.append("## Migration layout issues\n\n")
        for e in append_errors:
            lines.append(f"- {e}\n")

    lines.append("\n## Raw test output\n\n")
    lines.append("```text\n")
    lines.append(results["cargo_test_storage_tests"]["stdout"])
    lines.append(results["cargo_test_storage_adapters"]["stdout"])
    lines.append("```\n")

    REPORT_PATH.write_text("".join(lines), encoding="utf-8")
    print(f"\nReport written to {REPORT_PATH}")

    if results["cargo_test_storage_tests"]["returncode"] != 0:
        return 1
    if results["cargo_test_storage_adapters"]["returncode"] != 0:
        return 1
    if append_errors:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
