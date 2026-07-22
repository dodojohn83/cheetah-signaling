#!/usr/bin/env python3
"""Generate a reproducible baseline report for the workspace quality gates.

This script implements `BAS-006` of `dev-docs/003_next_round_vibe_coding_plan`.
It runs the commands listed in `BAS-003`, captures raw output under
`target/reports/baseline/<commit>/`, and writes a de-sensitized Markdown
summary that can be committed to the docs directory.

Raw output is intentionally *not* committed. The summary records:
- toolchain, OS, arch, CPU, memory
- each command, duration, exit code and a pass/fail/skip summary
- unrun items and the reason (missing tool, skipped target, etc.)
- warnings / ignored tests / active feature list
- failing task IDs if the failure can be mapped to a known issue.
"""

import datetime
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Optional

REPO = Path(__file__).resolve().parent.parent
REPORT_DIR = REPO / "target" / "reports" / "baseline"


def _run(cmd: list[str], timeout: Optional[int] = None) -> tuple[int, str, float]:
    start = time.monotonic()
    proc = subprocess.run(
        cmd,
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=timeout,
    )
    duration = time.monotonic() - start
    return proc.returncode, proc.stdout, duration


def _git_commit() -> str:
    rc, out, _ = _run(["git", "rev-parse", "HEAD"])
    return out.strip() if rc == 0 else "unknown"


def _tool_version(name: str, cmd: list[str]) -> str:
    try:
        rc, out, _ = _run(cmd, timeout=30)
        first = (out or "").strip().splitlines()[0]
        return first if rc == 0 else f"{name} not available"
    except FileNotFoundError:
        return f"{name} not found"


def _cpu_info() -> str:
    if shutil.which("lscpu"):
        rc, out, _ = _run(["lscpu"], timeout=10)
        if rc == 0:
            for line in out.splitlines():
                if line.startswith("Model name:"):
                    return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def _memory_gb() -> str:
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    kb = int(line.split()[1])
                    return f"{kb / 1024 / 1024:.1f} GiB"
    except (OSError, ValueError):
        pass
    return "unknown"


def _cargo_test_summary(stdout: str) -> tuple[int, int, int, list[str]]:
    passed = failed = ignored = 0
    failure_lines: list[str] = []
    for line in stdout.splitlines():
        m = re.search(
            r"test result: (?:ok|FAILED).*?(\d+) passed; (\d+) failed; (\d+) ignored",
            line,
        )
        if m:
            passed += int(m.group(1))
            failed += int(m.group(2))
            ignored += int(m.group(3))
        if "FAILED" in line or "error[" in line:
            failure_lines.append(line.strip())
    return passed, failed, ignored, failure_lines


def _known_failure_mapping(line: str) -> Optional[str]:
    if "missing field `compatibility`" in line and "MediaConfig" in line:
        return "GB4-COMP-003/004: MediaConfig compatibility field drift; fix in PR #210"
    return None


def main() -> int:
    commit = _git_commit()
    report_root = REPORT_DIR / commit
    report_root.mkdir(parents=True, exist_ok=True)

    raw_dir = report_root / "raw"
    raw_dir.mkdir(exist_ok=True)

    summary: dict = {
        "commit": commit,
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "toolchain": {
            "rustc": _tool_version("rustc", ["rustc", "--version"]),
            "cargo": _tool_version("cargo", ["cargo", "--version"]),
            "buf": _tool_version("buf", ["buf", "--version"]),
            "protoc": _tool_version("protoc", ["protoc", "--version"]),
        },
        "system": {
            "os": platform.system(),
            "os_version": platform.release(),
            "arch": platform.machine(),
            "cpu": _cpu_info(),
            "memory": _memory_gb(),
            "python": platform.python_version(),
        },
        "commands": [],
        "unrun": [],
        "features": [],
        "warnings": [],
        "failures": [],
    }

    commands: list[tuple[str, list[str], Optional[int]]] = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"], 120),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"], 600),
        ("test", ["cargo", "test", "--workspace", "--lib", "--bins", "--tests"], 900),
        ("deny", ["cargo", "deny", "check"], 120),
    ]

    if shutil.which("buf"):
        commands.extend([
            ("buf-format", ["buf", "format", "--diff", "--exit-code"], 120),
            ("buf-lint", ["buf", "lint"], 120),
        ])
    else:
        summary["unrun"].append({"command": "buf format/lint", "reason": "buf not installed in this environment"})

    if shutil.which("cargo-nextest"):
        commands.append(("nextest", ["cargo", "nextest", "run", "--workspace"], 900))
    else:
        summary["unrun"].append({"command": "cargo nextest", "reason": "cargo-nextest not installed; fell back to cargo test"})

    total_pass = total_fail = total_ignore = 0
    for name, cmd, timeout in commands:
        try:
            rc, out, dur = _run(cmd, timeout=timeout)
        except subprocess.TimeoutExpired:
            rc, out, dur = -1, f"Timed out after {timeout}s", float(timeout or 0)

        (raw_dir / f"{name}.txt").write_text(out, encoding="utf-8")

        if name == "test":
            p, f, i, _ = _cargo_test_summary(out)
            total_pass += p
            total_fail += f
            total_ignore += i

        for line in out.splitlines():
            mapping = _known_failure_mapping(line)
            if mapping and mapping not in summary["failures"]:
                summary["failures"].append(mapping)
            if "warning:" in line and line not in summary["warnings"]:
                summary["warnings"].append(line.strip()[:500])

        summary["commands"].append({
            "name": name,
            "command": " ".join(cmd),
            "exit_code": rc,
            "duration_seconds": round(dur, 2),
            "raw": os.path.relpath(raw_dir / f"{name}.txt", REPO),
            "passed": total_pass if name == "test" else None,
            "failed": total_fail if name == "test" else None,
            "ignored": total_ignore if name == "test" else None,
        })

    # Collect active cargo features from workspace members for the report.
    try:
        rc, out, _ = _run(["cargo", "metadata", "--format-version", "1"], timeout=120)
        if rc == 0:
            meta = json.loads(out)
            members = set(meta.get("workspace_members", []))
            features: set[str] = set()
            resolve = meta.get("resolve")
            if resolve:
                for node in resolve.get("nodes", []):
                    if node.get("id") in members:
                        features.update(node.get("features", []))
            else:
                for pkg in meta.get("packages", []):
                    if pkg.get("id") in members:
                        features.update(pkg.get("features", {}).keys())
            summary["features"] = sorted(features)
    except Exception:
        summary["features"] = []

    # Write machine-readable summary.
    (report_root / "summary.json").write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")

    # Write Markdown summary suitable for committing to dev-docs.
    md = [
        "# BAS-006: Workspace Baseline Report\n\n",
        f"- Commit: `{commit}`\n",
        f"- Generated: {summary['generated_at']}\n\n",
        "## Toolchain\n\n",
        f"- rustc: {summary['toolchain']['rustc']}\n",
        f"- cargo: {summary['toolchain']['cargo']}\n",
        f"- buf: {summary['toolchain']['buf']}\n",
        f"- protoc: {summary['toolchain']['protoc']}\n\n",
        "## System\n\n",
        f"- OS: {summary['system']['os']} {summary['system']['os_version']}\n",
        f"- Arch: {summary['system']['arch']}\n",
        f"- CPU: {summary['system']['cpu']}\n",
        f"- Memory: {summary['system']['memory']}\n\n",
        "## Commands\n\n",
        "| command | exit | duration (s) | passed | failed | ignored | raw |\n",
        "| --- | ---: | ---: | ---: | ---: | ---: | --- |\n",
    ]
    for c in summary["commands"]:
        passed = c["passed"] if c["passed"] is not None else "-"
        failed = c["failed"] if c["failed"] is not None else "-"
        ignored = c["ignored"] if c["ignored"] is not None else "-"
        md.append(
            f"| `{c['name']}` | {c['exit_code']} | {c['duration_seconds']} | {passed} | {failed} | {ignored} | `{c['raw']}` |\n"
        )
    md.append("\n## Unrun\n\n")
    if summary["unrun"]:
        for u in summary["unrun"]:
            md.append(f"- `{u['command']}`: {u['reason']}\n")
    else:
        md.append("- All configured commands executed.\n")
    md.append("\n## Features\n\n")
    if summary["features"]:
        md.append(", ".join(f"`{f}`" for f in summary["features"]) + "\n")
    else:
        md.append("- No workspace features captured.\n")
    md.append("\n## Warnings\n\n")
    if summary["warnings"]:
        for w in summary["warnings"][:20]:
            md.append(f"- {w}\n")
    else:
        md.append("- None captured.\n")
    md.append("\n## Failure Mapping\n\n")
    if summary["failures"]:
        for f in summary["failures"]:
            md.append(f"- {f}\n")
    else:
        md.append("- No known failures mapped.\n")

    md_path = report_root / "summary.md"
    md_path.write_text("".join(md), encoding="utf-8")

    print(f"Baseline report written to: {report_root}")
    print(f"  Markdown summary: {md_path}")
    print(f"  JSON summary: {report_root / 'summary.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
