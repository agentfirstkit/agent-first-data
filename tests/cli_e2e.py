#!/usr/bin/env python3
"""End-to-end checks for the four canonical agent-first-data CLI examples."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class CliCase:
    name: str
    cwd: Path
    command_prefix: tuple[str, ...]
    success_args: tuple[str, ...]


def cli_cases() -> list[CliCase]:
    pytool = os.environ.get("AFDATA_PYTOOL", sys.executable)
    return [
        CliCase(
            name="rust",
            cwd=ROOT,
            command_prefix=(
                "cargo",
                "run",
                "--quiet",
                "--example",
                "agent_cli",
                "--features",
                "cli-help,cli-help-markdown",
                "--",
            ),
            success_args=("ping", "--host", "example.com"),
        ),
        CliCase(
            name="go",
            cwd=ROOT / "go",
            command_prefix=("go", "run", "./examples/agent_cli"),
            success_args=("echo",),
        ),
        CliCase(
            name="python",
            cwd=ROOT / "python",
            command_prefix=(pytool, "examples/agent_cli.py"),
            success_args=("echo",),
        ),
        CliCase(
            name="typescript",
            cwd=ROOT / "typescript",
            command_prefix=("npx", "tsx", "examples/agent_cli.ts"),
            success_args=("echo",),
        ),
    ]


def run_cli(case: CliCase, args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    if case.name == "python":
        env["PYTHONPATH"] = "."
    return subprocess.run(
        [*case.command_prefix, *args],
        cwd=case.cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )


def run_afdata(args: Sequence[str], stdin: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "run", "--quiet", "--bin", "afdata", "--", *args],
        cwd=ROOT,
        text=True,
        input=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )


def run_afdata_skill(args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "run", "--quiet", "--features", "skill-admin", "--bin", "afdata", "--", *args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )


def parse_events(stdout: str) -> list[dict[str, object]]:
    lines = [line for line in stdout.splitlines() if line.strip()]
    return [json.loads(line) for line in lines]


def assert_single_terminal(case: CliCase) -> None:
    proc = run_cli(case, case.success_args)
    assert proc.returncode == 0, f"{case.name}: success returned {proc.returncode}, stderr={proc.stderr!r}"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one event, got {events!r}"
    assert events[0]["kind"] == "result", f"{case.name}: expected result, got {events[0]!r}"


def assert_startup_log(case: CliCase) -> None:
    proc = run_cli(case, ("--log", "startup", *case.success_args))
    assert proc.returncode == 0, f"{case.name}: startup log returned {proc.returncode}, stderr={proc.stderr!r}"
    events = parse_events(proc.stdout)
    assert len(events) == 2, f"{case.name}: expected log + terminal, got {events!r}"
    assert events[0]["kind"] == "log", f"{case.name}: first event not log: {events!r}"
    log = events[0]["log"]
    assert isinstance(log, dict), f"{case.name}: log payload not object: {events[0]!r}"
    assert log.get("category") == "startup", f"{case.name}: startup category missing: {events[0]!r}"
    assert events[1]["kind"] == "result", f"{case.name}: terminal event not result: {events!r}"


def assert_unknown_arg(case: CliCase) -> None:
    proc = run_cli(case, ("--unknown", *case.success_args))
    assert proc.returncode != 0, f"{case.name}: unknown arg unexpectedly succeeded"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one fallback error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected error, got {events[0]!r}"


def assert_invalid_output_fallback(case: CliCase) -> None:
    proc = run_cli(case, ("--output", "xml", *case.success_args))
    assert proc.returncode != 0, f"{case.name}: invalid output unexpectedly succeeded"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one JSON fallback error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected error fallback, got {events[0]!r}"


def assert_json_alias(case: CliCase) -> None:
    proc = run_cli(case, ("--json", *case.success_args))
    assert proc.returncode == 0, f"{case.name}: --json returned {proc.returncode}, stderr={proc.stderr!r}"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one JSON event, got {events!r}"
    assert events[0]["kind"] == "result", f"{case.name}: --json did not emit result: {events[0]!r}"


def assert_format_conflict_fallback(case: CliCase) -> None:
    proc = run_cli(case, ("--json", "--output", "plain", *case.success_args))
    assert proc.returncode != 0, f"{case.name}: output conflict unexpectedly succeeded"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one conflict error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected conflict error, got {events[0]!r}"


def assert_cancelled(case: CliCase) -> None:
    proc = run_cli(case, ("cancel",))
    assert proc.returncode != 0, f"{case.name}: cancellation unexpectedly succeeded"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"{case.name}: expected one cancellation event, got {events!r}"
    event = events[0]
    assert event["kind"] == "error", f"{case.name}: cancellation did not emit error: {event!r}"
    error = event["error"]
    assert isinstance(error, dict), f"{case.name}: error payload not object: {event!r}"
    assert error.get("code") == "cancelled", f"{case.name}: wrong cancellation code: {event!r}"


def assert_broken_pipe_no_traceback(case: CliCase) -> None:
    env = os.environ.copy()
    if case.name == "python":
        env["PYTHONPATH"] = "."
    proc = subprocess.Popen(
        [*case.command_prefix, "--log", "startup", *case.success_args],
        cwd=case.cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert proc.stdout is not None
    assert proc.stderr is not None
    proc.stdout.close()
    stderr = proc.stderr.read()
    proc.wait(timeout=60)
    lowered = stderr.lower()
    forbidden = ("panic", "traceback", "stack backtrace", "brokenpipeerror", "epipe")
    assert not any(token in lowered for token in forbidden), (
        f"{case.name}: broken pipe leaked panic/traceback diagnostics: {stderr!r}"
    )


def assert_afdata_validate() -> None:
    proc = run_afdata(("validate",), '{"kind":"result","result":{"ok":true}}\n')
    assert proc.returncode == 0, f"afdata validate failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"afdata validate emitted unexpected events: {events!r}"
    assert events[0]["kind"] == "result", f"afdata validate did not emit result: {events[0]!r}"
    assert events[0]["trace"] == {}, f"afdata validate result is not strict: {events[0]!r}"


def assert_afdata_validate_strict_event() -> None:
    valid = '{"kind":"log","log":{"event":"startup"},"trace":{}}\n'
    proc = run_afdata(("validate", "--strict", "--event"), valid)
    assert proc.returncode == 0, f"strict event failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = parse_events(proc.stdout)
    assert events[0]["kind"] == "result", f"strict event result missing: {events!r}"

    invalid = '{"kind":"log","log":{"event":"startup"}}\n'
    proc = run_afdata(("validate", "--strict", "--event"), invalid)
    assert proc.returncode != 0, "strict event accepted an event without trace"
    events = parse_events(proc.stdout)
    assert events[0]["kind"] == "error", f"strict event error missing: {events!r}"
    assert events[0]["trace"] == {}, f"strict event error is not strict: {events[0]!r}"


def assert_afdata_validate_stream_error() -> None:
    proc = run_afdata(("validate",), '{"kind":"log","log":{"event":"startup"}}\n')
    assert proc.returncode != 0, "afdata validate accepted a stream without terminal event"
    events = parse_events(proc.stdout)
    assert events[0]["kind"] == "error", f"afdata validate stream error missing: {events!r}"
    assert events[0]["error"]["code"] == "validation_failed", f"wrong validation code: {events[0]!r}"


def assert_afdata_lint_schema_secret() -> None:
    schema = '{"type":"object","properties":{"api_key_secret":{"type":"string","default":"sk-live"}}}\n'
    proc = run_afdata(("lint",), schema)
    assert proc.returncode != 0, "afdata lint accepted exposed secret default"
    events = parse_events(proc.stdout)
    assert events[0]["kind"] == "error", f"afdata lint error missing: {events!r}"
    findings = events[0]["error"]["findings"]
    assert findings[0]["rule_id"] == "secret_schema_value_exposed", f"wrong lint finding: {findings!r}"


def assert_afdata_format_redacts() -> None:
    proc = run_afdata(("format", "--output", "json"), '{"api_key_secret":"sk-live","ok":true}\n')
    assert proc.returncode == 0, f"afdata format failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    value = json.loads(proc.stdout)
    assert value["api_key_secret"] == "***", f"afdata format did not redact: {value!r}"
    assert value["ok"] is True, f"afdata format changed non-secret value: {value!r}"


def assert_afdata_parse_error() -> None:
    proc = run_afdata(("format",), '{"ok":true}\nnot-json\n')
    assert proc.returncode != 0, "afdata format accepted invalid JSONL"
    events = parse_events(proc.stdout)
    assert events[0]["kind"] == "error", f"afdata parse error missing: {events!r}"
    assert events[0]["error"]["code"] == "jsonl_parse_failed", f"wrong parse code: {events[0]!r}"


def assert_afdata_skill_status_feature() -> None:
    with tempfile.TemporaryDirectory(prefix="afdata-skill-e2e-") as tmp:
        proc = run_afdata_skill(("skill", "status", "--agent", "codex", "--skills-dir", tmp))
    assert proc.returncode == 0, f"afdata skill status failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"afdata skill status emitted unexpected events: {events!r}"
    assert events[0]["kind"] == "result", f"skill status not a result event: {events[0]!r}"
    result = events[0]["result"]
    assert result["code"] == "skill_status", f"wrong skill status code: {result!r}"
    assert result["skill"] == "agent-first-data", f"wrong skill name: {result!r}"


def assert_afdata_skill_error_includes_partial_report() -> None:
    with tempfile.TemporaryDirectory(prefix="afdata-skill-error-e2e-") as tmp:
        skill_dir = Path(tmp) / "agent-first-data"
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(
            "---\nname: custom\ndescription: custom\n---\n", encoding="utf-8"
        )
        proc = run_afdata_skill(("skill", "install", "--agent", "codex", "--skills-dir", tmp))
    assert proc.returncode != 0, "afdata skill install overwrote unmanaged skill without --force"
    events = parse_events(proc.stdout)
    assert len(events) == 1, f"afdata skill install emitted unexpected events: {events!r}"
    error = events[0]["error"]
    assert error["code"] == "cli_error", f"wrong skill error code: {events[0]!r}"
    report = error["partial_report"]
    assert report["code"] == "skill_install", f"wrong partial report code: {report!r}"
    assert report["installed"] is False, f"partial install report should be failed: {report!r}"
    assert report["targets"][0]["installed"] is True, f"partial report lost target status: {report!r}"
    assert report["targets"][0]["managed"] is False, f"unmanaged target should be reported: {report!r}"


def assert_afdata_skill_help_is_feature_gated() -> None:
    default_help = run_afdata(("--help",), "")
    assert default_help.returncode == 0, f"default afdata help failed: {default_help.stderr!r}"
    assert "skill" not in default_help.stdout, "default afdata help must not show skill subcommand"
    feature_help = run_afdata_skill(("--help",))
    assert feature_help.returncode == 0, f"feature afdata help failed: {feature_help.stderr!r}"
    assert "skill" in feature_help.stdout, "feature afdata help must show skill subcommand"


def main() -> None:
    checks = (
        assert_single_terminal,
        assert_startup_log,
        assert_unknown_arg,
        assert_invalid_output_fallback,
        assert_json_alias,
        assert_format_conflict_fallback,
        assert_cancelled,
        assert_broken_pipe_no_traceback,
    )
    for case in cli_cases():
        for check in checks:
            check(case)
        print(f"[e2e] {case.name}: ok")
    for check in (
        assert_afdata_validate,
        assert_afdata_validate_strict_event,
        assert_afdata_validate_stream_error,
        assert_afdata_lint_schema_secret,
        assert_afdata_format_redacts,
        assert_afdata_parse_error,
        assert_afdata_skill_status_feature,
        assert_afdata_skill_error_includes_partial_report,
        assert_afdata_skill_help_is_feature_gated,
    ):
        check()
    print("[e2e] afdata: ok")


if __name__ == "__main__":
    main()
