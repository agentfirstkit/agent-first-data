#!/usr/bin/env python3
"""Keep `cli-spec-v1.schema.json` in step with the Rust types it describes.

`cli-spec-v1` is published as a machine-readable contract, and every object in
it is `additionalProperties: false`. That makes the schema strict in one
direction only: adding a field to the Rust structs does not fail any build, it
just quietly produces documents that no longer validate against their own
published schema. That is precisely how `display_name`, `build`, and
`sensitive` drifted out of sync.

So this gate compares, field by field, the serde field names of the four
serializable spec types against the schema locations that describe them, in
both directions. It is deliberately structural rather than a JSON Schema
validation run: it needs no serialized sample and no schema library, and it
fails the moment a field exists on one side and not the other.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC_RS = ROOT / "rust" / "src" / "cli_spec" / "spec.rs"
SCHEMA = ROOT / "spec" / "cli-spec-v1.schema.json"

# Rust struct -> the JSON pointer (as a key path) describing it.
STRUCTS = {
    "CliSpec": ("properties",),
    "CommandSpec": ("$defs", "command", "properties"),
    "ArgSpec": ("$defs", "argument", "properties"),
    "Combination": ("$defs", "combination", "properties"),
}

FIELD = re.compile(r"^\s*pub (?P<name>[a-z_][a-z0-9_]*)\s*:")


def rust_fields(source: str, struct: str) -> list[str]:
    start = source.find(f"pub struct {struct} {{")
    if start == -1:
        raise SystemExit(f"{SPEC_RS}: cannot find `pub struct {struct}`")
    body = source[start : source.index("\n}", start)]
    return [match.group("name") for line in body.splitlines() if (match := FIELD.match(line))]


def schema_at(schema: dict, path: tuple[str, ...]) -> dict:
    node = schema
    for key in path:
        node = node[key]
    return node


def main() -> int:
    source = SPEC_RS.read_text(encoding="utf-8")
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    failures: list[str] = []

    for struct, path in STRUCTS.items():
        try:
            properties = schema_at(schema, path)
        except KeyError:
            failures.append(f"schema is missing {'.'.join(path)} for `{struct}`")
            continue
        declared = set(rust_fields(source, struct))
        described = set(properties)
        where = "/".join(path)
        for name in sorted(declared - described):
            failures.append(
                f"`{struct}.{name}` has no `{where}.{name}` in cli-spec-v1: a spec "
                f"carrying it fails its own schema (additionalProperties is false)"
            )
        for name in sorted(described - declared):
            failures.append(
                f"cli-spec-v1 describes `{where}.{name}`, but `{struct}` has no such field"
            )

    if failures:
        print("cli-spec-v1 schema drift:\n", file=sys.stderr)
        print("\n".join(f"  - {failure}" for failure in failures), file=sys.stderr)
        return 1

    total = sum(len(rust_fields(source, struct)) for struct in STRUCTS)
    print(f"cli-spec-v1 schema ok: {total} fields across {len(STRUCTS)} types")
    return 0


if __name__ == "__main__":
    sys.exit(main())
