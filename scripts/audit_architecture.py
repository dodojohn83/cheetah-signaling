#!/usr/bin/env python3
"""Audit crate dependency directions and production-code placeholders."""

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def cargo_metadata():
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(result.stdout)


# Map crate name prefix to its architectural layer. Lower number = higher in the
# dependency stack (closer to the app). Dependencies must flow downward in number.
LAYER_PATTERNS = [
    # Layer 1: apps / assembly
    ("cheetah-signaling", 1),
    ("cheetah-ctl", 1),
    ("cheetah-migration-tool", 1),
    # Layer 2: storage / messaging / API / media adapters
    ("cheetah-storage-postgres", 2),
    ("cheetah-storage-sqlite", 2),
    ("cheetah-message-nats", 2),
    ("cheetah-message-local", 2),
    ("cheetah-http-api", 2),
    ("cheetah-grpc-api", 2),
    ("cheetah-media-client", 2),
    # Layer 3: application services
    ("cheetah-signal-application", 3),
    ("cheetah-cluster-ownership", 3),
    ("cheetah-media-scheduler", 3),
    # Layer 4: protocol modules
    ("cheetah-gb28181-module", 4),
    ("cheetah-onvif-module", 4),
    ("cheetah-plugin-host", 4),
    # Layer 5: protocol drivers / runtime implementations
    ("cheetah-gb28181-driver-tokio", 5),
    ("cheetah-onvif-driver-tokio", 5),
    ("cheetah-runtime-tokio", 5),
    # Layer 6: core / foundation / domain / ports
    ("cheetah-signal-types", 6),
    ("cheetah-signal-contracts", 6),
    ("cheetah-config", 6),
    ("cheetah-secret", 6),
    ("cheetah-domain", 6),
    ("cheetah-storage-api", 6),
    ("cheetah-message-api", 6),
    ("cheetah-runtime-api", 6),
    ("cheetah-cluster-registry", 6),
    ("cheetah-gb28181-core", 6),
    ("cheetah-onvif-core", 6),
    ("cheetah-plugin-sdk", 6),
]


def layer_of(name):
    for prefix, layer in LAYER_PATTERNS:
        if name == prefix:
            return layer
    return None


FORBIDDEN_DEPS = {
    6: ["tokio", "sqlx", "tonic", "axum", "async-nats", "reqwest"],
    5: ["sqlx"],
    4: ["sqlx", "async-nats", "cheetah-storage-postgres", "cheetah-storage-sqlite"],
    3: ["sqlx"],
}


def check_dependency_direction(metadata):
    errors = []
    warnings = []
    packages_by_id = {p["id"]: p for p in metadata["packages"]}
    dep_kinds = {}

    for pkg in metadata["packages"]:
        for dep in pkg.get("dependencies", []):
            key = (pkg["id"], dep["name"])
            dep_kinds[key] = dep.get("kind", None)

    for node in metadata["resolve"]["nodes"]:
        pkg = packages_by_id.get(node["id"])
        if pkg is None:
            continue
        name = pkg["name"]
        src_layer = layer_of(name)
        if src_layer is None:
            continue
        for dep_id in node.get("dependencies", []):
            dep_pkg = packages_by_id.get(dep_id)
            dep_name = dep_pkg["name"] if dep_pkg else dep_id.split(" ")[0]
            dep_layer = layer_of(dep_name)
            kind = dep_kinds.get((node["id"], dep_name))
            if kind in ("dev", "build"):
                continue
            if dep_layer is not None and dep_layer < src_layer:
                errors.append(
                    f"LAYER VIOLATION: {name} (layer {src_layer}) depends on "
                    f"{dep_name} (layer {dep_layer})"
                )
            forbidden = FORBIDDEN_DEPS.get(src_layer, [])
            for f in forbidden:
                if dep_name == f or dep_name.startswith(f + "-"):
                    warnings.append(
                        f"FORBIDDEN DEP: {name} (layer {src_layer}) -> {dep_name}"
                    )
    return errors, warnings


def in_test_block(path: Path, text, pos):
    """Return True if the position is inside a test module or file."""
    rel = str(path.relative_to(REPO))
    if "/tests/" in rel or rel.endswith("_test.rs") or rel.endswith("_tests.rs"):
        return True
    if path.name in ("tests.rs", "test_support.rs") or "_test_support.rs" in path.name:
        return True
    pre = text[:pos]
    last_cfg_test = pre.rfind("#[cfg(test)]")
    last_mod_tests = pre.rfind("mod tests")
    if last_cfg_test == -1 and last_mod_tests == -1:
        return False
    marker = max(last_cfg_test, last_mod_tests)
    open_braces = 0
    for ch in pre[marker:]:
        if ch == "{":
            open_braces += 1
        elif ch == "}":
            open_braces -= 1
    return open_braces > 0


def scan_placeholders():
    production = []
    test_fakes = []
    panic_warnings = []
    src_dirs = sorted((REPO / "crates").glob("*/*/src")) + sorted(
        (REPO / "apps").glob("*/src")
    )
    for src_dir in src_dirs:
        for path in src_dir.rglob("*.rs"):
            if "target" in path.parts:
                continue
            text = path.read_text(encoding="utf-8")
            for m in re.finditer(r"\b(todo!|unimplemented!|panic!)\s*\(", text):
                line = text[: m.start()].count("\n") + 1
                marker = m.group(1)
                block = in_test_block(path, text, m.start())
                if path.name == "lib.rs" and "tonic" in text and "generated" in text:
                    # crude skip of generated tonic module files
                    continue
                if marker in ("todo!", "unimplemented!"):
                    if block:
                        test_fakes.append(
                            (str(path.relative_to(REPO)), line, marker, block)
                        )
                    else:
                        production.append(
                            (str(path.relative_to(REPO)), line, marker, block)
                        )
                elif marker == "panic!" and not block:
                    panic_warnings.append(
                        (str(path.relative_to(REPO)), line, marker, block)
                    )
    return production, test_fakes, panic_warnings


def scan_direct_sql():
    hits = []
    src_dirs = sorted((REPO / "crates").glob("*/*/src")) + sorted(
        (REPO / "apps").glob("*/src")
    )
    for src_dir in src_dirs:
        for path in src_dir.rglob("*.rs"):
            if "target" in path.parts or "storage" in path.parts:
                continue
            text = path.read_text(encoding="utf-8")
            for m in re.finditer(
                r'r#?"\s*(SELECT|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP)\b', text, re.I
            ):
                line = text[: m.start()].count("\n") + 1
                hits.append((str(path.relative_to(REPO)), line, m.group(1)))
    return hits


def main():
    metadata = cargo_metadata()
    dep_errors, dep_warnings = check_dependency_direction(metadata)
    prod_placeholders, test_placeholders, panic_warnings = scan_placeholders()
    sql_hits = scan_direct_sql()

    print("=== Architecture Audit ===")
    print(f"Dependency layer violations: {len(dep_errors)}")
    for e in dep_errors[:20]:
        print("  ", e)
    print(f"Forbidden dependency warnings: {len(dep_warnings)}")
    for w in dep_warnings[:20]:
        print("  ", w)
    print(f"Production todo!/unimplemented! hits: {len(prod_placeholders)}")
    for h in prod_placeholders[:20]:
        print(f"  {h[0]}:{h[1]} {h[2]}")
    print(f"Production panic! warnings: {len(panic_warnings)}")
    for h in panic_warnings[:20]:
        print(f"  {h[0]}:{h[1]} {h[2]}")
    print(f"Test-fake todo!/unimplemented! hits: {len(test_placeholders)}")
    for h in test_placeholders[:20]:
        print(f"  {h[0]}:{h[1]} {h[2]} {'(inside cfg(test))' if h[3] else '(file contains tests)'}")
    print(f"Direct SQL outside storage crates: {len(sql_hits)}")
    for h in sql_hits[:20]:
        print(f"  {h[0]}:{h[1]} {h[2]}")

    report_path = REPO / "target" / "reports" / "bas-004-architecture-audit.md"
    report_path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# BAS-004: Architecture and Placeholder Audit\n\n",
        f"- Dependency layer violations: {len(dep_errors)}\n",
        f"- Forbidden dependency warnings: {len(dep_warnings)}\n",
        f"- Production `todo!` / `unimplemented!` hits: {len(prod_placeholders)}\n",
        f"- Production `panic!` warnings: {len(panic_warnings)}\n",
        f"- Test-fake `todo!` / `unimplemented!` hits: {len(test_placeholders)}\n",
        f"- Direct SQL outside storage crates: {len(sql_hits)}\n\n",
        "## Dependency layer violations\n\n",
    ]
    for e in dep_errors:
        lines.append(f"- {e}\n")
    lines.append("\n## Forbidden dependency warnings\n\n")
    for w in dep_warnings:
        lines.append(f"- {w}\n")
    lines.append("\n## Production todo! / unimplemented! hits\n\n")
    for h in prod_placeholders:
        lines.append(f"- `{h[0]}:{h[1]}` `{h[2]}`\n")
    lines.append("\n## Production panic! warnings\n\n")
    for h in panic_warnings:
        lines.append(f"- `{h[0]}:{h[1]}` `{h[2]}`\n")
    lines.append("\n## Test-fake placeholder hits\n\n")
    for h in test_placeholders:
        lines.append(f"- `{h[0]}:{h[1]}` `{h[2]}`\n")
    lines.append("\n## Direct SQL outside storage crates\n\n")
    for h in sql_hits:
        lines.append(f"- `{h[0]}:{h[1]}` `{h[2]}`\n")
    report_path.write_text("".join(lines), encoding="utf-8")
    print(f"\nReport written to {report_path}")

    if prod_placeholders or test_placeholders or sql_hits:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
