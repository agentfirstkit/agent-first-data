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
    ROOT / "skills" / "agent-first-data" / "references" / "cli-protocol.md",
    ROOT / "rust" / "README.md",
    ROOT / "python" / "README.md",
    ROOT / "go" / "README.md",
    ROOT / "typescript" / "README.md",
)


def documented_events() -> list[dict]:
    return [event for _, event in sourced_documented_events()]


def sourced_documented_events() -> list[tuple[str, dict]]:
    """Every documented protocol event, paired with where it is written.

    The provenance is what makes a rejection actionable: `validate_protocol_schema.py`
    reports events by source, and "the third example in cli-protocol.md" is a
    place to go fix, where "documented event 27" is not.
    """
    events: list[tuple[str, dict]] = []
    for path in DOCS:
        text = path.read_text(encoding="utf-8")
        source = path.relative_to(ROOT).as_posix()
        if '"code": "ok"' in text:
            raise ValueError(f"legacy code:ok example in {path.relative_to(ROOT)}")
        if "use `code` / `result` / `error` / `trace` structure" in text:
            raise ValueError(f"legacy protocol checklist in {path.relative_to(ROOT)}")
        for number, block in enumerate(
            re.findall(r"```json\s*\n(.*?)```", text, re.DOTALL), start=1
        ):
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
                    events.append((f"{source} json block {number}", value))
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


HELP_SCHEMA = ROOT / "spec" / "cli-help-v2.schema.json"


# Every keyword this checker understands, assertions and annotations alike.
# The list is a whitelist on purpose: a keyword nobody implemented is a
# constraint the walk below would skip in silence, and a partial validator that
# says nothing is indistinguishable from a passing one. Meeting an unknown
# keyword is therefore reported as a failure of the checker, not ignored —
# which also means adding a keyword to a schema cannot quietly go unchecked.
KNOWN_KEYWORDS = frozenset(
    {
        # annotations, no assertion of their own
        "$schema",
        "$id",
        "$defs",
        "$comment",
        "title",
        "description",
        "default",
        "examples",
        # assertions
        "$ref",
        "type",
        "const",
        "enum",
        "properties",
        "required",
        "additionalProperties",
        "minProperties",
        "items",
        "minItems",
        "uniqueItems",
        "minLength",
        "pattern",
        "minimum",
        "oneOf",
        "allOf",
        "if",
        "then",
        "else",
    }
)

JSON_TYPES = ("object", "array", "string", "boolean", "number", "integer", "null")


def json_type_name(value) -> str:
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, dict):
        return "object"
    if isinstance(value, list):
        return "array"
    if isinstance(value, str):
        return "string"
    if isinstance(value, (int, float)):
        return "number"
    if value is None:
        return "null"
    return type(value).__name__


def json_type_matches(value, name: str) -> bool:
    if name == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if name == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    return json_type_name(value) == name


def branch_label(branch, index: int) -> str:
    """Name a `oneOf`/`allOf` branch so a rejection can say which one it is."""
    if isinstance(branch, dict) and isinstance(branch.get("$ref"), str):
        return branch["$ref"].rsplit("/", 1)[-1]
    return f"branch {index}"


def check_against_schema(value, schema, root: dict, where: str) -> list[str]:
    """Structural conformance against a JSON Schema subset.

    Implements `$ref`, boolean schemas, `type`, `const`, `enum`, `required`,
    `properties`, `additionalProperties`, `minProperties`, `items`, `minItems`,
    `uniqueItems`, `minLength`, `pattern`, `minimum`, and the `oneOf` / `allOf`
    / `if`-`then`-`else` combinators — everything `cli-help-v2` and
    `protocol-v1` use, and nothing else (see `KNOWN_KEYWORDS`).

    Not a full JSON Schema implementation. It exists to catch a documented
    example that violates the contract it is documenting: the `help` object is
    `additionalProperties: false`, and a skill reference shipped an example with
    `examples` and `detail_command` on it; nothing looked, because protocol-v1
    validation never descends into `result.help`. `protocol-v1` needs the
    combinators because it discriminates its four event kinds with `oneOf`.

    Its verdicts are pinned to the ones the afdata validators reach, over the
    whole protocol corpus, by `validate_protocol_schema.py` — a subset checker
    that drifted lax or strict would fail that gate.
    """
    if schema is True:
        return []
    if schema is False:
        return [f"{where}: schema allows no value here"]
    while "$ref" in schema:
        ref = schema["$ref"]
        if not ref.startswith("#/$defs/"):
            return [f"{where}: cannot resolve $ref {ref!r}"]
        schema = root["$defs"][ref.removeprefix("#/$defs/")]

    unknown = sorted(set(schema) - KNOWN_KEYWORDS)
    if unknown:
        return [
            f"{where}: schema uses {', '.join(unknown)}, which this checker does "
            f"not implement — it would pass silently. Extend the checker or validate "
            f"this document with a real JSON Schema implementation."
        ]

    if "const" in schema and value != schema["const"]:
        return [f"{where}: expected {schema['const']!r}, found {value!r}"]
    if "enum" in schema and value not in schema["enum"]:
        return [f"{where}: {value!r} is not one of {schema['enum']}"]
    if "type" in schema:
        names = schema["type"] if isinstance(schema["type"], list) else [schema["type"]]
        if any(name not in JSON_TYPES for name in names):
            return [f"{where}: schema names an unknown type: {schema['type']!r}"]
        if not any(json_type_matches(value, name) for name in names):
            return [
                f"{where}: expected {' or '.join(names)}, found {json_type_name(value)}"
            ]

    problems: list[str] = []
    # Every remaining keyword applies to one instance type and passes for the
    # others, exactly as JSON Schema defines it: `required` on a string is not a
    # violation, it is silence. That is why `type` is asserted separately above
    # rather than gating the walk.
    if isinstance(value, dict):
        problems += check_object(value, schema, root, where)
    elif isinstance(value, list):
        problems += check_array(value, schema, root, where)
    elif isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            problems.append(
                f"{where}: needs at least {schema['minLength']} character(s), "
                f"found {len(value)}"
            )
        if "pattern" in schema and not re.search(schema["pattern"], value):
            problems.append(f"{where}: {value!r} does not match {schema['pattern']!r}")
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        if "minimum" in schema and value < schema["minimum"]:
            problems.append(f"{where}: {value} is below the minimum {schema['minimum']}")

    problems += check_combinators(value, schema, root, where)
    return problems


def check_object(value: dict, schema: dict, root: dict, where: str) -> list[str]:
    problems: list[str] = []
    properties = schema.get("properties", {})
    for name in schema.get("required", []):
        if name not in value:
            problems.append(f"{where}: missing required field `{name}`")
    additional = schema.get("additionalProperties")
    if additional is False:
        for name in value:
            if name not in properties:
                problems.append(
                    f"{where}: field `{name}` is not allowed "
                    f"(permitted: {', '.join(sorted(properties))})"
                )
    if len(value) < schema.get("minProperties", 0):
        problems.append(
            f"{where}: needs at least {schema['minProperties']} field(s), "
            f"found {len(value)}"
        )
    for name, child in value.items():
        if name in properties:
            problems += check_against_schema(
                child, properties[name], root, f"{where}.{name}"
            )
        elif isinstance(additional, dict):
            problems += check_against_schema(child, additional, root, f"{where}.{name}")
    return problems


def check_array(value: list, schema: dict, root: dict, where: str) -> list[str]:
    problems: list[str] = []
    if "items" in schema:
        for index, item in enumerate(value):
            problems += check_against_schema(
                item, schema["items"], root, f"{where}[{index}]"
            )
    if len(value) < schema.get("minItems", 0):
        problems.append(
            f"{where}: needs at least {schema['minItems']} item(s), found {len(value)}"
        )
    if schema.get("uniqueItems"):
        for index, item in enumerate(value):
            if item in value[index + 1 :]:
                problems.append(f"{where}: duplicate item {item!r}")
                break
    return problems


def check_combinators(value, schema: dict, root: dict, where: str) -> list[str]:
    problems: list[str] = []
    for branch in schema.get("allOf", []):
        problems += check_against_schema(value, branch, root, where)
    if "oneOf" in schema:
        branches = schema["oneOf"]
        matched: list[str] = []
        rejected: list[tuple[str, list[str]]] = []
        for index, branch in enumerate(branches):
            label = branch_label(branch, index)
            found = check_against_schema(value, branch, root, where)
            if found:
                rejected.append((label, found))
            else:
                matched.append(label)
        # A `oneOf` rejection has to say why *every* branch refused the value,
        # or the four-way event dispatch is undebuggable: "no branch matched"
        # alone cannot distinguish an unknown `kind` from a malformed payload.
        if not matched:
            detail = "".join(
                f"\n      {label}: {'; '.join(found)}" for label, found in rejected
            )
            problems.append(
                f"{where}: matches none of the {len(branches)} `oneOf` branches:{detail}"
            )
        elif len(matched) > 1:
            problems.append(
                f"{where}: matches {len(matched)} `oneOf` branches "
                f"({', '.join(matched)}); exactly one may match"
            )
    if "if" in schema:
        condition = check_against_schema(value, schema["if"], root, where)
        taken = "else" if condition else "then"
        if taken in schema:
            problems += check_against_schema(value, schema[taken], root, where)
    return problems


def check_help_events(events: list[dict]) -> tuple[int, list[str]]:
    schema = json.loads(HELP_SCHEMA.read_text(encoding="utf-8"))
    problems: list[str] = []
    checked = 0
    for event in events:
        result = event.get("result")
        if not isinstance(result, dict) or result.get("code") != "help":
            continue
        checked += 1
        problems += check_against_schema(event, schema, schema, "help event")
    return checked, problems


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
                handle.name,
                "--strict",
                "--per-event",
            ],
            cwd=ROOT,
            check=False,
        )
    if result.returncode != 0:
        return result.returncode
    helps, problems = check_help_events(events)
    if problems:
        print("documented help events do not match cli-help-v2:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"protocol docs ok: {len(events)} strict events, {helps} against cli-help-v2")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
