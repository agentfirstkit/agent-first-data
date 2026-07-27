#!/usr/bin/env python3
"""Assert that `--output` selects a help *format*, never a different surface.

Scope and format are orthogonal in the spec, so the same `--help --recursive`
request must describe the same commands and the same arguments no matter which
renderer produced it. When Markdown was rendered by Clap's own help writer
instead of the shared model it silently disagreed twice: it advertised `-h`
after that flag was removed, and it repeated `--help` on all 23 commands where
JSON listed it only on the root.

Prose may differ — Markdown is the documentation export and keeps long-form
sections the compact model drops. The *surface* may not.

Usage: validate_help_formats_agree.py [PATH_TO_AFDATA]
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(binary: str, args: list[str]) -> str:
    result = subprocess.run(
        [binary, *args], check=True, capture_output=True, text=True
    )
    return result.stdout


def json_surface(binary: str) -> dict[str, list[str]]:
    event = json.loads(run(binary, ["--help", "--recursive"]))
    surface: dict[str, list[str]] = {}

    def walk(command: dict, prefix: str) -> None:
        path = f"{prefix} {command['name']}".strip() if prefix else command["name"]
        surface[path] = [arg["name"] for arg in command.get("arguments", [])]
        for sub in command.get("subcommands", []):
            walk(sub, path)

    walk(event["result"]["help"], "")
    return surface


def markdown_surface(binary: str) -> dict[str, list[str]]:
    surface: dict[str, list[str]] = {}
    current: str | None = None
    for line in run(binary, ["--help", "--recursive", "--output", "markdown"]).splitlines():
        if line.startswith("#"):
            current = line.lstrip("# ").split(" - ")[0].strip()
            surface.setdefault(current, [])
        elif current and line.startswith("| `") and "Command | Summary" not in line:
            signature = line.split("|")[1].strip().strip("`")
            name = signature.split(",")[0].split(" <")[0].strip().rstrip(".")
            # Subcommand tables list invocable paths; argument tables do not.
            if not name.startswith(current.split()[0]):
                surface[current].append(name)
    return surface


def plain_surface(binary: str) -> dict[str, list[str]]:
    surface: dict[str, list[str]] = {}
    current: str | None = None
    for line in run(binary, ["--help", "--recursive", "--output", "plain"]).splitlines():
        if not line.startswith(" ") and line.strip():
            current = re.split(r" [—\[<]", line)[0].strip()
            surface.setdefault(current, [])
        elif current and line.startswith("  "):
            # A positional's placeholder may itself contain `=` (FIELD=VALUE),
            # and a repeatable one is suffixed with `...` before the `: help`.
            match = re.match(r"\s+(-{1,2}[\w-]+|[A-Z][A-Z0-9_=]*)", line)
            if match:
                surface[current].append(match.group(1))
    return surface


def main(argv: list[str]) -> int:
    binary = argv[1] if len(argv) > 1 else str(ROOT / "target" / "debug" / "afdata")
    if not Path(binary).exists():
        print(f"binary not found: {binary}", file=sys.stderr)
        return 1

    reference = json_surface(binary)
    failures = []
    for label, surface in (
        ("markdown", markdown_surface(binary)),
        ("plain", plain_surface(binary)),
    ):
        missing = sorted(set(reference) - set(surface))
        extra = sorted(set(surface) - set(reference))
        if missing or extra:
            failures.append(f"{label}: command set differs (missing {missing}, extra {extra})")
        for command in sorted(set(reference) & set(surface)):
            if reference[command] != surface[command]:
                failures.append(
                    f"{label}: {command}\n"
                    f"      json: {reference[command]}\n"
                    f"  {label:>8}: {surface[command]}"
                )

    if failures:
        print("Help formats disagree about the command surface:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        print(
            "`--output` selects a format, not a different set of commands or arguments.",
            file=sys.stderr,
        )
        return 1

    print(f"help formats agree: {len(reference)} commands across json, plain, markdown")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
