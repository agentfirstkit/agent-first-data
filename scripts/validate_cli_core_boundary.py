#!/usr/bin/env python3
"""Enforce the dependency direction between the CLI core and AFDATA.

`rust/src/cli_spec/` is the closed-world CLI compiler: it decides whether an
invocation is legal and what it resolved to. `rust/src/cli_afdata.rs` is the
only place that expresses such an outcome as an AFDATA protocol event.

Keeping that direction one-way is what makes the core extractable into its own
crate later without a redesign, so it is checked rather than trusted:

- nothing under `cli_spec/` may reference `crate::` — no protocol events, no
  emitter, no rendering, no redaction — nor escape upwards via `super::super::`,
  which resolves to the same crate root;
- nothing under `cli_spec/` may name `serde_json::Value` in its API. AFDATA
  turns on `serde_json/arbitrary_precision`, and cargo unifies features across
  a whole binary, so a parsed value's number semantics would depend on what
  else happens to be linked. Raw JSON argument values stay source text;
- nothing under `cli_spec/` may touch process I/O — resolving argv must not
  print, read files, or install sinks;
- nothing under `cli_spec/` may hardcode AFDATA's `_secret` naming convention;
  the core carries an opaque `sensitive` bit and the adapter owns the policy.

The complementary direction is checked by the build itself: `scripts/test.sh
static` builds `--no-default-features`, which links neither the compiler nor
its adapter and so only succeeds while the rest of the crate depends on
neither.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "rust" / "src" / "cli_spec"
ADAPTER = ROOT / "rust" / "src" / "cli_afdata.rs"

# (regex, why it is banned in the core)
BANNED = [
    (r"\bcrate::", "references another AFDATA module"),
    (r"\bsuper::super::", "escapes to the crate root, bypassing the crate:: ban"),
    (r"\bserde_json::Value\b", "names serde_json::Value (feature-unification hazard)"),
    (r"\bstd::io\b", "performs process I/O"),
    (r"\bstd::fs\b", "touches the filesystem"),
    (r"\bstd::process\b", "touches the process"),
    (r"\b(?:e)?print(?:ln)?!", "writes to a standard stream"),
    (r"_secret", "hardcodes AFDATA's `_secret` naming convention"),
]


def code_lines(path: Path) -> list[tuple[int, str]]:
    """Source lines with comments stripped, so prose may discuss the rules."""
    out: list[tuple[int, str]] = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = raw.strip()
        if stripped.startswith("//"):
            continue
        out.append((number, raw.split("//", 1)[0]))
    return out


def main() -> int:
    failures: list[str] = []

    if not CORE.is_dir():
        failures.append(f"missing CLI core directory: {CORE.relative_to(ROOT)}")
    if not ADAPTER.is_file():
        failures.append(f"missing AFDATA adapter: {ADAPTER.relative_to(ROOT)}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1

    for path in sorted(CORE.rglob("*.rs")):
        rel = path.relative_to(ROOT)
        for number, line in code_lines(path):
            for pattern, why in BANNED:
                if re.search(pattern, line):
                    failures.append(f"{rel}:{number}: {why}\n    {line.strip()}")

    adapter = ADAPTER.read_text(encoding="utf-8")
    if "crate::cli_spec" not in adapter or "crate::protocol" not in adapter:
        failures.append(
            "rust/src/cli_afdata.rs: the adapter must bridge crate::cli_spec and "
            "crate::protocol; if it no longer does, the boundary moved"
        )

    if failures:
        print("CLI core boundary violations:\n", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1

    checked = len(list(CORE.rglob("*.rs")))
    print(f"CLI core boundary OK ({checked} files under rust/src/cli_spec/)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
