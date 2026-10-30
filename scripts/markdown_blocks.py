"""Code blocks of a Markdown file, read through this spore's own afdata.

The doc checks need every fenced or indented code block, wherever it sits: at
the top level, inside a list item, inside a blockquote. Deciding what counts as
a code block is CommonMark's job, and `afdata --input-format markdown` already
does it, so these checks use that reading instead of a scanner of their own.
The binary is the one `scripts/test.sh` builds before running them, so the
tree is judged by its own code rather than by an installed release.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AFDATA = ROOT / "target" / "debug" / "afdata"
SECTION_KEYS = tuple(f"h{level}" for level in range(1, 7))


def code_blocks(path: Path) -> list[dict]:
    """Every code block in `path`, in source order.

    Each item is afdata's block object: `language`, `text`, and the 1-based
    `source_start_line` / `source_end_line`.
    """
    run = subprocess.run(
        [str(AFDATA), "get", str(path), "--input-format", "markdown", "--output", "json"],
        capture_output=True,
        encoding="utf-8",
        check=False,
    )
    if run.returncode != 0:
        raise RuntimeError(f"afdata could not read {path}: {run.stderr.strip()[:300]}")
    tree = json.loads(run.stdout)["result"]["value"]
    found: list[dict] = []
    _collect(tree["preamble"], found)
    _collect(tree, found)
    return sorted(found, key=lambda block: block["source_start_line"])


def _collect(container: dict, found: list[dict]) -> None:
    # Only `blocks` and child sections: the `paragraph` and `blockquote` views
    # repeat blocks already listed, and `inlines` holds code *spans*.
    for block in container.get("blocks", []):
        if block["type"] == "code":
            found.append(block)
        _collect(block, found)
    for key in SECTION_KEYS:
        for section in container.get(key, []):
            _collect(section, found)
