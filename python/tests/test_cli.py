"""Tests for agent_first_data CLI helpers."""
import json

import pytest
from io import StringIO
from agent_first_data import (
    OutputFormat,
    OutputStyle,
    OutputOptions,
    LogLevel,
    CliEmitter,
    json_error,
    json_log,
    json_result,
    cli_parse_output,
    cli_parse_log_filters,
    cli_output,
    build_cli_error,
    build_cli_version,
    cli_render_version,
    cli_handle_version_or_continue,
    output_json,
)


# ── cli_parse_output ──────────────────────────────────────────────────────────

def test_parse_output_all_formats():
    assert cli_parse_output("json") is OutputFormat.JSON
    assert cli_parse_output("yaml") is OutputFormat.YAML
    assert cli_parse_output("plain") is OutputFormat.PLAIN


def test_parse_output_rejects_unknown():
    with pytest.raises(ValueError):
        cli_parse_output("xml")
    with pytest.raises(ValueError):
        cli_parse_output("JSON")
    with pytest.raises(ValueError):
        cli_parse_output("")


def test_parse_output_error_contains_value():
    with pytest.raises(ValueError, match="toml"):
        cli_parse_output("toml")
    with pytest.raises(ValueError, match="json"):
        cli_parse_output("toml")


# ── cli_parse_log_filters ─────────────────────────────────────────────────────

def test_parse_log_filters_trims_and_lowercases():
    assert list(cli_parse_log_filters(["  Query  ", "ERROR"])) == ["query", "error"]


def test_parse_log_filters_deduplicates():
    assert list(cli_parse_log_filters(["query", "error", "Query", "query"])) == ["query", "error"]


def test_parse_log_filters_removes_empty():
    assert list(cli_parse_log_filters(["", "query", "  "])) == ["query"]


def test_parse_log_filters_empty_list():
    assert list(cli_parse_log_filters([])) == []


def test_parse_log_filters_preserves_order():
    assert list(cli_parse_log_filters(["startup", "request", "retry"])) == ["startup", "request", "retry"]


# ── build_cli_error ───────────────────────────────────────────────────────────

def test_build_cli_error_required_fields():
    v = build_cli_error("missing --sql")
    assert v["kind"] == "error"
    assert v["error"]["code"] == "cli_error"
    assert v["error"]["message"] == "missing --sql"
    assert v["error"]["retryable"] is False
    assert "error_code" not in v
    assert "retryable" not in v
    assert v["trace"] == {}


def test_build_cli_error_is_valid_json():
    import json
    v = build_cli_error("oops")
    s = output_json(v)
    parsed = json.loads(s)
    assert parsed["kind"] == "error"
    assert parsed["error"]["code"] == "cli_error"


def test_build_cli_error_with_hint():
    v = build_cli_error("bad flag", hint="try --help")
    assert v["error"]["hint"] == "try --help"


def test_build_cli_error_without_hint_has_no_hint_key():
    v = build_cli_error("oops")
    assert "hint" not in v["error"]


# ── cli_output ────────────────────────────────────────────────────────────────

def test_cli_output_dispatches_json():
    v = json_result({"size_bytes": 1024}).build().to_dict()
    out = cli_output(v, OutputFormat.JSON)
    assert "size_bytes" in out   # json: raw keys, no suffix processing
    assert "\n" not in out


def test_cli_output_dispatches_yaml():
    v = json_result({"size_bytes": 1024}).build().to_dict()
    out = cli_output(v, OutputFormat.YAML)
    assert out.startswith("---")
    assert "size:" in out        # yaml: suffix stripped


def test_cli_output_dispatches_plain():
    v = json_result({"ok": True}).build().to_dict()
    out = cli_output(v, OutputFormat.PLAIN)
    assert "\n" not in out
    assert "kind=result" in out


def test_cli_output_dispatches_raw_yaml_with_options():
    v = {"size_bytes": 1024}
    out = cli_output(
        v,
        OutputFormat.YAML,
        options=OutputOptions(style=OutputStyle.Raw),
    )
    assert "size_bytes: 1024" in out
    assert "size:" not in out


# ── CliEmitter ────────────────────────────────────────────────────────────────

def test_cli_emitter_writes_events_and_tracks_terminal():
    writer = StringIO()
    emitter = CliEmitter(writer, OutputFormat.JSON)
    emitter.emit(json_log(LogLevel.INFO, "startup").build())
    emitter.emit(json_result({"rows": 2}).build())
    lines = writer.getvalue().splitlines()
    assert len(lines) == 2
    assert '"kind":"log"' in lines[0]
    assert '"kind":"result"' in lines[1]


def test_cli_emitter_framing_all_formats():
    events = [
        json_log(LogLevel.INFO, "startup").build(),
        json_result({"rows": 2}).build(),
    ]
    for fmt in (OutputFormat.JSON, OutputFormat.PLAIN, OutputFormat.YAML):
        writer = StringIO()
        emitter = CliEmitter(writer, fmt)
        for event in events:
            emitter.emit(event)
        out = writer.getvalue()
        if fmt is OutputFormat.JSON:
            lines = out.rstrip("\n").split("\n")
            assert len(lines) == 2
            assert [json.loads(line)["kind"] for line in lines] == ["log", "result"]
        elif fmt is OutputFormat.PLAIN:
            lines = out.rstrip("\n").split("\n")
            assert len(lines) == 2
            assert lines[0].startswith("kind=log")
            assert lines[1].startswith("kind=result")
        else:
            assert out.count("---") == 2


def test_cli_emitter_rejects_duplicate_terminal():
    writer = StringIO()
    emitter = CliEmitter(writer, OutputFormat.JSON)
    emitter.emit(json_result({"rows": 2}).build())
    with pytest.raises(RuntimeError, match="duplicate terminal"):
        emitter.emit(json_error("late_error", "too late").build())


def test_cli_emitter_rejects_non_terminal_after_terminal():
    writer = StringIO()
    emitter = CliEmitter(writer, OutputFormat.JSON)
    emitter.emit(json_result({"rows": 2}).build())
    with pytest.raises(RuntimeError, match="after terminal"):
        emitter.emit_progress("100%")


class FailingWriter:
    def write(self, _value: str) -> None:
        raise BrokenPipeError("closed")


def test_cli_emitter_returns_writer_errors():
    emitter = CliEmitter(FailingWriter(), OutputFormat.JSON)
    with pytest.raises(BrokenPipeError):
        emitter.emit(json_result({"rows": 2}).build())


class FailOnceWriter:
    def __init__(self) -> None:
        self.failed = False
        self.value = ""

    def write(self, value: str) -> None:
        if not self.failed:
            self.failed = True
            raise InterruptedError("retry")
        self.value += value

    def flush(self) -> None:
        pass


def test_cli_emitter_does_not_commit_terminal_state_when_write_fails():
    writer = FailOnceWriter()
    emitter = CliEmitter(writer, OutputFormat.JSON)
    event = json_result({"rows": 2}).build()
    with pytest.raises(InterruptedError):
        emitter.emit(event)
    emitter.emit(event)
    assert len(writer.value.rstrip("\n").split("\n")) == 1


def test_cli_emitter_convenience_methods():
    writer = StringIO()
    emitter = CliEmitter(writer, OutputFormat.JSON)
    emitter.emit_log(LogLevel.INFO, "starting")
    emitter.emit_result({"ok": True})
    lines = writer.getvalue().splitlines()
    assert len(lines) == 2
    assert '"kind":"log"' in lines[0]
    assert '"kind":"result"' in lines[1]


def test_cli_emitter_with_log_fields_provider():
    writer = StringIO()
    def log_fields():
        return {"source": "test"}
    emitter = CliEmitter(writer, OutputFormat.JSON, log_fields=log_fields)
    emitter.emit_log(LogLevel.INFO, "test message")
    lines = writer.getvalue().splitlines()
    parsed = json.loads(lines[0])
    assert parsed["log"]["source"] == "test"
    assert parsed["log"]["message"] == "test message"


# ── version helpers ───────────────────────────────────────────────────────────

def test_build_cli_version_standard_shape():
    v = build_cli_version("1.2.3")
    assert v["kind"] == "result"
    assert v["result"]["version"] == "1.2.3"
    # 0.16 spec: all events have trace by default
    assert v["trace"] == {}


def test_cli_render_version_is_conventional_by_default():
    assert cli_render_version("agent-cli", "1.2.3") == "agent-cli 1.2.3\n"


def test_cli_render_version_can_render_json():
    out = cli_render_version("agent-cli", "1.2.3", OutputFormat.JSON)
    assert out.endswith("\n")
    assert '"kind":"result"' in out
    assert '"version":"1.2.3"' in out


def test_cli_handle_version_is_conventional_by_default():
    assert cli_handle_version_or_continue(["--version"], "agent-cli", "1.2.3") == "agent-cli 1.2.3\n"


def test_cli_handle_version_honors_output_flag():
    out = cli_handle_version_or_continue(
        ["--version", "--output", "plain"],
        "agent-cli",
        "1.2.3",
    )
    assert out is not None
    assert "kind=result" in out
    assert "result.version=1.2.3" in out


def test_cli_handle_version_json_alias():
    out = cli_handle_version_or_continue(
        ["--version", "--json"],
        "agent-cli",
        "1.2.3",
    )
    assert out is not None
    assert '"kind":"result"' in out
    assert '"version":"1.2.3"' in out


def test_cli_handle_version_json_alias_conflict():
    with pytest.raises(ValueError, match="conflicting output formats"):
        cli_handle_version_or_continue(
            ["--version", "--json", "--output", "yaml"],
            "agent-cli",
            "1.2.3",
        )


def test_cli_handle_version_returns_none_without_version():
    assert cli_handle_version_or_continue(["ping"], "agent-cli", "1.2.3") is None


def test_cli_handle_version_rejects_invalid_output():
    with pytest.raises(ValueError, match="xml"):
        cli_handle_version_or_continue(
            ["--version", "--output", "xml"],
            "agent-cli",
            "1.2.3",
        )
