#!/usr/bin/env python3
"""Sync or validate offline AFDATA assets bundled by release packages."""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

# The non-Rust packages bundle only the core spec data (suffix registry +
# protocol schema). The Agent Skill (SKILL.md + references/) is not synced into
# them: skill-admin is a Rust-only capability, so its bundled skill lives only in
# the Rust CLI (which reads skills/ directly via include_str!).
CANONICAL_FILES = (
    Path("spec/registry.json"),
    Path("spec/protocol-v1.schema.json"),
)

PACKAGE_ASSET_ROOTS = (
    Path("go/assets"),
    Path("python/agent_first_data/assets"),
    Path("typescript/assets"),
)


def package_relative_path(canonical: Path) -> Path:
    if canonical.parts[0] == "spec":
        return Path(*canonical.parts[1:])
    return canonical


def sync(check: bool) -> list[str]:
    failures: list[str] = []
    for asset_root in PACKAGE_ASSET_ROOTS:
        for canonical in CANONICAL_FILES:
            source = ROOT / canonical
            target = ROOT / asset_root / package_relative_path(canonical)
            if check:
                if not target.exists():
                    failures.append(f"missing {target.relative_to(ROOT)}")
                    continue
                if source.read_bytes() != target.read_bytes():
                    failures.append(
                        f"stale {target.relative_to(ROOT)}; run scripts/sync_offline_assets.py"
                    )
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="validate without writing")
    args = parser.parse_args()
    failures = sync(args.check)
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
