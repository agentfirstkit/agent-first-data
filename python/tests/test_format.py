"""Tests for AFDATA output formatting — driven by shared spec/fixtures."""

import json
import os

from agent_first_data import (
    build_json_ok,
    build_json_error,
    build_json,
    RedactionPolicy,
    RedactionOptions,
    OutputStyle,
    OutputOptions,
    redact_secrets_in_place,
    redact_secrets_in_place_with_options,
    redacted_value,
    redacted_value_with,
    redacted_value_with_options,
    redact_url_secrets_with_options,
    output_json,
    output_json_with,
    output_json_with_options,
    output_yaml,
    output_yaml_with_options,
    output_plain,
    output_plain_with_options,
    normalize_utc_offset,
)
from agent_first_data.format import (
    _format_bytes_human,
    _format_with_commas,
    _extract_currency_code,
    parse_size,
)

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "..", "..", "spec", "fixtures")


def _load(name):
    with open(os.path.join(FIXTURES_DIR, name)) as f:
        return json.load(f)


def _redaction_options(case):
    opts = case.get("options", {})
    policy = RedactionPolicy(opts["policy"]) if "policy" in opts else None
    return RedactionOptions(
        policy=policy,
        secret_names=opts.get("secret_names", ()),
    )


# --- Redact fixtures ---


def test_redact_url_fixtures():
    for case in _load("redact_url.json"):
        name = case["name"]
        options = _redaction_options(case)
        got = redact_url_secrets_with_options(case["input"], options)
        assert got == case["expected"], f"[redact_url/{name}] got {got!r}"


def test_redact_fixtures():
    for case in _load("redact.json"):
        name = case["name"]
        inp = json.loads(json.dumps(case["input"]))  # deep copy
        redact_secrets_in_place(inp)
        assert inp == case["expected"], f"[redact/{name}] got {inp}"


def test_redaction_options_fixtures():
    for case in _load("redaction_options.json"):
        name = case["name"]
        options = _redaction_options(case)
        output_options = OutputOptions(redaction=options, style=OutputStyle.Readable)
        expected = case["expected"]

        got = redacted_value_with_options(case["input"], options)
        assert got == expected, f"[redaction_options/{name}] value mismatch: {got}"

        inp = json.loads(json.dumps(case["input"]))
        redact_secrets_in_place_with_options(inp, options)
        assert inp == expected, f"[redaction_options/{name}] in-place mismatch: {inp}"

        got_json = json.loads(output_json_with_options(case["input"], output_options))
        assert got_json == expected, f"[redaction_options/{name}] json mismatch: {got_json}"

        if "expected_yaml" in case:
            got_yaml = output_yaml_with_options(case["input"], output_options)
            assert got_yaml == case["expected_yaml"], f"[redaction_options/{name}] yaml mismatch: {got_yaml!r}"
        if "expected_plain" in case:
            got_plain = output_plain_with_options(case["input"], output_options)
            assert got_plain == case["expected_plain"], f"[redaction_options/{name}] plain mismatch: {got_plain!r}"


# --- Protocol fixtures ---


def test_protocol_fixtures():
    for case in _load("protocol.json"):
        name = case["name"]
        typ = case["type"]
        args = case["args"]
        if typ == "ok":
            result = build_json_ok(args["result"])
        elif typ == "ok_trace":
            result = build_json_ok(args["result"], args["trace"])
        elif typ == "error":
            result = build_json_error(args["message"])
        elif typ == "error_trace":
            result = build_json_error(args["message"], trace=args["trace"])
        elif typ == "error_hint":
            result = build_json_error(args["message"], hint=args.get("hint"))
        elif typ == "error_hint_trace":
            result = build_json_error(args["message"], hint=args.get("hint"), trace=args["trace"])
        elif typ == "status":
            result = build_json(args["code"], args.get("fields"))
        else:
            raise ValueError(f"unknown type: {typ}")

        if "expected" in case:
            assert result == case["expected"], f"[protocol/{name}] got {result}"
        if "expected_contains" in case:
            for k, v in case["expected_contains"].items():
                assert result[k] == v, f"[protocol/{name}] key {k}: got {result.get(k)}"


# --- Helper fixtures ---


def test_helper_fixtures():
    for case in _load("helpers.json"):
        name = case["name"]
        for tc in case["cases"]:
            inp, expected = tc
            if name == "format_bytes_human":
                got = _format_bytes_human(inp)
                assert got == expected, f"[helpers/{name}({inp})] got {got!r}"
            elif name == "format_with_commas":
                got = _format_with_commas(inp)
                assert got == expected, f"[helpers/{name}({inp})] got {got!r}"
            elif name == "extract_currency_code":
                got = _extract_currency_code(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "parse_size":
                got = parse_size(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "normalize_utc_offset":
                got = normalize_utc_offset(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"


def test_output_format_fixtures():
    for case in _load("output_formats.json"):
        name = case["name"]
        inp = json.loads(json.dumps(case["input"]))

        got_json = json.loads(output_json(inp))
        assert got_json == case["expected_json"], f"[output/{name}] json mismatch: {got_json}"

        got_yaml = output_yaml(inp)
        assert got_yaml == case["expected_yaml"], f"[output/{name}] yaml mismatch: {got_yaml!r}"

        got_plain = output_plain(inp)
        assert got_plain == case["expected_plain"], f"[output/{name}] plain mismatch: {got_plain!r}"


def test_output_yaml_raw_keeps_suffix_keys_and_structure():
    options = OutputOptions(
        redaction=RedactionOptions(policy=RedactionPolicy.RedactionTraceOnly),
        style=OutputStyle.Raw,
    )
    out = output_yaml_with_options(
        {
            "code": "result",
            "rows": [{"api_key_secret": "sk-live-1", "duration_ms": 42}],
            "trace": {"request_secret": "top-secret"},
        },
        options,
    )

    assert "rows:\n  -" in out
    assert 'api_key_secret: "sk-live-1"' in out
    assert "duration_ms: 42" in out
    assert 'request_secret: "***"' in out
    assert 'duration: "42ms"' not in out


def test_output_plain_raw_keeps_suffix_keys_and_redacts_trace():
    options = OutputOptions(
        redaction=RedactionOptions(policy=RedactionPolicy.RedactionTraceOnly),
        style=OutputStyle.Raw,
    )
    out = output_plain_with_options(
        {
            "duration_ms": 42,
            "trace": {"request_secret": "top-secret"},
        },
        options,
    )

    assert "duration_ms=42" in out
    assert "trace.request_secret=***" in out
    assert "duration=42ms" not in out


def test_output_with_options_defaults_to_readable_style():
    out = output_yaml_with_options(
        {"duration_ms": 42},
        OutputOptions(
            redaction=RedactionOptions(policy=RedactionPolicy.RedactionNone),
            style=OutputStyle.Readable,
        ),
    )
    assert 'duration: "42ms"' in out
    assert "duration_ms:" not in out


def test_output_json_exception_field_is_readable():
    out = output_json({"error": Exception("timeout")})
    parsed = json.loads(out)
    assert parsed["error"] == "timeout"


def test_output_json_unsupported_value_does_not_leak_secret():
    class SecretRepr:
        def __repr__(self) -> str:
            return "Secret(sk-live-123)"

    out = output_json({"meta": SecretRepr(), "api_key_secret": "sk-live-123"})
    assert "sk-live-123" not in out
    parsed = json.loads(out)
    assert parsed["api_key_secret"] == "***"
    assert parsed["meta"].startswith("<unsupported:")


def test_output_json_circular_reference():
    v = {}
    v["self"] = v
    out = output_json(v)
    parsed = json.loads(out)
    assert parsed["self"] == "<unsupported:circular>"


def test_output_json_with_trace_only_redacts_only_trace():
    out = output_json_with(
        {
            "code": "ok",
            "result": {"api_key_secret": "sk-live-123"},
            "trace": {"request_secret": "top-secret"},
        },
        RedactionPolicy.RedactionTraceOnly,
    )
    parsed = json.loads(out)
    assert parsed["trace"]["request_secret"] == "***"
    assert parsed["result"]["api_key_secret"] == "sk-live-123"


def test_output_json_with_none_keeps_secrets():
    out = output_json_with(
        {"api_key_secret": "sk-live-123"},
        RedactionPolicy.RedactionNone,
    )
    parsed = json.loads(out)
    assert parsed["api_key_secret"] == "sk-live-123"


def test_redacted_value_returns_safe_copy():
    inp = {"api_key_secret": "sk-live-123", "nested": {"token_secret": "tok"}}
    got = redacted_value(inp)
    assert got["api_key_secret"] == "***"
    assert got["nested"]["token_secret"] == "***"
    assert inp["api_key_secret"] == "sk-live-123"


def test_redacted_value_redacts_secret_subtree_by_default():
    inp = {"db_secret": {"password_secret": "real", "host": "localhost"}}
    default = redacted_value(inp)
    assert default["db_secret"] == "***"
