"""Policy test: runtime library sources must not emit protocol/log events to stderr."""

from pathlib import Path
import re


DISALLOWED = re.compile(
    r"\bsys\.stderr\b|\bfile\s*=\s*sys\.stderr\b|\bstderr\.write\s*\(",
)


def test_no_stderr_usage_in_runtime_sources() -> None:
    root = Path(__file__).resolve().parents[1] / "agent_first_data"
    files = sorted(root.glob("*.py"))
    assert files, "no python source files found"

    violations: list[str] = []
    for path in files:
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            if DISALLOWED.search(line):
                violations.append(f"{path.name}:{lineno}: {line.strip()}")

    assert not violations, "stderr usage is disallowed:\n" + "\n".join(violations)
