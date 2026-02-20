"""Tests for agent_first_data CLI helpers."""
import pytest
from agent_first_data import (
    OutputFormat,
    cli_parse_output,
    cli_parse_log_filters,
    cli_output,
    build_cli_error,
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
    assert v["error_code"] == "invalid_request"
    assert v["error"] == "missing --sql"
    assert v["retryable"] is False
    assert v["trace"]["duration_ms"] == 0


def test_build_cli_error_is_valid_json():
    import json
    v = build_cli_error("oops")
    s = output_json(v)
    parsed = json.loads(s)
    assert parsed["code"] == "error"


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
