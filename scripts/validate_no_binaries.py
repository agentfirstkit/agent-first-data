#!/usr/bin/env python3
"""Fail when a build artifact is tracked in the package.

A release stages the whole spore tree, so anything tracked ships to crates.io,
npm, and PyPI. `go build` inside an example directory writes a binary beside its
source, and a stray `python3` run writes `__pycache__`; both have reached a
published release this way. Detect them from content, not from a name list.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Binary formats that are legitimately part of a package.
ALLOWED_SUFFIXES = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".webp",
    ".pdf",
    ".woff",
    ".woff2",
    ".ttf",
    ".otf",
}

# A NUL byte in the first block is the same heuristic `git diff` uses.
PROBE_BYTES = 8000


def tracked_files() -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return [ROOT / name for name in out.split("\0") if name]


def main() -> int:
    tracked = tracked_files()
    offenders = []
    for path in tracked:
        if path.suffix.lower() in ALLOWED_SUFFIXES or not path.is_file():
            continue
        try:
            with path.open("rb") as handle:
                chunk = handle.read(PROBE_BYTES)
        except OSError:
            continue
        # An empty file is not an artifact; an empty __init__.py is normal.
        if chunk and b"\0" in chunk:
            offenders.append(path.relative_to(ROOT))

    if offenders:
        print("Binary files are tracked in the package:", file=sys.stderr)
        for path in sorted(offenders):
            print(f"  {path}", file=sys.stderr)
        print(
            "Remove them and add an ignore rule; a release would publish them.",
            file=sys.stderr,
        )
        return 1
    print(f"no tracked binaries: {len(tracked)} files checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
