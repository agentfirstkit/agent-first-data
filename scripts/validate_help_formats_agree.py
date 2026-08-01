#!/usr/bin/env python3
"""Assert that CLI help-v2 JSON and plain render the same registered shapes.

Usage: validate_help_formats_agree.py [PATH_TO_AFDATA]
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOOL_PREFIX = ""


def run(binary: str, args: list[str]) -> str:
    result = subprocess.run(
        [binary, *args], check=True, capture_output=True, text=True
    )
    return result.stdout


def help_model(binary: str, command_path: list[str]) -> dict:
    event = json.loads(run(binary, [*command_path, "--help", "--output", "json"]))
    assert event["kind"] == "result"
    assert event["result"]["code"] == "help"
    model = event["result"]["help"]
    assert model["schema"] == "cli-help-v2"
    return model


def plain_usages(text: str) -> list[str]:
    """The syntax lines a plain reader sees, in order."""
    # A syntax line is exactly a line that starts with the invocation itself.
    # The command's description, a shape's id/description, and the subcommand
    # pointers never do.
    return [line for line in text.splitlines() if line.startswith(TOOL_PREFIX)]


def subcommand_path(command: str) -> list[str]:
    prefix = "afdata "
    suffix = " --help"
    if not command.startswith(prefix) or not command.endswith(suffix):
        raise ValueError(f"non-canonical subcommand help template: {command!r}")
    return command[len(prefix) : -len(suffix)].split()


def main(argv: list[str]) -> int:
    binary = argv[1] if len(argv) > 1 else str(ROOT / "target" / "debug" / "afdata")
    global TOOL_PREFIX
    TOOL_PREFIX = Path(binary).stem + " "
    if not Path(binary).exists():
        print(f"binary not found: {binary}", file=sys.stderr)
        return 1

    pending: list[list[str]] = [[]]
    visited: set[tuple[str, ...]] = set()
    shapes_count = 0
    while pending:
        command_path = pending.pop()
        key = tuple(command_path)
        if key in visited:
            continue
        visited.add(key)
        model = help_model(binary, command_path)
        plain = run(binary, [*command_path, "--help", "--output", "plain"])
        shapes = model.get("shapes", [])
        shapes_count += len(shapes)

        # One round trip, so JSON and plain must carry the same complete
        # syntax for every shape — there is no second level to defer to.
        expected = [shape["usage"] for shape in shapes]
        actual = plain_usages(plain)
        if expected != actual:
            raise ValueError(
                f"help syntax differs for {model['command_path']}: "
                f"json={expected!r}, plain={actual!r}"
            )
        # Every argument meaning and default the structured form carries has to
        # reach the plain reader too. Comparing only the syntax lines let plain
        # drop `notes` and `defaults` entirely while this check stayed green —
        # the "human catalog" was strictly weaker than the JSON.
        missing_notes = [
            f"{name}: {note}"
            for name, note in model.get("notes", {}).items()
            if name not in plain or note not in plain
        ]
        if missing_notes:
            raise ValueError(
                f"plain help for {model['command_path']} omits argument notes "
                f"the JSON carries: {missing_notes!r}"
            )
        missing_defaults = [
            name for name in model.get("defaults", {}) if name not in plain
        ]
        if missing_defaults:
            raise ValueError(
                f"plain help for {model['command_path']} omits defaults "
                f"the JSON carries: {missing_defaults!r}"
            )

        if len(shapes) > 1:
            undescribed = [shape["id"] for shape in shapes if not shape.get("about")]
            if undescribed:
                raise ValueError(
                    f"{model['command_path']} has sibling shapes without a description: "
                    f"{undescribed!r}"
                )
        elif shapes and shapes[0].get("about"):
            raise ValueError(
                f"{model['command_path']} has one shape that repeats a description"
            )

        pending.extend(
            subcommand_path(command)
            for command in model.get("subcommands", [])
        )

    print(
        f"help formats agree: {len(visited)} commands, "
        f"{shapes_count} registered shapes"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except (AssertionError, KeyError, ValueError) as error:
        print(f"Help formats disagree: {error}", file=sys.stderr)
        sys.exit(1)
