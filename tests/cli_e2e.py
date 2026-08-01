#!/usr/bin/env python3
"""End-to-end checks for the closed-world afdata CLI."""

from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path
from typing import Sequence


ROOT = Path(__file__).resolve().parents[1]


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


def run_rust_example(args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo", "run", "--quiet", "--example", "agent_cli", "--", *args],
        cwd=ROOT,
        text=True,
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



def validate_help_v2_event(event: dict[str, object]) -> dict[str, object]:
    assert set(event) == {"kind", "result", "trace"}, event
    assert event["kind"] == "result", event
    assert event["trace"] == {}, event
    result = event["result"]
    assert isinstance(result, dict) and set(result) == {"code", "help"}, event
    assert result["code"] == "help", event
    help_model = result["help"]
    assert isinstance(help_model, dict), event
    assert help_model.get("schema") == "cli-help-v2", event
    assert isinstance(help_model.get("command_path"), str), event
    about = help_model.get("about")
    assert about is None or (
        isinstance(about, str) and about and "\n" not in about and "\r" not in about
    ), event
    shapes = help_model.get("shapes", [])
    assert isinstance(shapes, list), event
    for shape in shapes:
        assert set(shape) <= {"id", "about", "usage"}, event
        assert isinstance(shape.get("id"), str) and shape["id"], event
        usage = shape.get("usage")
        assert isinstance(usage, str) and usage and "\n" not in usage, event
    # Help answers in one round trip, so every shape is complete: there is no
    # second level that could have carried the optional arguments.
    if len(shapes) > 1:
        assert all(shape.get("about") for shape in shapes), event
    elif shapes:
        assert "about" not in shapes[0], event
    return help_model


def assert_rust_example_uses_help_v2() -> None:
    help_proc = run_rust_example(("echo", "--help"))
    assert help_proc.returncode == 0, help_proc.stderr
    help_model = validate_help_v2_event(json.loads(help_proc.stdout))
    assert [shape["id"] for shape in help_model["shapes"]] == ["echo"]
    assert help_model["shapes"][0]["usage"].startswith("agent-cli echo <MESSAGE>")

    version_proc = run_rust_example(("--version",))
    assert version_proc.returncode == 0, version_proc.stderr
    version = json.loads(version_proc.stdout)
    assert version["result"]["code"] == "version", version

    conflict = run_rust_example(("ping", "--host", "example.com", "--dry-run"))
    assert conflict.returncode == 2, conflict
    assert not conflict.stdout, conflict.stdout
    error = json.loads(conflict.stderr)
    assert error["error"]["code"] == "cli_unknown_argument", error


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

    # Naming suffixes are ASCII case-insensitive. Uppercase spellings must
    # enforce the same contracts as their lowercase equivalents.
    uppercase_bad = run_afdata(
        ("lint", "-"),
        '{"CACHED_EPOCH_S":"abc","SIZE_BYTES":-1,"CALLBACK_URL":7}\n',
    )
    assert uppercase_bad.returncode != 0, (
        f"afdata lint accepted invalid uppercase suffix-typed fields: "
        f"{uppercase_bad.stdout!r}"
    )
    uppercase_findings = terminal_events(uppercase_bad)[0]["error"]["findings"]
    assert len(uppercase_findings) == 3, uppercase_findings
    assert all(
        finding["rule_id"] == "suffix_type_mismatch"
        for finding in uppercase_findings
    ), uppercase_findings

    uppercase_ok = run_afdata(
        ("lint", "-"),
        '{"CACHED_EPOCH_S":1707868800,"CALLBACK_URL":"https://example.com"}\n',
    )
    assert uppercase_ok.returncode == 0, (
        f"afdata lint rejected valid uppercase suffix-typed fields: "
        f"{uppercase_ok.stdout!r}"
    )


def assert_afdata_cli_capabilities() -> None:
    ver = run_afdata(("--version", "--output", "json"), "")
    assert ver.returncode == 0, f"afdata --version --output json failed: {ver.stderr!r}"
    payload = json.loads(ver.stdout)
    assert payload["result"]["version"], f"no version in {ver.stdout!r}"

    default_help = run_afdata(("--help",), "")
    assert default_help.returncode == 0, f"afdata --help failed: {default_help.stderr!r}"
    help_event = json.loads(default_help.stdout)
    help_model = validate_help_v2_event(help_event)
    assert help_model["command_path"] == "afdata", help_model
    assert "arguments" not in help_model, help_model
    assert len(default_help.stdout.encode()) < 2000, (
        f"root discovery help is too large: {len(default_help.stdout.encode())} bytes"
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
    assert "more:" in plain.stdout, plain.stdout[:160]

    recursive_json = run_afdata(("--help", "--recursive"), "")
    assert recursive_json.returncode == 2, "--recursive must not survive help-v2"
    assert json.loads(recursive_json.stderr)["error"]["code"] == "cli_unknown_argument"

    md = run_afdata(("--help", "--output", "markdown"), "")
    assert md.returncode == 2, "help-v2 must not retain Markdown export"
    assert json.loads(md.stderr)["error"]["code"] == "cli_invalid_argument_value"

    scoped = run_afdata(("set", "--help"), "")
    assert scoped.returncode == 0, f"afdata set --help failed: {scoped.stderr!r}"
    scoped_event = json.loads(scoped.stdout)
    scoped_help = validate_help_v2_event(scoped_event)
    assert scoped_help["command_path"] == "afdata set", scoped_help
    assert [shape["id"] for shape in scoped_help["shapes"]] == [
        "set-value",
        "set-null",
        "set-secret",
    ], scoped_help
    for shape in scoped_help["shapes"]:
        assert "--output <json|yaml|plain>" in shape["usage"], shape
    # One complete answer costs more than the old first level did, and less than
    # that level plus a follow-up for the shape the caller actually wanted.
    assert len(scoped.stdout.encode()) < 2200, (
        f"set help is too large: {len(scoped.stdout.encode())} bytes"
    )

    combination_help = run_afdata(("set", "--help-combination", "set-null"), "")
    assert combination_help.returncode == 2, "the second help level must not survive"
    assert (
        json.loads(combination_help.stderr)["error"]["code"] == "cli_unknown_argument"
    ), combination_help.stderr

    missing = run_afdata((), "")
    assert missing.returncode == 2, f"afdata without a command returned {missing.returncode}"
    assert not missing.stdout, f"split output leaked an error to stdout: {missing.stdout!r}"
    missing_event = json.loads(missing.stderr)
    assert missing_event["error"]["code"] == "cli_unregistered_combination", missing_event
    assert "`afdata --help`" in missing_event["error"]["hint"], missing_event
    assert len(missing.stderr.encode()) < 512, (
        f"missing-command error embedded eager help: {len(missing.stderr.encode())} bytes"
    )

    # `-h`/`-V` are deliberately unsupported: AFDATA spells both out in full, and
    # Short aliases are deliberately unsupported by the registered grammar.
    for short_flag in ("-h", "-V"):
        short = run_afdata((short_flag,), "")
        assert short.returncode == 2, (
            f"afdata {short_flag} returned {short.returncode}, expected a structured error"
        )
        assert not short.stdout, f"split output leaked an error to stdout: {short.stdout!r}"
        short_event = json.loads(short.stderr)
        assert short_event["error"]["code"] == "cli_unknown_argument", short_event
        assert "--help" in short_event["error"]["hint"], short_event
    scoped_short = run_afdata(("get", "-h"), "")
    assert scoped_short.returncode == 2, (
        f"afdata get -h returned {scoped_short.returncode}, expected a structured error"
    )

    pseudo = run_afdata(("help",), "")
    assert pseudo.returncode == 2, f"afdata help pseudo-command returned {pseudo.returncode}"
    assert not pseudo.stdout, f"split output leaked an error to stdout: {pseudo.stdout!r}"
    pseudo_event = json.loads(pseudo.stderr)
    assert pseudo_event["error"]["code"] == "cli_unknown_command", pseudo_event

    conflict = run_afdata(
        ("set", "data.json", "key", "visible", "--secret-from", "env:VALUE_SECRET"),
        "",
    )
    assert conflict.returncode == 2
    assert not conflict.stdout
    assert "VALUE_SECRET" not in conflict.stderr
    assert json.loads(conflict.stderr)["error"]["code"] == "cli_unregistered_combination"

    raw_conflict = run_afdata(("value", "data.json", "key", "--output", "json"), "")
    assert raw_conflict.returncode == 2
    assert json.loads(raw_conflict.stderr)["error"]["code"] == "cli_unregistered_combination"

    # A value the registry accepted but the command cannot use is still an
    # invalid argument value. `cli-spec-v1` has no "non-empty string" type, so
    # this one is decided a layer later — but a caller branches on `error.code`,
    # and the layer that noticed is not something it should have to know. The
    # generic `cli_error` stays available to CLIs with no registry to name a
    # rule; a registry-compiled one must never fall back to it.
    for argv in (
        ("emit", "result", ""),
        ("emit", "log", "info", ""),
        ("emit", "error", "", "boom"),
    ):
        empty = run_afdata(argv, "")
        assert empty.returncode == 2, f"{argv} returned {empty.returncode}"
        assert not empty.stdout, f"split output leaked an error to stdout: {empty.stdout!r}"
        empty_event = json.loads(empty.stderr)
        assert empty_event["error"]["code"] == "cli_invalid_argument_value", empty_event
        assert "--help" in empty_event["error"]["hint"], empty_event


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
    assert_rust_example_uses_help_v2()
    print("[e2e] Rust CliSpec example: ok")
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
