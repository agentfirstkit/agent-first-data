#!/usr/bin/env python3
"""Run the `afdata` commands the docs show, against the files they name.

A documented command that fails is worse than no example: the skill reference
is what an agent reads before its first attempt, so a broken line there is a
failure handed to every reader. This caught `h1.0.h2.install.text`, which the
README taught while its own two `## Install ...` headings made it ambiguous.

Only self-contained read commands are run — those naming a file that exists
relative to the repo root, with no shell plumbing and no mutation.
"""

from __future__ import annotations

import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Optional

ROOT = Path(__file__).resolve().parent.parent
# Everything an agent or a reader is handed. The skill files are the first thing
# read before a first attempt, and docs/ is what the README points at; a broken
# line in any of them is a failure handed to every reader, so none of them get
# to sit outside the check. docs/cli.md is absent on purpose: it is generated
# from the CLI spec and compared byte-for-byte against `afdata --docs` in
# scripts/test.sh, which is a stronger guarantee than running its examples.
SOURCES = [
    ROOT / "README.md",
    ROOT / "docs/bash.md",
    ROOT / "docs/protocol-v1.md",
    ROOT / "docs/transport-mappings.md",
    ROOT / "skills/agent-first-data/SKILL.md",
    ROOT / "skills/agent-first-data/references/bash.md",
    ROOT / "skills/agent-first-data/references/cli-protocol.md",
    ROOT / "skills/agent-first-data/references/documents.md",
    ROOT / "skills/agent-first-data/references/naming-output.md",
]
# The sources that teach document addressing by example, and so must never
# silently stop being checked: a reformat that made nothing match would
# otherwise report success while running nothing. The rest are scanned
# opportunistically — they are about the Bash kit or the protocol, and their
# examples are shell functions, generic filenames, and pipelines rather than
# self-contained reads. Requiring a runnable line from them would only push
# examples into them for the checker's benefit.
MUST_HAVE_EXAMPLES = {
    ROOT / "README.md",
    ROOT / "skills/agent-first-data/references/documents.md",
}
# Read verbs only: a doc example must never mutate the tree it is checked in.
READ_VERBS = {"get", "value", "values", "keys", "paths", "lint", "render", "validate"}
FENCE_OPEN = re.compile(r"^ {0,3}(?P<fence>`{3,}|~{3,})(?P<info>.*)$")


def fenced_blocks(text: str) -> list[str]:
    """Extract CommonMark-style backtick and tilde fenced code blocks."""
    blocks = []
    block: Optional[list[str]] = None
    fence_char = ""
    fence_width = 0

    for line in text.splitlines():
        if block is None:
            match = FENCE_OPEN.fullmatch(line)
            if not match:
                continue
            fence = match.group("fence")
            info = match.group("info")
            # CommonMark forbids a backtick in a backtick fence's info string.
            if fence[0] == "`" and "`" in info:
                continue
            block = []
            fence_char = fence[0]
            fence_width = len(fence)
            continue

        candidate = line.lstrip(" ")
        indent = len(line) - len(candidate)
        candidate = candidate.rstrip(" \t")
        if (
            indent <= 3
            and len(candidate) >= fence_width
            and all(char == fence_char for char in candidate)
        ):
            blocks.append("\n".join(block))
            block = None
            fence_char = ""
            fence_width = 0
        else:
            block.append(line)

    # An unclosed fence continues to EOF in CommonMark.
    if block is not None:
        blocks.append("\n".join(block))
    return blocks


def examples(text: str) -> list[str]:
    found = []
    for block in fenced_blocks(text):
        # Join backslash continuations first. Docs wrap a long command across
        # lines to stay readable, and treating each physical line separately
        # skipped exactly those — the longest, most easily broken examples, and
        # the ones this check most needs to run.
        joined: list[str] = []
        for line in block.splitlines():
            if joined and joined[-1].endswith("\\"):
                joined[-1] = joined[-1][:-1].rstrip() + " " + line.strip()
            else:
                joined.append(line.strip())
        for line in joined:
            if not line.startswith("afdata ") or line.endswith("\\"):
                continue
            # Skip anything needing a shell: pipes, substitution, redirection.
            if any(ch in line for ch in "|$><&"):
                continue
            found.append(line)
    return found


def main() -> int:
    failures = []
    counts: list[str] = []
    checked = 0
    for source in SOURCES:
        if not source.is_file():
            failures.append(f"{source.relative_to(ROOT)}: source file is missing")
            continue
        source_checked = 0
        for line in examples(source.read_text(encoding="utf-8")):
            # `comments=True` drops the trailing `# what this shows` the docs
            # put on each example; without it the comment words arrive as
            # positional arguments and every line "fails" for the wrong reason.
            try:
                argv = shlex.split(line, comments=True)
            except ValueError:
                continue
            if len(argv) < 2 or argv[1] not in READ_VERBS:
                continue
            # The example must name a file that exists here, or it is generic.
            targets = [a for a in argv[2:] if not a.startswith("-") and Path(ROOT / a).is_file()]
            if not targets:
                continue
            checked += 1
            source_checked += 1
            run = subprocess.run(
                [str(ROOT / "target/debug/afdata")] + argv[1:],
                cwd=ROOT,
                capture_output=True,
                encoding="utf-8",
            )
            if run.returncode != 0:
                failures.append(
                    f"{source.relative_to(ROOT)}: `{line}`\n    -> {run.stderr.strip()[:200]}"
                )
        if source_checked == 0 and source in MUST_HAVE_EXAMPLES:
            failures.append(
                f"{source.relative_to(ROOT)}: no runnable read examples found"
            )
        counts.append(f"{source.relative_to(ROOT)}: {source_checked}")

    if failures:
        print("documented examples that do not run:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    # Per-source counts, so a source quietly dropping to zero is visible in the
    # gate output rather than hidden inside a healthy-looking total.
    print(f"doc examples ok: {checked} runnable commands execute cleanly")
    print(f"  ({'; '.join(counts)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
