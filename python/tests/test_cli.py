"""Tests for agent_first_data CLI helpers."""
import pytest
from agent_first_data import (
    OutputFormat,
    OutputStyle,
    OutputOptions,
    cli_parse_output,
    cli_parse_log_filters,
    cli_output,
    cli_output_with_options,
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
    assert cli_parse_log_filters(["  Query  ", "ERROR"]) == ["query", "error"]


def test_parse_log_filters_deduplicates():
    assert cli_parse_log_filters(["query", "error", "Query", "query"]) == ["query", "error"]


def test_parse_log_filters_removes_empty():
    assert cli_parse_log_filters(["", "query", "  "]) == ["query"]


def test_parse_log_filters_empty_list():
    assert cli_parse_log_filters([]) == []


def test_parse_log_filters_preserves_order():
    assert cli_parse_log_filters(["startup", "request", "retry"]) == ["startup", "request", "retry"]


# ── build_cli_error ───────────────────────────────────────────────────────────

def test_build_cli_error_required_fields():
    v = build_cli_error("missing --sql")
    assert v["code"] == "error"
    assert v["error"] == "missing --sql"
    assert "error_code" not in v
    assert "retryable" not in v
    assert "trace" not in v


def test_build_cli_error_is_valid_json():
    import json
    v = build_cli_error("oops")
    s = output_json(v)
    parsed = json.loads(s)
    assert parsed["code"] == "error"


def test_build_cli_error_with_hint():
    v = build_cli_error("bad flag", hint="try --help")
    assert v["hint"] == "try --help"


def test_build_cli_error_without_hint_has_no_hint_key():
    v = build_cli_error("oops")
    assert "hint" not in v


# ── cli_output ────────────────────────────────────────────────────────────────

def test_cli_output_dispatches_json():
    v = {"code": "ok", "size_bytes": 1024}
    out = cli_output(v, OutputFormat.JSON)
    assert "size_bytes" in out   # json: raw keys, no suffix processing
    assert "\n" not in out


def test_cli_output_dispatches_yaml():
    v = {"code": "ok", "size_bytes": 1024}
    out = cli_output(v, OutputFormat.YAML)
    assert out.startswith("---")
    assert "size:" in out        # yaml: suffix stripped


def test_cli_output_dispatches_plain():
    v = {"code": "ok"}
    out = cli_output(v, OutputFormat.PLAIN)
    assert "\n" not in out
    assert "code=ok" in out


def test_cli_output_with_options_dispatches_raw_yaml():
    v = {"size_bytes": 1024}
    out = cli_output_with_options(
        v,
        OutputFormat.YAML,
        OutputOptions(style=OutputStyle.Raw),
    )
    assert "size_bytes: 1024" in out
    assert "size:" not in out


# ── version helpers ───────────────────────────────────────────────────────────

def test_build_cli_version_standard_shape():
    v = build_cli_version("1.2.3")
    assert v["code"] == "version"
    assert v["version"] == "1.2.3"
    assert "trace" not in v


def test_cli_render_version_is_conventional_by_default():
    assert cli_render_version("agent-cli", "1.2.3") == "agent-cli 1.2.3\n"


def test_cli_render_version_can_render_json():
    out = cli_render_version("agent-cli", "1.2.3", OutputFormat.JSON)
    assert out.endswith("\n")
    assert '"code":"version"' in out
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
    assert "code=version" in out
    assert "version=1.2.3" in out


def test_cli_handle_version_returns_none_without_version():
    assert cli_handle_version_or_continue(["ping"], "agent-cli", "1.2.3") is None


def test_cli_handle_version_rejects_invalid_output():
    with pytest.raises(ValueError, match="xml"):
        cli_handle_version_or_continue(
            ["--version", "--output", "xml"],
            "agent-cli",
            "1.2.3",
        )
