#!/usr/bin/env python3
"""End-to-end checks for the four canonical agent-first-data CLI examples."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from scripts.validate_cli_help import validate_help_event


@dataclass(frozen=True)
class CliCase:
    name: str
    cwd: Path
    command_prefix: tuple[str, ...]
    success_args: tuple[str, ...]


def cli_cases() -> list[CliCase]:
    pytool = os.environ.get("AFDATA_PYTOOL", sys.executable)
    npx = shutil.which("npx") or "npx"
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
            command_prefix=(npx, "tsx", "examples/agent_cli.ts"),
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


def run_afdata_minimal(args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    """Run the afdata binary built with only the core CLI (default features off)."""
    return subprocess.run(
        [
            "cargo", "run", "--quiet",
            "--no-default-features", "--features", "cli",
            "--bin", "afdata", "--", *args,
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=120,
        check=False,
    )


def parse_events(stdout: str) -> list[dict[str, object]]:
    # `go run` appends its own "exit status N" line to stderr when the program
    # exits non-zero. That is a go-run wrapper artifact, not part of the
    # program's AFDATA output, so drop it before parsing. (Rust/afdata use
    # `cargo run --quiet`, which suppresses the equivalent trailer.)
    lines = [
        line
        for line in stdout.splitlines()
        if line.strip() and not line.strip().startswith("exit status ")
    ]
    return [json.loads(line) for line in lines]


def terminal_events(proc: subprocess.CompletedProcess[str]) -> list[dict[str, object]]:
    """The terminal event(s) from the stream a finite CLI used under the default
    split: `result` on stdout (exit 0), `error` on stderr (non-zero exit).
    Diagnostics (`log`/`progress`) go to stderr regardless — parse `proc.stderr`
    directly for those."""
    return parse_events(proc.stdout if proc.returncode == 0 else proc.stderr)


def assert_single_terminal(case: CliCase) -> None:
    proc = run_cli(case, case.success_args)
    assert proc.returncode == 0, f"{case.name}: success returned {proc.returncode}, stderr={proc.stderr!r}"
    events = terminal_events(proc)
    assert len(events) == 1, f"{case.name}: expected one event, got {events!r}"
    assert events[0]["kind"] == "result", f"{case.name}: expected result, got {events[0]!r}"


def assert_startup_log(case: CliCase) -> None:
    proc = run_cli(case, ("--log", "startup", *case.success_args))
    assert proc.returncode == 0, f"{case.name}: startup log returned {proc.returncode}, stderr={proc.stderr!r}"
    # Under the finite split, the terminal `result` is on stdout while the
    # startup `log` (a diagnostic) is on stderr.
    results = parse_events(proc.stdout)
    assert len(results) == 1 and results[0]["kind"] == "result", (
        f"{case.name}: expected one result on stdout, got {results!r}"
    )
    logs = parse_events(proc.stderr)
    assert any(
        e["kind"] == "log"
        and isinstance(e.get("log"), dict)
        and e["log"].get("category") == "startup"
        for e in logs
    ), f"{case.name}: startup log missing from stderr: {logs!r}"


def assert_unknown_arg(case: CliCase) -> None:
    proc = run_cli(case, ("--unknown", *case.success_args))
    assert proc.returncode != 0, f"{case.name}: unknown arg unexpectedly succeeded"
    events = terminal_events(proc)
    assert len(events) == 1, f"{case.name}: expected one fallback error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected error, got {events[0]!r}"


def assert_invalid_output_fallback(case: CliCase) -> None:
    proc = run_cli(case, ("--output", "xml", *case.success_args))
    assert proc.returncode != 0, f"{case.name}: invalid output unexpectedly succeeded"
    events = terminal_events(proc)
    assert len(events) == 1, f"{case.name}: expected one JSON fallback error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected error fallback, got {events[0]!r}"


def assert_json_flag_is_not_afdata_s(case: CliCase) -> None:
    # `--json` is a plain application flag now. These examples do not declare
    # one, so it must surface as an unknown argument rather than being silently
    # consumed as a second spelling of `--output json`.
    proc = run_cli(case, ("--json", *case.success_args))
    assert proc.returncode != 0, (
        f"{case.name}: --json was consumed by AFDATA instead of reaching the app"
    )
    events = terminal_events(proc)
    assert len(events) == 1, f"{case.name}: expected one error, got {events!r}"
    assert events[0]["kind"] == "error", f"{case.name}: expected error, got {events[0]!r}"


def assert_cancelled(case: CliCase) -> None:
    proc = run_cli(case, ("cancel",))
    assert proc.returncode != 0, f"{case.name}: cancellation unexpectedly succeeded"
    events = terminal_events(proc)
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


def assert_version(case: CliCase) -> None:
    # Parity check: across all four SDKs `--version` is always a structured
    # protocol-v1 version event. These examples all declare JSON as their normal
    # output default; an explicit `--output` still wins.
    proc = run_cli(case, ("--version",))
    assert proc.returncode == 0, f"{case.name}: --version returned {proc.returncode}, stderr={proc.stderr!r}"
    events = terminal_events(proc)
    assert len(events) == 1, f"{case.name}: expected one version event, got {events!r}"
    event = events[0]
    assert event["kind"] == "result", f"{case.name}: --version did not emit a result: {event!r}"
    result = event["result"]
    assert result["code"] == "version", f"{case.name}: --version result code is not 'version': {event!r}"
    assert result["name"], f"{case.name}: --version missing result.name: {event!r}"
    assert result["version"], f"{case.name}: --version missing result.version: {event!r}"
    plain = run_cli(case, ("--version", "--output", "plain"))
    assert plain.returncode == 0, f"{case.name}: --version --output plain failed: {plain.stderr!r}"
    assert "result.code=version" in plain.stdout, (
        f"{case.name}: --version --output plain is not the structured event: {plain.stdout!r}"
    )


def assert_help_output_contract(case: CliCase) -> None:
    proc = run_cli(case, ("--help",))
    assert proc.returncode == 0, f"{case.name}: --help failed: {proc.stderr!r}"
    event = json.loads(proc.stdout)
    validate_help_event(event)
    help_model = event["result"]["help"]
    assert help_model["scope"] == "one_level", (
        f"{case.name}: default help is not one-level: {event!r}"
    )
    assert help_model["command_path"], f"{case.name}: help omitted command_path"
    serialized_help = json.dumps(help_model)
    assert '"description"' not in serialized_help, (
        f"{case.name}: default JSON help embeds a long description"
    )
    assert '"description_markdown"' not in serialized_help, (
        f"{case.name}: default JSON help unexpectedly exposes full Markdown"
    )
    assert "code" not in help_model, f"{case.name}: help duplicated result.code"
    assert "versions" not in help_model, f"{case.name}: help eagerly disclosed versions"
    assert "--version" in json.dumps(help_model), f"{case.name}: help omitted --version"
    plain = run_cli(case, ("--help", "--output", "plain"))
    assert plain.returncode == 0, f"{case.name}: plain help failed: {plain.stderr!r}"
    assert "usage:" in plain.stdout.lower(), (
        f"{case.name}: explicit plain help is not text: {plain.stdout!r}"
    )
    assert "AFDATA:" not in plain.stdout and "Version:" not in plain.stdout, (
        f"{case.name}: plain help eagerly disclosed version metadata"
    )
    recursive = run_cli(case, ("--help", "--recursive"))
    assert recursive.returncode == 0, f"{case.name}: recursive help failed: {recursive.stderr!r}"
    recursive_event = json.loads(recursive.stdout)
    validate_help_event(recursive_event)
    recursive_help = recursive_event["result"]["help"]
    assert recursive_help["scope"] == "recursive", (
        f"{case.name}: recursive help has wrong scope: {recursive_event!r}"
    )
    serialized_recursive_help = json.dumps(recursive_help)
    assert '"description"' not in serialized_recursive_help, (
        f"{case.name}: recursive JSON help embeds a long description"
    )
    assert '"description_markdown"' not in serialized_recursive_help, (
        f"{case.name}: recursive JSON help unexpectedly exposes full Markdown"
    )


def help_surface_counts(help_model: dict) -> tuple[int, int]:
    commands_count = 0
    arguments_count = 0

    def visit(command: dict) -> None:
        nonlocal commands_count, arguments_count
        commands_count += 1
        arguments_count += len(command.get("arguments", []))
        for child in command.get("subcommands", []):
            visit(child)

    visit(help_model)
    return commands_count, arguments_count


def assert_afdata_validate() -> None:
    proc = run_afdata(("validate", "-"), '{"kind":"result","result":{"ok":true}}\n')
    assert proc.returncode == 0, f"afdata validate failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = terminal_events(proc)
    assert len(events) == 1, f"afdata validate emitted unexpected events: {events!r}"
    assert events[0]["kind"] == "result", f"afdata validate did not emit result: {events[0]!r}"
    assert events[0]["trace"] == {}, f"afdata validate result is not strict: {events[0]!r}"


def assert_afdata_validate_strict_event() -> None:
    valid = '{"kind":"log","log":{"event":"startup"},"trace":{}}\n'
    proc = run_afdata(("validate", "-", "--strict", "--per-event"), valid)
    assert proc.returncode == 0, f"strict event failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = terminal_events(proc)
    assert events[0]["kind"] == "result", f"strict event result missing: {events!r}"

    invalid = '{"kind":"log","log":{"event":"startup"}}\n'
    proc = run_afdata(("validate", "-", "--strict", "--per-event"), invalid)
    assert proc.returncode != 0, "strict event accepted an event without trace"
    events = terminal_events(proc)
    assert events[0]["kind"] == "error", f"strict event error missing: {events!r}"
    assert events[0]["trace"] == {}, f"strict event error is not strict: {events[0]!r}"


def assert_afdata_validate_stream_error() -> None:
    proc = run_afdata(("validate", "-"), '{"kind":"log","log":{"event":"startup"}}\n')
    assert proc.returncode != 0, "afdata validate accepted a stream without terminal event"
    events = terminal_events(proc)
    assert events[0]["kind"] == "error", f"afdata validate stream error missing: {events!r}"
    assert events[0]["error"]["code"] == "validation_failed", f"wrong validation code: {events[0]!r}"


def assert_afdata_lint_schema_secret() -> None:
    schema = '{"type":"object","properties":{"api_key_secret":{"type":"string","default":"sk-live"}}}\n'
    proc = run_afdata(("lint", "-"), schema)
    assert proc.returncode != 0, "afdata lint accepted exposed secret default"
    events = terminal_events(proc)
    assert events[0]["kind"] == "error", f"afdata lint error missing: {events!r}"
    findings = events[0]["error"]["findings"]
    assert findings[0]["rule_id"] == "secret_schema_value_exposed", f"wrong lint finding: {findings!r}"
    # A null default/example is a valid absent/redacted secret literal, not an
    # exposed one.
    ok_schema = (
        '{"type":"object","properties":{"api_key_secret":'
        '{"type":"string","default":null,"examples":[null,"***"]}}}\n'
    )
    ok = run_afdata(("lint", "-"), ok_schema)
    assert ok.returncode == 0, f"afdata lint rejected a null secret schema default/examples: {ok.stdout!r}"


def assert_afdata_lint_schema_suffix_types() -> None:
    schema = {
        "type": "object",
        "properties": {
            "duration_ms": {"type": ["integer", "null"], "minimum": 0},
            "callback_url": {"type": "string"},
            "created_rfc3339": {"type": "string"},
            "nested": {
                "type": "object",
                "properties": {
                    "payload_bytes": {"type": "integer", "minimum": 0},
                },
            },
        },
    }
    proc = run_afdata(("lint", "-"), json.dumps(schema) + "\n")
    assert proc.returncode == 0, (
        "afdata lint treated JSON Schema descriptors as runtime values: "
        f"{proc.stdout!r} {proc.stderr!r}"
    )

    schema["properties"]["duration_ms"] = {"type": "string"}
    invalid = run_afdata(("lint", "-"), json.dumps(schema) + "\n")
    assert invalid.returncode != 0, "afdata lint accepted an incompatible schema suffix type"
    findings = terminal_events(invalid)[0]["error"]["findings"]
    assert findings[0]["rule_id"] == "suffix_type_mismatch", findings

    # A `properties` object whose values are not schemas (an object or a
    # boolean) is runtime data using `properties` as a field name, so its
    # values keep the ordinary runtime suffix check.
    data = run_afdata(("lint", "-"), '{"properties":{"timeout_ms":"5000"}}\n')
    assert data.returncode != 0, "afdata lint skipped runtime data under a `properties` field"
    findings = terminal_events(data)[0]["error"]["findings"]
    assert findings[0]["rule_id"] == "suffix_type_mismatch", findings
    assert findings[0]["pointer"] == "/properties/timeout_ms", findings


def assert_afdata_lint_bcp47() -> None:
    proc = run_afdata(("lint", "-"), '{"language_bcp47":"zh_CN"}\n')
    assert proc.returncode != 0, "afdata lint accepted malformed BCP 47 tag"
    events = terminal_events(proc)
    assert events[0]["kind"] == "error", f"afdata lint error missing: {events!r}"
    findings = events[0]["error"]["findings"]
    assert findings[0]["rule_id"] == "suffix_type_mismatch", f"wrong lint finding: {findings!r}"
    ok = run_afdata(("lint", "-"), '{"language_bcp47":"zh-CN"}\n')
    assert ok.returncode == 0, f"afdata lint rejected valid BCP 47 tag: {ok.stdout!r}"


def assert_afdata_lint_strict_strings() -> None:
    for payload in (
        '{"timezone_utc_offset":"Asia/Shanghai"}\n',
        '{"market_open_rfc3339_time":"09:30:00Z"}\n',
        '{"invoice_due_rfc3339_date":"2026-13-01"}\n',
        # RFC 3339 date-time with no offset — the offset is mandatory.
        '{"expires_rfc3339":"2026-02-14T10:30:00"}\n',
        # Space separator instead of T.
        '{"expires_rfc3339":"2026-02-14 10:30:00Z"}\n',
    ):
        proc = run_afdata(("lint", "-"), payload)
        assert proc.returncode != 0, f"afdata lint accepted malformed strict string: {payload!r}"
        events = terminal_events(proc)
        findings = events[0]["error"]["findings"]
        assert findings[0]["rule_id"] == "suffix_type_mismatch", f"wrong lint finding: {findings!r}"
    ok = run_afdata(
        ("lint", "-"),
        '{"timezone_utc_offset":"+08:00","market_open_rfc3339_time":"09:30:00","invoice_due_rfc3339_date":"2026-06-13","expires_rfc3339":"2026-02-14T10:30:00.5+08:00"}\n',
    )
    assert ok.returncode == 0, f"afdata lint rejected valid strict strings: {ok.stdout!r}"


def assert_afdata_lint_numeric_and_url() -> None:
    for payload in (
        # Durations must be numeric, not unit-in-value strings.
        '{"timeout_s":"30"}\n',
        '{"retry_after_ms":"100ms"}\n',
        # Minor-unit currency amounts must be integers.
        '{"price_usd_cents":12.5}\n',
        '{"fee_jpy":"100"}\n',
        # A _url must be a single URL: no internal whitespace, no bare credentials.
        '{"callback_url":"https://example.com/a b"}\n',
        '{"db_url":"user:pass@host:5432/db"}\n',
    ):
        proc = run_afdata(("lint", "-"), payload)
        assert proc.returncode != 0, f"afdata lint accepted malformed numeric/url field: {payload!r}"
        events = terminal_events(proc)
        findings = events[0]["error"]["findings"]
        assert findings[0]["rule_id"] == "suffix_type_mismatch", f"wrong lint finding: {findings!r}"
    ok = run_afdata(
        ("lint", "-"),
        '{"timeout_s":30,"retry_after_ms":100,"price_usd_cents":1250,"fee_jpy":100,"callback_url":"https://example.com/cb?page=2","final_url":"/relative/path"}\n',
    )
    assert ok.returncode == 0, f"afdata lint rejected valid numeric/url fields: {ok.stdout!r}"


def assert_afdata_lint_null_suffix_exempt() -> None:
    # `null` means the field is absent/unset, so every suffix-typed family
    # must accept it with zero findings — one field per family.
    payload = {
        "cached_epoch_s": None,
        "created_at_epoch_ms": None,
        "created_epoch_ns": None,
        "payload_bytes": None,
        "withdrawn_sats": None,
        "balance_msats": None,
        "cpu_percent": None,
        "dns_ttl_s": None,
        "latency_ms": None,
        "session_timeout_minutes": None,
        "price_usd_cents": None,
        "deposit_eur_cents": None,
        "cost_usd_micro": None,
        "fee_jpy": None,
        "expires_rfc3339": None,
        "invoice_due_rfc3339_date": None,
        "market_open_rfc3339_time": None,
        "timezone_utc_offset": None,
        "callback_url": None,
        "language_bcp47": None,
        "api_key_secret": None,
    }
    proc = run_afdata(("lint", "-"), json.dumps(payload) + "\n")
    assert proc.returncode == 0, f"afdata lint rejected null suffix-typed fields: {proc.stdout!r} {proc.stderr!r}"
    events = terminal_events(proc)
    assert events[0]["kind"] == "result", f"afdata lint did not pass null suffix-typed fields: {events!r}"
    assert events[0]["result"]["findings"] == [], f"unexpected findings for null fields: {events[0]!r}"


def assert_afdata_lint_null_suffix_exempt_nested() -> None:
    # The same exemption applies at any nesting depth: inside an object and
    # inside array elements.
    payload = {
        "meta": {"cached_epoch_s": None, "callback_url": None},
        "items": [
            {"withdrawn_sats": None, "language_bcp47": None},
            {"price_usd_cents": None, "api_key_secret": None},
        ],
    }
    proc = run_afdata(("lint", "-"), json.dumps(payload) + "\n")
    assert proc.returncode == 0, f"afdata lint rejected nested null suffix-typed fields: {proc.stdout!r} {proc.stderr!r}"
    events = terminal_events(proc)
    assert events[0]["kind"] == "result", f"afdata lint did not pass nested null suffix-typed fields: {events!r}"
    assert events[0]["result"]["findings"] == [], f"unexpected findings for nested null fields: {events[0]!r}"


def assert_afdata_lint_suffix_type_regressions() -> None:
    # Present-but-wrong-type values must still fail — the null exemption must
    # not have loosened the type checks for actual values.
    for payload in (
        '{"count_epoch_s":"abc"}\n',
        '{"size_bytes":-1}\n',
        # A bare number, not the required decimal integer string.
        '{"x_epoch_ns":123}\n',
        '{"when_rfc3339":"not-a-date"}\n',
    ):
        proc = run_afdata(("lint", "-"), payload)
        assert proc.returncode != 0, f"afdata lint accepted an invalid present value: {payload!r}"
        events = terminal_events(proc)
        findings = events[0]["error"]["findings"]
        assert findings[0]["rule_id"] == "suffix_type_mismatch", f"wrong lint finding: {findings!r}"
    # Valid present values must still pass.
    ok = run_afdata(
        ("lint", "-"),
        '{"cached_epoch_s":1707868800,"id_epoch_ns":"1707868800000000000"}\n',
    )
    assert ok.returncode == 0, f"afdata lint rejected valid present suffix-typed values: {ok.stdout!r}"


def assert_afdata_cli_capabilities() -> None:
    # The default binary is full-featured, so every capability is present.
    ver = run_afdata(("--version", "--output", "json"), "")
    assert ver.returncode == 0, f"afdata --version --output json failed: {ver.stderr!r}"
    payload = json.loads(ver.stdout)
    assert payload["result"]["version"], f"no version in {ver.stdout!r}"
    default_help = run_afdata(("--help",), "")
    assert default_help.returncode == 0, f"afdata --help failed: {default_help.stderr!r}"
    help_event = json.loads(default_help.stdout)
    validate_help_event(help_event)
    help_model = help_event["result"]["help"]
    assert help_model["command_path"] == "afdata", help_model
    assert "code" not in help_model and "versions" not in help_model, help_model
    assert all(
        argument.get("global") is True
        for argument in help_model["arguments"]
        if argument["name"] in {"--output", "--output-to", "--stdout-file", "--stderr-file"}
    ), f"root global arguments are not marked global: {help_model['arguments']!r}"
    assert any(argument["name"] == "--version" for argument in help_model["arguments"]), (
        f"help omitted clap's version flag: {help_model['arguments']!r}"
    )
    validated_help = run_afdata(
        ("validate", "-", "--strict", "--per-event"),
        default_help.stdout,
    )
    assert validated_help.returncode == 0, (
        f"default help is not a strict protocol event: {validated_help.stderr!r}"
    )
    plain = run_afdata(("--help", "--output", "plain"), "")
    assert plain.returncode == 0, f"afdata --help --output plain failed: {plain.stderr!r}"
    assert "Usage: afdata" in plain.stdout, f"plain help is not conventional text: {plain.stdout[:80]!r}"
    recursive_json = run_afdata(("--help", "--recursive", "--output", "json"), "")
    assert recursive_json.returncode == 0, f"recursive JSON help failed: {recursive_json.stderr!r}"
    recursive_event = json.loads(recursive_json.stdout)
    validate_help_event(recursive_event)
    recursive_model = recursive_event["result"]["help"]
    commands_count, arguments_count = help_surface_counts(recursive_model)
    json_budget_bytes = 512 + commands_count * 160 + arguments_count * 120
    assert len(recursive_json.stdout.encode()) < json_budget_bytes, (
        "recursive JSON help exceeded its structural payload budget: "
        f"{len(recursive_json.stdout.encode())} >= {json_budget_bytes} bytes "
        f"for {commands_count} commands and {arguments_count} arguments"
    )
    linted_help = run_afdata(("lint", "-"), recursive_json.stdout)
    assert linted_help.returncode == 0, (
        f"recursive JSON help is not valid AFDATA: {linted_help.stderr!r}"
    )
    recursive_plain = run_afdata(("--help", "--recursive", "--output", "plain"), "")
    assert recursive_plain.returncode == 0, f"recursive plain help failed: {recursive_plain.stderr!r}"
    plain_budget_bytes = 400 + commands_count * 160 + arguments_count * 70
    assert len(recursive_plain.stdout.encode()) < plain_budget_bytes, (
        "recursive plain help exceeded its structural payload budget: "
        f"{len(recursive_plain.stdout.encode())} >= {plain_budget_bytes} bytes "
        f"for {commands_count} commands and {arguments_count} arguments"
    )
    md = run_afdata(("--help", "--output", "markdown"), "")
    assert md.returncode == 0, f"afdata --help --output markdown failed: {md.stderr!r}"
    assert md.stdout.lstrip().startswith("# afdata"), f"help is not markdown: {md.stdout[:80]!r}"
    assert "--stdout-file" in md.stdout, f"stream-redirect flag missing from help: {md.stdout[:200]!r}"
    assert "skill" in md.stdout, f"skill command missing from help: {md.stdout[:200]!r}"

    scoped = run_afdata(("set", "--help"), "")
    assert scoped.returncode == 0, f"afdata set --help failed: {scoped.stderr!r}"
    scoped_event = json.loads(scoped.stdout)
    validate_help_event(scoped_event)
    scoped_help = scoped_event["result"]["help"]
    assert scoped_help["command_path"] == "afdata set", scoped_help
    assert scoped_help["inherited_arguments_from"] == ["afdata"], scoped_help
    scoped_names = {argument["name"] for argument in scoped_help["arguments"]}
    assert {"FILE", "KEY", "--value-type"} <= scoped_names, scoped_help
    assert "--output" not in scoped_names, (
        f"scoped structured help repeated inherited globals: {scoped_help!r}"
    )
    scoped_plain = run_afdata(("set", "--help", "--output", "plain"), "")
    assert scoped_plain.returncode == 0, f"afdata set plain help failed: {scoped_plain.stderr!r}"
    assert "Usage: afdata set" in scoped_plain.stdout, scoped_plain.stdout[:160]
    assert "--output" in scoped_plain.stdout, (
        f"scoped conventional help omitted inherited globals: {scoped_plain.stdout[:300]!r}"
    )

    missing = run_afdata((), "")
    assert missing.returncode == 2, f"afdata without a command returned {missing.returncode}"
    assert not missing.stdout, f"split output leaked an error to stdout: {missing.stdout!r}"
    missing_event = json.loads(missing.stderr)
    assert missing_event["error"]["message"] == "a command is required", missing_event
    assert missing_event["error"]["hint"] == "try: afdata --help", missing_event
    assert len(missing.stderr.encode()) < 256, (
        f"missing-command error embedded eager help: {len(missing.stderr.encode())} bytes"
    )

    # `-h`/`-V` are deliberately unsupported: AFDATA spells both out in full, and
    # letting them reach Clap would answer in plain text at exit 0, bypassing the
    # output contract entirely.
    for short_flag in ("-h", "-V"):
        short = run_afdata((short_flag,), "")
        assert short.returncode == 2, (
            f"afdata {short_flag} returned {short.returncode}, expected a structured error"
        )
        assert not short.stdout, f"split output leaked an error to stdout: {short.stdout!r}"
        short_event = json.loads(short.stderr)
        assert short_event["error"]["code"] == "cli_error", short_event
        assert "--help" in short_event["error"]["hint"], short_event
    scoped_short = run_afdata(("get", "-h"), "")
    assert scoped_short.returncode == 2, (
        f"afdata get -h returned {scoped_short.returncode}, expected a structured error"
    )

    pseudo = run_afdata(("help",), "")
    assert pseudo.returncode == 2, f"afdata help pseudo-command returned {pseudo.returncode}"
    assert not pseudo.stdout, f"split output leaked an error to stdout: {pseudo.stdout!r}"
    pseudo_event = json.loads(pseudo.stderr)
    assert pseudo_event["error"]["code"] == "cli_error", pseudo_event


def assert_afdata_render_redacts() -> None:
    proc = run_afdata(("render", "-", "--output", "json"), '{"api_key_secret":"sk-live","ok":true}\n')
    assert proc.returncode == 0, f"afdata render failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    value = json.loads(proc.stdout)
    assert value["api_key_secret"] == "***", f"afdata render did not redact: {value!r}"
    assert value["ok"] is True, f"afdata render changed non-secret value: {value!r}"


def assert_afdata_parse_error() -> None:
    proc = run_afdata(("render", "-"), '{"ok":true}\nnot-json\n')
    assert proc.returncode != 0, "afdata render accepted invalid JSONL"
    events = terminal_events(proc)
    assert events[0]["kind"] == "error", f"afdata parse error missing: {events!r}"
    assert events[0]["error"]["code"] == "jsonl_parse_failed", f"wrong parse code: {events[0]!r}"


def assert_afdata_skill_status_feature() -> None:
    with tempfile.TemporaryDirectory(prefix="afdata-skill-e2e-") as tmp:
        proc = run_afdata_skill(("skill", "status", "--agent", "codex", "--skills-dir", tmp))
    assert proc.returncode == 0, f"afdata skill status failed: stderr={proc.stderr!r}, stdout={proc.stdout!r}"
    events = terminal_events(proc)
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
    events = terminal_events(proc)
    assert len(events) == 1, f"afdata skill install emitted unexpected events: {events!r}"
    error = events[0]["error"]
    assert error["code"] == "cli_error", f"wrong skill error code: {events[0]!r}"
    report = error["partial_report"]
    assert report["code"] == "skill_install", f"wrong partial report code: {report!r}"
    assert report["installed"] is False, f"partial install report should be failed: {report!r}"
    assert report["targets"][0]["installed"] is True, f"partial report lost target status: {report!r}"
    assert report["targets"][0]["managed"] is False, f"unmanaged target should be reported: {report!r}"


def assert_afdata_skill_help_is_feature_gated() -> None:
    # Default build is full-featured: skill management is present.
    full_help = run_afdata(("--help",), "")
    assert full_help.returncode == 0, f"default afdata help failed: {full_help.stderr!r}"
    assert "skill" in full_help.stdout, "default afdata help must show skill subcommand"
    # Opting out (default-features = false) drops back to the core CLI.
    minimal_help = run_afdata_minimal(("--help",))
    assert minimal_help.returncode == 0, f"minimal afdata help failed: {minimal_help.stderr!r}"
    assert "skill" not in minimal_help.stdout, "minimal afdata help must not show skill subcommand"


def main() -> None:
    checks = (
        assert_single_terminal,
        assert_startup_log,
        assert_unknown_arg,
        assert_invalid_output_fallback,
        assert_json_flag_is_not_afdata_s,
        assert_cancelled,
        assert_broken_pipe_no_traceback,
        assert_version,
        assert_help_output_contract,
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
        assert_afdata_lint_schema_suffix_types,
        assert_afdata_lint_bcp47,
        assert_afdata_lint_strict_strings,
        assert_afdata_lint_numeric_and_url,
        assert_afdata_lint_null_suffix_exempt,
        assert_afdata_lint_null_suffix_exempt_nested,
        assert_afdata_lint_suffix_type_regressions,
        assert_afdata_cli_capabilities,
        assert_afdata_render_redacts,
        assert_afdata_parse_error,
        assert_afdata_skill_status_feature,
        assert_afdata_skill_error_includes_partial_report,
        assert_afdata_skill_help_is_feature_gated,
    ):
        check()
    print("[e2e] afdata: ok")


if __name__ == "__main__":
    main()
