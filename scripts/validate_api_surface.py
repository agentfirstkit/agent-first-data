#!/usr/bin/env python3
"""Cross-check the shared AFDATA API surface across all four SDKs.

Reads spec/api-surface.json (the canonical cross-language symbol manifest)
and diffs it against each SDK's real exports:

- A manifest entry naming a symbol that doesn't actually exist in that
  language's exports is a regression (something got deleted/renamed without
  updating the manifest, or vice versa).
- A real exported symbol that isn't declared anywhere in the manifest is an
  undeclared addition (either add it to the manifest deliberately, alongside
  its equivalents in the other three languages, or it's an internal helper
  that shouldn't be exported).

Scope: only the "shared contract" surface (protocol builders/reader, output,
redaction, CLI helpers, core types) — the same groups listed in the README's
"One contract, four languages" section. cli-help, skill-admin, tracing, and
stream-redirect are intentionally out of scope: they are Rust-only tools, not
part of the cross-language contract, so the Go/Python/TypeScript SDKs do not
ship them.

A Rust `pub use` block directly preceded by `#[cfg(feature = "...")]` is
"feature-gated": it's never required to have a manifest entry (the reverse
"undeclared" check ignores it), but if the manifest *does* declare one of its
names — as it does for `output_yaml`/`output_yaml_with_options`, which are
gated behind the on-by-default `yaml` feature but still part of the shared
cross-language contract — that still counts as found.
"""

from __future__ import annotations

import ast
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "spec" / "api-surface.json"

LANGUAGES = ("rust", "python", "typescript", "go")


def fail(messages: list[str]) -> None:
    print("api-surface validation failed:", file=sys.stderr)
    for m in messages:
        print(f"  - {m}", file=sys.stderr)
    raise SystemExit(1)


def extract_rust() -> tuple[set[str], set[str]]:
    """Returns `(always_exported, feature_gated_exported)`.

    `always_exported` backs both directions of the cross-check.
    `feature_gated_exported` (behind a directly-preceding `#[cfg(feature =
    "...")]`) only backs the forward "manifest name must exist" direction —
    it's exempt from the reverse "must be declared" direction, so most
    Rust-only optional-feature symbols never need a manifest entry.
    """
    text = (ROOT / "rust" / "src" / "lib.rs").read_text()
    always: set[str] = set()
    gated: set[str] = set()
    lines = text.splitlines()
    i = 0
    prev_nonblank = ""
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        m = re.match(r"pub use (\w+)::\{", stripped)
        if m:
            block = [stripped]
            while "};" not in block[-1] and "}" not in block[-1].split("::", 1)[-1]:
                i += 1
                block.append(lines[i].strip())
                if "}" in lines[i]:
                    break
            full = " ".join(block)
            inner = full.split("{", 1)[1].rsplit("}", 1)[0]
            target = gated if prev_nonblank.startswith("#[cfg(") else always
            for ident in inner.split(","):
                ident = ident.strip()
                if ident:
                    target.add(ident)
        else:
            # Single-item re-export without braces: `pub use path::name;`
            # (rustfmt collapses one-item brace lists to this form, so the
            # brace matcher above never sees them).
            m_single = re.match(r"pub use (?:\w+::)+(\w+);", stripped)
            if m_single:
                target = gated if prev_nonblank.startswith("#[cfg(") else always
                target.add(m_single.group(1))
        if stripped:
            prev_nonblank = stripped
        i += 1
    return always, gated


def extract_python() -> set[str]:
    text = (ROOT / "python" / "agent_first_data" / "__init__.py").read_text()
    m = re.search(r"__all__\s*=\s*(\[.*?\])", text, re.DOTALL)
    if not m:
        fail(["could not find __all__ in python/agent_first_data/__init__.py"])
    return set(ast.literal_eval(m.group(1)))


def extract_typescript() -> set[str]:
    text = (ROOT / "typescript" / "src" / "index.ts").read_text()
    names: set[str] = set()
    for block in re.findall(r"export \{(.*?)\} from", text, re.DOTALL):
        for entry in block.split(","):
            entry = entry.strip()
            if not entry:
                continue
            entry = re.sub(r"^type\s+", "", entry)
            entry = entry.split(" as ")[0].strip()
            if entry:
                names.add(entry)
    return names


def extract_go() -> set[str]:
    names: set[str] = set()
    for filename in ("afdata.go", "afdata_cli.go", "afdata_decode.go"):
        text = (ROOT / "go" / filename).read_text()
        for m in re.finditer(r"^func (New)?([A-Z]\w*)", text, re.MULTILINE):
            names.add((m.group(1) or "") + m.group(2))
        for m in re.finditer(r"^type ([A-Z]\w*)", text, re.MULTILINE):
            names.add(m.group(1))
    return names


EXTRACTORS = {
    "python": extract_python,
    "typescript": extract_typescript,
    "go": extract_go,
}


def main() -> int:
    manifest = json.loads(MANIFEST.read_text())
    groups = manifest["groups"]

    real: dict[str, set[str]] = {lang: EXTRACTORS[lang]() for lang in EXTRACTORS}
    gated: dict[str, set[str]] = {lang: set() for lang in LANGUAGES}
    real["rust"], gated["rust"] = extract_rust()
    declared: dict[str, set[str]] = {lang: set() for lang in LANGUAGES}

    failures: list[str] = []

    for group in groups:
        gid = group["id"]
        for lang in LANGUAGES:
            name = group.get(lang)
            if name is None:
                continue
            declared[lang].add(name)
            if name not in real[lang] and name not in gated[lang]:
                failures.append(
                    f"{lang}: manifest entry '{gid}' names '{name}', which is not "
                    f"in the SDK's actual exports (renamed, deleted, or manifest is stale)"
                )

    for lang in LANGUAGES:
        undeclared = sorted(real[lang] - declared[lang])
        if undeclared:
            failures.append(
                f"{lang}: exported but not in spec/api-surface.json: {', '.join(undeclared)} "
                f"(add deliberately alongside its equivalents in the other languages, "
                f"or it should not be public)"
            )

    if failures:
        fail(failures)

    print(f"api-surface ok: {len(groups)} shared symbols cross-checked across {len(LANGUAGES)} languages")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
