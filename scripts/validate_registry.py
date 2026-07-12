#!/usr/bin/env python3
"""Validate the AFDATA registry and its direct documentation references."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "spec" / "registry.json"
SPEC = ROOT / "spec" / "agent-first-data.md"
SKILL_DIR = ROOT / "skills" / "agent-first-data"
SKILL = SKILL_DIR / "SKILL.md"
FIXTURES = ROOT / "spec" / "fixtures"

REQUIRED_TOP_LEVEL = {"$schema", "schema_version", "name", "description", "safe_integer", "suffixes"}
REQUIRED_SUFFIX_KEYS = {
    "suffix",
    "category",
    "json_types",
    "unit",
    "formatting",
    "redaction",
    "constraints",
}
BANNED_KEYS = {"deprecated", "legacy", "old_marker", "protocol_version"}
ALLOWED_JSON_TYPES = {"any", "null", "boolean", "integer", "number", "string", "array", "object"}


def fail(message: str) -> None:
    print(f"registry validation failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def walk_json(value, path: str = ""):
    yield path, value
    if isinstance(value, dict):
        for key, child in value.items():
            yield from walk_json(child, f"{path}/{key}")
    elif isinstance(value, list):
        for idx, child in enumerate(value):
            yield from walk_json(child, f"{path}/{idx}")


def require_no_banned_keys(registry: dict) -> None:
    for path, value in walk_json(registry):
        if isinstance(value, dict):
            for key in value:
                if key in BANNED_KEYS:
                    fail(f"banned key {key!r} at {path or '/'}")


def validate_registry_shape(registry: dict) -> list[str]:
    missing = REQUIRED_TOP_LEVEL - set(registry)
    if missing:
        fail(f"missing top-level keys: {sorted(missing)}")
    if registry["schema_version"] != 1:
        fail("schema_version must be 1")
    safe = registry["safe_integer"]
    if safe != {"minimum": -9007199254740991, "maximum": 9007199254740991}:
        fail("safe_integer must be ±(2^53-1)")
    suffixes = registry["suffixes"]
    if not isinstance(suffixes, list) or not suffixes:
        fail("suffixes must be a non-empty list")
    seen: set[str] = set()
    out: list[str] = []
    for idx, entry in enumerate(suffixes):
        if not isinstance(entry, dict):
            fail(f"suffixes/{idx} must be an object")
        missing_entry = REQUIRED_SUFFIX_KEYS - set(entry)
        if missing_entry:
            fail(f"suffixes/{idx} missing keys: {sorted(missing_entry)}")
        suffix = entry["suffix"]
        if not isinstance(suffix, str) or not suffix.startswith("_"):
            fail(f"suffixes/{idx}/suffix must be an underscore-prefixed string")
        if suffix in seen:
            fail(f"duplicate suffix {suffix}")
        seen.add(suffix)
        out.append(suffix)
        json_types = entry["json_types"]
        if not isinstance(json_types, list) or not json_types:
            fail(f"{suffix}: json_types must be a non-empty list")
        unknown_types = sorted(set(json_types) - ALLOWED_JSON_TYPES)
        if unknown_types:
            fail(f"{suffix}: unknown json_types {unknown_types}")
        formatting = entry["formatting"]
        if not isinstance(formatting, dict) or "strip_key" not in formatting or "plain_yaml" not in formatting:
            fail(f"{suffix}: formatting must include strip_key and plain_yaml")
        if not isinstance(formatting["strip_key"], bool):
            fail(f"{suffix}: formatting.strip_key must be boolean")
        redaction = entry["redaction"]
        if not isinstance(redaction, dict) or "mode" not in redaction:
            fail(f"{suffix}: redaction must include mode")
        if not isinstance(entry["constraints"], dict):
            fail(f"{suffix}: constraints must be an object")
    return out


def validate_references(suffixes: list[str]) -> None:
    spec = SPEC.read_text(encoding="utf-8")
    skill = skill_reference_text()
    for suffix in suffixes:
        if "{" in suffix:
            literal = suffix.replace("{code}", "usd")
            if literal not in spec and literal not in skill:
                fail(f"pattern suffix {suffix} has no concrete reference in spec or skill")
            continue
        if suffix not in spec:
            fail(f"{suffix} missing from formal spec")
        if suffix not in skill:
            fail(f"{suffix} missing from Skill text")


def skill_reference_text() -> str:
    validate_skill_frontmatter()
    parts = [SKILL.read_text(encoding="utf-8")]
    for path in sorted((SKILL_DIR / "references").glob("*")):
        if path.is_file() and path.suffix in {".md", ".json"}:
            parts.append(path.read_text(encoding="utf-8"))
    return "\n".join(parts)


def validate_skill_frontmatter() -> None:
    text = SKILL.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        fail("standard Skill must start with YAML frontmatter")
    try:
        _, frontmatter, _ = text.split("---\n", 2)
    except ValueError:
        fail("standard Skill frontmatter must be closed")
    keys = []
    for line in frontmatter.splitlines():
        if not line.strip():
            continue
        if ":" not in line:
            fail(f"invalid Skill frontmatter line: {line!r}")
        keys.append(line.split(":", 1)[0])
    if keys != ["name", "description"]:
        fail(f"standard Skill frontmatter must contain only name and description, got {keys}")


def fixture_keys() -> set[str]:
    keys: set[str] = set()

    def collect(value) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                keys.add(key)
                collect(child)
        elif isinstance(value, list):
            for child in value:
                collect(child)

    for path in sorted(FIXTURES.glob("*.json")):
        collect(json.loads(path.read_text(encoding="utf-8")))
    return keys


def validate_fixture_coverage(suffixes: list[str]) -> None:
    keys = fixture_keys()
    concrete_by_word: dict[str, set[str]] = {}
    for suffix in suffixes:
        if "{" not in suffix and "_" in suffix:
            word = suffix.rsplit("_", 1)[-1]
            concrete_by_word.setdefault(word, set()).add(suffix)
    for suffix in suffixes:
        if "{" in suffix:
            word = suffix.rsplit("_", 1)[-1]  # e.g. "cents" from "_{code}_cents"
            pattern = re.compile(rf"_[A-Za-z]{{3,4}}_{word}$")
            excluded = concrete_by_word.get(word, set())
            if not any(
                pattern.search(key) and not any(key.endswith(e) for e in excluded)
                for key in keys
            ):
                fail(f"pattern suffix {suffix} missing from shared fixtures")
            continue
        if not any(key.endswith(suffix) for key in keys):
            fail(f"{suffix} missing from shared fixtures")


def main() -> None:
    registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
    if not isinstance(registry, dict):
        fail("registry root must be an object")
    require_no_banned_keys(registry)
    suffixes = validate_registry_shape(registry)
    validate_references(suffixes)
    validate_fixture_coverage(suffixes)
    print(f"registry ok: {len(suffixes)} suffix records")


if __name__ == "__main__":
    main()
