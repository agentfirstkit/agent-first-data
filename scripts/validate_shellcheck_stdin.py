#!/usr/bin/env python3
"""Prove the ShellCheck gate never inherits an indefinitely open stdin.

`shellcheck -x` follows a `source=/dev/stdin` directive by opening that path.
This repo's scripts use exactly that directive to load the Bash kit that
`afdata shell bash` writes at runtime, so an analyzer handed a stdin that never
reaches EOF waits on it forever — silently, with no output and no timeout. CI
happens to hand the gate a closed stdin, so the failure only appears on a
developer's terminal, where it reads as "the tests hang".

The check runs the gate against `scripts/fixtures/shellcheck-open-stdin.sh`
with a pipe held open and requires it to finish within a deadline.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
INSTALLER = SCRIPT_DIR / "install-shellcheck.sh"
FIXTURE_DIR = SCRIPT_DIR / "fixtures"
DEADLINE_S = 20


def main() -> int:
    if not INSTALLER.is_file():
        print(f"missing {INSTALLER}", file=sys.stderr)
        return 1
    if not (FIXTURE_DIR / "shellcheck-open-stdin.sh").is_file():
        print(f"missing the open-stdin fixture under {FIXTURE_DIR}", file=sys.stderr)
        return 1

    # A pipe nobody ever writes to or closes: an interactive terminal's shape,
    # not a redirect the gate could accidentally satisfy.
    process = subprocess.Popen(
        [str(INSTALLER), "--check", str(FIXTURE_DIR)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        status = process.wait(timeout=DEADLINE_S)
    except subprocess.TimeoutExpired:
        process.kill()
        process.communicate()
        print(
            "the ShellCheck gate inherited an open stdin and did not finish "
            f"within {DEADLINE_S}s",
            file=sys.stderr,
        )
        return 1

    stdout, stderr = process.communicate()
    if status != 0:
        print(f"the ShellCheck gate exited {status} on its own fixture", file=sys.stderr)
        sys.stderr.write(stdout)
        sys.stderr.write(stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
