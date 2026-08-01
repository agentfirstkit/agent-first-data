#!/usr/bin/env python3
"""Hold `spec/protocol-v1.schema.json` to the verdicts the implementation reaches.

`protocol-v1.schema.json` is the machine-readable protocol contract: it is
bundled into every language package and into the installed skill, and it is what
an external consumer validates AFDATA events against. Nothing in this repository
ever validated an event against it. It appeared in exactly two roles — a file
`sync_offline_assets.py` copies, and an asset `test.sh` asserts is packaged — so
it could say anything at all and every gate stayed green. It did: it required
`retryable` on error payloads, which the spec lists as a strict-profile
recommendation, so a base-profile-conformant error event was rejected by the
schema its consumers had been handed.

The gate closes that by judging the same events twice. Every protocol event in
`spec/fixtures/` and every documented example is validated by the schema (base
profile at the document root, strict profile at `#/$defs/strictEvent`) and by
`afdata validate --per-event`, in both profiles, and the two verdicts must agree
event by event. The schema drifting from the spec, and the schema drifting from
the validators, are then the same failure — and it is a loud one.

What the schema cannot express is stream lifecycle, which is per-stream rather
than per-event; `validate_protocol_stream` and the fixtures' own `valid` flags
cover that in the language test suites.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

# Importing a sibling script must not leave a `scripts/__pycache__` behind.
sys.dont_write_bytecode = True

from validate_protocol_docs import (  # noqa: E402  (after the bytecode switch)
    check_against_schema,
    sourced_documented_events,
)

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "spec" / "protocol-v1.schema.json"
FIXTURES = ROOT / "spec" / "fixtures"

KINDS = frozenset({"result", "error", "progress", "log"})

# The document root is the base profile; the strict profile is a named
# definition inside the same document.
PROFILES = (
    ("base", {}, False),
    ("strict", {"$ref": "#/$defs/strictEvent"}, True),
)


def collect(node, where: str, found: list[tuple[str, dict]]) -> None:
    if isinstance(node, dict):
        if node.get("kind") in KINDS:
            found.append((where, node))
        for key, child in node.items():
            collect(child, f"{where}.{key}", found)
    elif isinstance(node, list):
        for index, child in enumerate(node):
            collect(child, f"{where}[{index}]", found)


def fixture_events() -> list[tuple[str, dict]]:
    """Every protocol event anywhere under `spec/fixtures/`.

    Collected by walking the documents rather than by naming the three protocol
    fixture files and their key layouts: a fixture added tomorrow is covered
    without anyone remembering to register it here.
    """
    found: list[tuple[str, dict]] = []
    for path in sorted(FIXTURES.glob("*.json")):
        document = json.loads(path.read_text(encoding="utf-8"))
        collect(document, path.relative_to(ROOT).as_posix(), found)
    return found


def implementation_verdicts(events: list[dict], strict: bool) -> list[str | None]:
    """One verdict per event from `afdata validate`: `None`, or why it failed."""
    with tempfile.TemporaryDirectory() as directory:
        corpus = Path(directory) / "events.json"
        corpus.write_text(json.dumps(events), encoding="utf-8")
        command = [
            "cargo",
            "run",
            "--quiet",
            "--bin",
            "afdata",
            "--",
            "validate",
            str(corpus),
            "--per-event",
            # Findings ride an `error` event, which the default split sends to
            # stderr — where cargo also writes. Pin them to stdout so the
            # verdicts parse regardless of what the build says.
            "--output-to",
            "stdout",
        ]
        if strict:
            command.append("--strict")
        completed = subprocess.run(
            command, cwd=ROOT, capture_output=True, text=True, encoding="utf-8"
        )
    try:
        event = json.loads(completed.stdout)
    except json.JSONDecodeError:
        raise SystemExit(
            f"afdata validate produced no event (exit {completed.returncode}):\n"
            f"{completed.stderr.strip()}"
        ) from None
    payload = event.get("error") if event.get("kind") == "error" else event.get("result")
    verdicts: list[str | None] = [None] * len(events)
    for finding in (payload or {}).get("findings", []):
        if finding.get("severity") != "error":
            continue
        index = int(finding["pointer"].split("/")[1])
        if verdicts[index] is None:
            verdicts[index] = f"{finding['rule_id']}: {finding['message']}"
    return verdicts


def main() -> int:
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    fixtures = fixture_events()
    documented = sourced_documented_events()
    corpus = fixtures + documented
    if not corpus:
        print("no protocol events found to validate", file=sys.stderr)
        return 1

    failures: list[str] = []
    rejections = 0
    for profile, selector, strict in PROFILES:
        verdicts = implementation_verdicts([event for _, event in corpus], strict)
        rejections += sum(verdict is not None for verdict in verdicts)
        for (where, event), verdict in zip(corpus, verdicts):
            problems = check_against_schema(event, selector or schema, schema, "event")
            body = json.dumps(event)
            if problems and verdict is None:
                failures.append(
                    f"{where}: the {profile} profile accepts this event, "
                    f"protocol-v1.schema.json rejects it\n    {body}\n"
                    + "\n".join(f"    {problem}" for problem in problems)
                )
            elif not problems and verdict is not None:
                failures.append(
                    f"{where}: the {profile} profile rejects this event "
                    f"({verdict}), protocol-v1.schema.json accepts it\n    {body}"
                )

    kinds = {event["kind"] for _, event in corpus}
    if kinds != KINDS:
        failures.append(
            f"the corpus covers {sorted(kinds)}; every event kind must be covered "
            f"or the schema's dispatch is untested for {sorted(KINDS - kinds)}"
        )
    if not rejections:
        # Agreement is only worth something when the corpus contains events the
        # implementation turns down. Without one, a schema that accepts
        # everything would pass this gate.
        failures.append(
            "no event in the corpus is rejected by afdata validate, so this gate "
            "proves only that the schema accepts valid events"
        )

    if failures:
        print("protocol-v1.schema.json disagrees with the implementation:\n", file=sys.stderr)
        print("\n\n".join(f"  - {failure}" for failure in failures), file=sys.stderr)
        return 1

    print(
        f"protocol-v1 schema ok: {len(corpus)} events "
        f"({len(fixtures)} fixture, {len(documented)} documented) "
        f"agree with afdata validate in both profiles"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
