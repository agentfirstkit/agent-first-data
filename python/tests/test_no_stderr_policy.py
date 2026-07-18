"""Policy test: runtime sources must not write to stderr outside the emitter's sink.

The spec's CLI Event Framing sanctions one path to stderr — the emitter's own
formatted diagnostic sink (``CliEmitter.finite`` / ``CliEmitter.from_output_to``
hand ``sys.stderr`` to the emitter, which routes ``error``/``progress``/``log``
through it). Merely referencing ``sys.stderr`` to pass it as that sink is allowed.
What stays forbidden is *ad-hoc* stderr writes that bypass the formatter:
``sys.stderr.write(...)`` / ``.writelines(...)`` and ``print(..., file=sys.stderr)``.
"""

from pathlib import Path
import re


# Forbid ad-hoc stderr writes that bypass the emitter's formatted sink. A bare
# `sys.stderr` reference (handing the stream to CliEmitter as its diagnostic
# sink) is the sanctioned exception and intentionally not matched here.
DISALLOWED = re.compile(
    r"\bfile\s*=\s*sys\.stderr\b"          # print(..., file=sys.stderr)
    r"|\bstderr\.(?:write|writelines)\s*\(",  # sys.stderr.write(...) / .writelines(...)
)


def test_no_ad_hoc_stderr_writes_in_runtime_sources() -> None:
    root = Path(__file__).resolve().parents[1] / "agent_first_data"
    files = sorted(root.glob("*.py"))
    assert files, "no python source files found"

    violations: list[str] = []
    for path in files:
        if path.name == "stream_redirect.py":
            continue
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            if DISALLOWED.search(line):
                violations.append(f"{path.name}:{lineno}: {line.strip()}")

    assert not violations, "ad-hoc stderr writes are disallowed:\n" + "\n".join(violations)


def test_policy_permits_emitter_sink_but_forbids_ad_hoc_writes() -> None:
    # Sanctioned: handing sys.stderr to the emitter as its diagnostic sink.
    assert not DISALLOWED.search("return cls.finite_with(sys.stdout, sys.stderr, format)")
    assert not DISALLOWED.search("return cls.stream(sys.stderr, format, output_options, log_fields)")
    assert not DISALLOWED.search("CliEmitter.from_output_to(OutputTo.SPLIT, fmt)")
    # Forbidden: ad-hoc stderr writes that bypass the emitter.
    assert DISALLOWED.search("sys.stderr.write('boom')")
    assert DISALLOWED.search("sys.stderr.writelines(lines)")
    assert DISALLOWED.search("print('boom', file=sys.stderr)")
