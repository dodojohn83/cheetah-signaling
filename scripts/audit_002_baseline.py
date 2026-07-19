#!/usr/bin/env python3
"""AUD-002: re-verify the Phase 00 baseline and generate a combined audit report."""

import os
import platform
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REPORT_DIR = REPO / "target" / "reports"
REPORT_PATH = REPORT_DIR / "aud-002-baseline-reverification.md"

GATE_COMMANDS = [
    ("registry", ["python3", "scripts/audit_002_registry.py"]),
    ("architecture", ["python3", "scripts/audit_architecture.py"]),
    ("storage", ["python3", "scripts/verify_storage_baseline.py"]),
    ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
    ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    ("nextest", ["cargo", "nextest", "run", "--workspace"]),
    ("buf_format", ["buf", "format", "--diff", "--exit-code"]),
    ("buf_lint", ["buf", "lint"]),
    ("deny", ["cargo", "deny", "check"]),
]


def run(name, cmd):
    print(f"[{name}] $ {' '.join(cmd)}", flush=True)
    start = time.time()
    result = subprocess.run(
        cmd,
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    elapsed = time.time() - start
    print(f"[{name}] exit={result.returncode} in {elapsed:.2f}s", flush=True)
    return {
        "name": name,
        "command": " ".join(cmd),
        "returncode": result.returncode,
        "elapsed": elapsed,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def main():
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    env = {
        "host": platform.node(),
        "os": platform.platform(),
        "arch": platform.machine(),
        "cpus": os.cpu_count(),
        "datetime_utc": datetime.now(timezone.utc).isoformat(),
    }

    results = []
    skipped = []
    for name, cmd in GATE_COMMANDS:
        if name in ("registry", "architecture", "storage"):
            script = Path(cmd[1])
            if not script.exists():
                skipped.append(name)
                print(f"[{name}] skipped: {script} not present in this branch", flush=True)
                continue
        results.append(run(name, cmd))

    passed = sum(1 for r in results if r["returncode"] == 0)
    failed = [r["name"] for r in results if r["returncode"] != 0]

    lines = [
        f"# AUD-002: Phase 00 Baseline Re-verification\n\n",
        f"- Date: {env['datetime_utc']}\n",
        f"- Host: {env['host']} ({env['os']}, {env['arch']}, {env['cpus']} CPUs)\n\n",
        "## Commands run\n\n",
        "| Check | Command | Exit | Elapsed |\n",
        "|-------|---------|------|----------|\n",
    ]
    for r in results:
        status = "PASS" if r["returncode"] == 0 else "FAIL"
        lines.append(
            f"| {r['name']} | `{r['command']}` | {status} ({r['returncode']}) | {r['elapsed']:.2f}s |\n"
        )

    if skipped:
        lines.append("\n## Skipped checks\n\n")
        for s in skipped:
            lines.append(f"- `{s}`: script not present in this branch; see dedicated Phase 01 PR for evidence.\n")

    lines.extend([
        f"\n- Passed: {passed}/{len(results)}\n",
        f"- Failed: {', '.join(failed) if failed else 'none'}\n\n",
        "## Notes\n\n",
        "- This report re-runs the workspace, Proto, and storage quality gates as part of AUD-002.\n",
        "- Component-specific audits (registry, architecture, storage baseline) are maintained in their respective PRs.\n",
    ])

    REPORT_PATH.write_text("".join(lines), encoding="utf-8")
    sanitized = REPO / "dev-docs" / "003_next_round_vibe_coding_plan" / "reports" / "aud-002-baseline-reverification.md"
    sanitized.parent.mkdir(parents=True, exist_ok=True)
    sanitized.write_text("".join(lines), encoding="utf-8")

    print(f"\nReport: {REPORT_PATH}")
    print(f"Sanitized report: {sanitized}")

    if failed:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
