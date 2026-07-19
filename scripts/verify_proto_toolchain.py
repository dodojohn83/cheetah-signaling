#!/usr/bin/env python3
"""Verify the proto toolchain versions match BAS-002 baseline."""

import re
import subprocess
import sys

EXPECTED_BUF = "1.50.0"
EXPECTED_PROTOC_MAJOR = 25


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=False)


def check_buf():
    result = run(["buf", "--version"])
    if result.returncode != 0:
        print("ERROR: buf is not installed or not in PATH", file=sys.stderr)
        return False
    version = result.stdout.strip().split()[0]
    if not version.startswith(EXPECTED_BUF):
        print(f"ERROR: buf version {version} != expected {EXPECTED_BUF}", file=sys.stderr)
        return False
    print(f"buf: {version}")
    return True


def check_protoc():
    result = run(["protoc", "--version"])
    if result.returncode != 0:
        print("ERROR: protoc is not installed or not in PATH", file=sys.stderr)
        return False
    match = re.search(r"(\d+)\.\d+(?:\.\d+)?", result.stdout)
    if not match:
        print(f"ERROR: could not parse protoc version: {result.stdout}", file=sys.stderr)
        return False
    major = int(match.group(1))
    print(f"protoc: {match.group(0)} (major {major})")
    if major != EXPECTED_PROTOC_MAJOR:
        print(
            f"WARNING: protoc major {major} != expected {EXPECTED_PROTOC_MAJOR}; "
            "CI uses this as canonical",
            file=sys.stderr,
        )
        return False
    return True


def check_buf_format_lint():
    for cmd, desc in (
        (["buf", "format", "--diff", "--exit-code"], "format"),
        (["buf", "lint"], "lint"),
    ):
        result = run(cmd)
        if result.returncode != 0:
            print(f"ERROR: buf {desc} failed:\n{result.stderr}", file=sys.stderr)
            return False
        print(f"buf {desc}: ok")
    return True


def main():
    ok = True
    ok &= check_buf()
    ok &= check_protoc()
    ok &= check_buf_format_lint()
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
