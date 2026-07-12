#!/usr/bin/env python3
"""Validate documented AFDATA events through the afdata CLI itself."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DOCS = (
    ROOT / "spec" / "agent-first-data.md",
    ROOT / "skills" / "agent-first-data" / "references" / "rules.md",
    ROOT / "rust" / "README.md",
    ROOT / "python" / "README.md",
    ROOT / "go" / "README.md",
    ROOT / "typescript" / "README.md",
)


def documented_events() -> list[dict]:
    events: list[dict] = []
    for path in DOCS:
        text = path.read_text(encoding="utf-8")
        if '"code": "ok"' in text:
            raise ValueError(f"legacy code:ok example in {path.relative_to(ROOT)}")
        if "use `code` / `result` / `error` / `trace` structure" in text:
            raise ValueError(f"legacy protocol checklist in {path.relative_to(ROOT)}")
        for block in re.findall(r"```json\s*\n(.*?)```", text, re.DOTALL):
            values = []
            try:
                values.append(json.loads(block))
            except json.JSONDecodeError:
                for line in block.splitlines():
                    try:
                        values.append(json.loads(line))
                    except json.JSONDecodeError:
                        continue
            for value in values:
                if isinstance(value, dict) and value.get("kind") in {
                    "result",
                    "error",
                    "progress",
                    "log",
                }:
                    events.append(value)
                elif isinstance(value, dict) and value.get("code") in {
                    "ok",
                    "error",
                    "progress",
                    "log",
                }:
                    raise ValueError(
                        f"legacy top-level protocol event in {path.relative_to(ROOT)}: "
                        f"code={value['code']!r}"
                    )
    return events


def main() -> int:
    try:
        events = documented_events()
    except ValueError as error:
        print(f"protocol documentation validation failed: {error}", file=sys.stderr)
        return 1
    if not events:
        print("protocol documentation validation failed: no protocol events found", file=sys.stderr)
        return 1
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as handle:
        json.dump(events, handle)
        handle.flush()
        result = subprocess.run(
            [
                "cargo",
                "run",
                "--quiet",
                "--bin",
                "afdata",
                "--",
                "validate",
                "--strict",
                "--event",
                handle.name,
            ],
            cwd=ROOT,
            check=False,
        )
    if result.returncode != 0:
        return result.returncode
    print(f"protocol docs ok: {len(events)} strict events")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
