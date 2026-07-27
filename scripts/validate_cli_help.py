#!/usr/bin/env python3
"""Validate the compact cross-language AFDATA CLI help-v1 contract."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_PATH = ROOT / "spec" / "cli-help-v1.schema.json"
FIXTURE_PATH = ROOT / "spec" / "fixtures" / "cli_help_v1.json"


class HelpValidationError(ValueError):
    """A deterministic CLI help-v1 shape failure."""


def _schema() -> dict[str, Any]:
    value = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise HelpValidationError("schema root must be an object")
    return value


def _require_object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise HelpValidationError(f"{path} must be an object")
    return value


def _require_exact_keys(
    value: dict[str, Any],
    *,
    allowed: set[str],
    required: set[str],
    path: str,
) -> None:
    missing = required - set(value)
    if missing:
        raise HelpValidationError(f"{path} missing keys: {sorted(missing)}")
    unknown = set(value) - allowed
    if unknown:
        raise HelpValidationError(f"{path} has unknown keys: {sorted(unknown)}")


def _require_nonempty_string(value: Any, path: str) -> None:
    if not isinstance(value, str) or not value:
        raise HelpValidationError(f"{path} must be a non-empty string")


def _schema_keys(schema: dict[str, Any], definition: str) -> tuple[set[str], set[str]]:
    shape = schema["$defs"][definition]
    return set(shape["properties"]), set(shape["required"])


def _validate_argument(value: Any, path: str, schema: dict[str, Any]) -> None:
    argument = _require_object(value, path)
    allowed, required = _schema_keys(schema, "argument")
    _require_exact_keys(argument, allowed=allowed, required=required, path=path)
    _require_nonempty_string(argument["name"], f"{path}.name")

    for key in ("short", "help", "value"):
        if key in argument:
            _require_nonempty_string(argument[key], f"{path}.{key}")
    if "short" in argument and (
        len(argument["short"]) != 2 or not argument["short"].startswith("-")
    ):
        raise HelpValidationError(f"{path}.short must be one short flag")
    if "default" in argument and not isinstance(argument["default"], str):
        raise HelpValidationError(f"{path}.default must be a string")
    for key in ("required", "global", "repeatable"):
        if key in argument and argument[key] is not True:
            raise HelpValidationError(f"{path}.{key} must be true when present")
    for key in ("values", "defaults"):
        if key not in argument:
            continue
        values = argument[key]
        if not isinstance(values, list) or not values:
            raise HelpValidationError(f"{path}.{key} must be a non-empty array")
        for index, item in enumerate(values):
            if not isinstance(item, str) or (key == "values" and not item):
                raise HelpValidationError(f"{path}.{key}[{index}] must be a string")
    if "value" in argument and "values" in argument:
        raise HelpValidationError(f"{path} cannot contain both value and values")
    if "default" in argument and "defaults" in argument:
        raise HelpValidationError(f"{path} cannot contain both default and defaults")
    if not argument["name"].startswith("-"):
        # A positional's name already carries its placeholder, so a single
        # value/values entry would only repeat it. A positional taking several
        # values genuinely has several placeholders: it lists every one, the
        # first of which is the name.
        if "value" in argument:
            raise HelpValidationError(
                f"{path} positional name already carries its value placeholder"
            )
        values = argument.get("values")
        if values is not None:
            if len(values) < 2:
                raise HelpValidationError(
                    f"{path} positional name already carries its value placeholder"
                )
            if values[0] != argument["name"]:
                raise HelpValidationError(
                    f"{path}.values[0] must be the positional's own name"
                )


def _validate_command(
    value: Any,
    path: str,
    schema: dict[str, Any],
    *,
    help_root: bool,
) -> None:
    command = _require_object(value, path)
    definition = "help" if help_root else "command"
    allowed, required = _schema_keys(schema, definition)
    _require_exact_keys(command, allowed=allowed, required=required, path=path)
    _require_nonempty_string(command["name"], f"{path}.name")

    if help_root:
        if command["scope"] not in {"one_level", "recursive"}:
            raise HelpValidationError(f"{path}.scope must be one_level or recursive")
        _require_nonempty_string(command["command_path"], f"{path}.command_path")
        inherited = command.get("inherited_arguments_from", [])
        if not isinstance(inherited, list) or (
            "inherited_arguments_from" in command and not inherited
        ):
            raise HelpValidationError(
                f"{path}.inherited_arguments_from must be a non-empty array"
            )
        for index, source in enumerate(inherited):
            _require_nonempty_string(
                source, f"{path}.inherited_arguments_from[{index}]"
            )
        if len(inherited) != len(set(inherited)):
            raise HelpValidationError(
                f"{path}.inherited_arguments_from must contain unique paths"
            )
    for key in ("about", "usage"):
        if key in command:
            _require_nonempty_string(command[key], f"{path}.{key}")
    # A compact usage is one line. Scraping a parser's rendered usage would
    # otherwise smuggle its terminal-width wrapping and column padding into the
    # field, which is neither compact nor machine-friendly.
    if "usage" in command and ("\n" in command["usage"] or "\r" in command["usage"]):
        raise HelpValidationError(f"{path}.usage must be a single line")

    arguments = command.get("arguments", [])
    if not isinstance(arguments, list) or ("arguments" in command and not arguments):
        raise HelpValidationError(f"{path}.arguments must be a non-empty array")
    for index, argument in enumerate(arguments):
        _validate_argument(argument, f"{path}.arguments[{index}]", schema)

    subcommands = command.get("subcommands", [])
    if not isinstance(subcommands, list) or (
        "subcommands" in command and not subcommands
    ):
        raise HelpValidationError(f"{path}.subcommands must be a non-empty array")
    for index, subcommand in enumerate(subcommands):
        _validate_command(
            subcommand,
            f"{path}.subcommands[{index}]",
            schema,
            help_root=False,
        )


def validate_help_event(value: Any) -> None:
    """Validate one protocol-wrapped CLI help-v1 event."""

    schema = _schema()
    event = _require_object(value, "$")
    _require_exact_keys(
        event,
        allowed={"kind", "result", "trace"},
        required={"kind", "result", "trace"},
        path="$",
    )
    if event["kind"] != "result":
        raise HelpValidationError("$.kind must be result")
    _require_object(event["trace"], "$.trace")

    result = _require_object(event["result"], "$.result")
    _require_exact_keys(
        result,
        allowed={"code", "help"},
        required={"code", "help"},
        path="$.result",
    )
    if result["code"] != "help":
        raise HelpValidationError("$.result.code must be help")
    _validate_command(result["help"], "$.result.help", schema, help_root=True)


def main() -> int:
    fixtures = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
    valid = fixtures.get("valid", [])
    invalid = fixtures.get("invalid", [])
    if not valid or not invalid:
        print("cli help validation failed: fixtures need valid and invalid cases", file=sys.stderr)
        return 1

    try:
        for event in valid:
            validate_help_event(event)
        for case in invalid:
            try:
                validate_help_event(case["event"])
            except HelpValidationError:
                continue
            raise HelpValidationError(
                f"invalid fixture unexpectedly passed: {case.get('name', '<unnamed>')}"
            )
    except (HelpValidationError, KeyError, TypeError) as error:
        print(f"cli help validation failed: {error}", file=sys.stderr)
        return 1

    print(
        f"cli help schema ok: {len(valid)} valid and {len(invalid)} invalid fixtures"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
