#!/usr/bin/env python3
"""Sync or validate offline AFDATA assets bundled by release packages."""

from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

# Keep exact machine-readable contracts byte-identical in every language
# package and in the installed skill. Narrative skill references stay focused
# and are maintained separately.
CANONICAL_FILES = (
    Path("spec/registry.json"),
    Path("spec/protocol-v1.schema.json"),
    Path("spec/cli-help-v2.schema.json"),
    Path("spec/cli-spec-v1.schema.json"),
)

PACKAGE_ASSET_ROOTS = (
    Path("go/assets"),
    Path("python/agent_first_data/assets"),
    Path("typescript/assets"),
    Path("skills/agent-first-data/references"),
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
    failures.extend(orphans())
    return failures


def orphans() -> list[str]:
    """Contracts an asset root still carries after they left `spec/`.

    Syncing only ever writes the canonical list, so a deleted contract lingers
    in all four package trees, and `typescript/package.json` (`files: assets`)
    and `python/pyproject.toml` (`assets/*.json`) would ship it. Checking for
    missing files alone cannot see that.
    """
    expected = {package_relative_path(canonical).name for canonical in CANONICAL_FILES}
    stale: list[str] = []
    for asset_root in PACKAGE_ASSET_ROOTS:
        directory = ROOT / asset_root
        if not directory.is_dir():
            continue
        for found in sorted(directory.glob("*.json")):
            if found.name not in expected:
                stale.append(
                    f"orphan {found.relative_to(ROOT)}; it is no longer a canonical contract"
                )
    return stale


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
