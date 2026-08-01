#!/usr/bin/env python3
"""Assert every committed copy of this spore's version agrees.

One spore ships four packages — crates.io, npm, PyPI, and a Go module — and the
release pipeline reads the version from `Cargo.toml` alone. Every registry step
is idempotent by version, so a bump that misses one language does not fail: npm
and PyPI quietly skip publishing while crates.io and the GitHub release move
forward, and `afdata.Version` in Go reports a version that was never shipped.
That failure is invisible at release time and permanent afterwards, so the
agreement is checked here instead.

Usage: validate_versions.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def toml_package_version(path: Path, table: str) -> str:
    text = path.read_text(encoding="utf-8")
    section = re.search(rf"^\[{re.escape(table)}\]$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    if not section:
        raise SystemExit(f"{path}: no [{table}] table")
    match = re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), re.M)
    if not match:
        raise SystemExit(f"{path}: no version in [{table}]")
    return match.group(1)


def json_version(path: Path, *keys: str) -> str:
    node = json.loads(path.read_text(encoding="utf-8"))
    for key in keys:
        node = node[key]
    return node


def go_const_version(path: Path) -> str:
    match = re.search(r'^const Version = "([^"]+)"', path.read_text(encoding="utf-8"), re.M)
    if not match:
        raise SystemExit(f"{path}: no `const Version`")
    return match.group(1)


def cargo_lock_self_version(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    match = re.search(r'^\[\[package\]\]\nname = "agent-first-data"\nversion = "([^"]+)"', text, re.M)
    if not match:
        raise SystemExit(f"{path}: no self entry for agent-first-data")
    return match.group(1)


def main() -> int:
    sources = {
        "Cargo.toml": toml_package_version(ROOT / "Cargo.toml", "package"),
        "Cargo.lock": cargo_lock_self_version(ROOT / "Cargo.lock"),
        "python/pyproject.toml": toml_package_version(ROOT / "python" / "pyproject.toml", "project"),
        "typescript/package.json": json_version(ROOT / "typescript" / "package.json", "version"),
        "typescript/package-lock.json": json_version(
            ROOT / "typescript" / "package-lock.json", "version"
        ),
        "typescript/package-lock.json (packages)": json_version(
            ROOT / "typescript" / "package-lock.json", "packages", "", "version"
        ),
        "go/version.go": go_const_version(ROOT / "go" / "version.go"),
        "spore.core.json": json_version(ROOT / "spore.core.json", "version"),
    }

    canonical = sources["Cargo.toml"]
    disagree = {where: found for where, found in sources.items() if found != canonical}
    if disagree:
        print(f"version disagreement (Cargo.toml says {canonical}):\n", file=sys.stderr)
        for where, found in sorted(disagree.items()):
            print(f"  - {where}: {found}", file=sys.stderr)
        return 1

    print(f"versions ok: {canonical} in {len(sources)} places")
    return 0


if __name__ == "__main__":
    sys.exit(main())
