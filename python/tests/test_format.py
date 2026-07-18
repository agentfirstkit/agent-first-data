"""Tests for AFDATA output formatting — driven by shared spec/fixtures."""

import json
import os

import pytest

from agent_first_data import (
    json_result,
    json_error,
    json_progress,
    json_log,
    LogLevel,
    EventBuildError,
    validate_protocol_event,
    validate_protocol_stream,
    EventDecodeError,
    DecodedResult,
    DecodedError,
    DecodedProgress,
    DecodedLog,
    decode_protocol_event,
    RedactionPolicy,
    PlainStyle,
    OutputOptions,
    OutputFormat,
    redacted_value,
    redact_url_secrets,
    render,
    normalize_utc_offset,
    is_valid_rfc3339_date,
    is_valid_rfc3339_time,
    is_valid_rfc3339,
    is_valid_bcp47,
)
from agent_first_data.format import (
    _format_bytes_human,
    _format_with_commas,
    _extract_currency_code,
)

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "..", "..", "spec", "fixtures")


def _load(name):
    with open(os.path.join(FIXTURES_DIR, name)) as f:
        return json.load(f)


def _redaction_options(case):
    opts = case.get("options", {})
    policy = RedactionPolicy(opts["policy"]) if "policy" in opts else None
    return OutputOptions(policy=policy, secret_names=opts.get("secret_names", ()))


# --- Redact fixtures ---


def test_redact_url_fixtures():
    for case in _load("redact_url.json"):
        name = case["name"]
        options = _redaction_options(case)
        got = redact_url_secrets(case["input"], secret_names=options.secret_names)
        assert got == case["expected"], f"[redact_url/{name}] got {got!r}"


def test_redaction_options_fixtures():
    for case in _load("redaction_options.json"):
        name = case["name"]
        output_options = _redaction_options(case)
        expected = case["expected"]

        got = redacted_value(case["input"], secret_names=output_options.secret_names, policy=output_options.policy)
        assert got == expected, f"[redaction_options/{name}] value mismatch: {got}"

        got_json = json.loads(render(case["input"], OutputFormat.JSON, options=output_options))
        assert got_json == expected, f"[redaction_options/{name}] json mismatch: {got_json}"

        if "expected_yaml" in case:
            got_yaml = render(case["input"], OutputFormat.YAML, options=output_options)
            assert got_yaml == case["expected_yaml"], f"[redaction_options/{name}] yaml mismatch: {got_yaml!r}"
        if "expected_plain" in case:
            got_plain = render(case["input"], OutputFormat.PLAIN, options=output_options)
            assert got_plain == case["expected_plain"], f"[redaction_options/{name}] plain mismatch: {got_plain!r}"


def test_security_fixtures():
    fixture = _load("security.json")
    for case in fixture["redaction_cases"]:
        name = case["name"]
        output_options = _redaction_options(case)
        assert redacted_value(case["input"], secret_names=output_options.secret_names, policy=output_options.policy) == case["expected"]
        outputs = (
            render(case["input"], OutputFormat.JSON, options=output_options),
            render(case["input"], OutputFormat.YAML, options=output_options),
            render(case["input"], OutputFormat.PLAIN, options=output_options),
        )
        for output in outputs:
            for needle in case["must_contain"]:
                assert needle in output, f"[security/{name}] output missing {needle!r}: {output}"
            for needle in case["must_not_contain"]:
                assert needle not in output, f"[security/{name}] output leaked {needle!r}: {output}"


# --- Protocol fixtures ---


def _build_protocol_case_event(typ, args):
    """Build an Event from a protocol.json builder case.

    args vocabulary: "result" (payload), "code"+"message" (error),
    "hint" (-> .hint()), "retryable" (bool -> .retryable_if()),
    "fields" (object -> .fields()), "trace" (object -> .trace()), and complete
    progress/log payloads.
    """
    kind = typ.split("_", 1)[0]

    if kind == "result":
        builder = json_result(args["result"])
    elif kind == "error":
        builder = json_error(args["code"], args["message"])
        if "hint" in args:
            builder = builder.hint(args["hint"])
        if "retryable" in args:
            builder = builder.retryable_if(args["retryable"])
    elif kind == "progress":
        builder = json_progress({"message": args["message"], **args.get("fields", {})})
    elif kind == "log":
        builder = json_log({"level": args["level"], "message": args["message"], **args.get("fields", {})})
    else:
        raise ValueError(f"unknown fixture type: {typ}")

    if "fields" in args and kind not in ("progress", "log"):
        builder = builder.fields(args["fields"])
    if "trace" in args:
        builder = builder.trace(args["trace"])
    return builder.build()


def test_protocol_fixtures():
    for case in _load("protocol.json"):
        name = case["name"]
        if "invalid" in case:
            try:
                validate_protocol_event(case["invalid"], strict=False)
            except ValueError:
                pass
            else:
                raise AssertionError(f"[protocol/{name}] invalid event unexpectedly passed")
            continue

        result = _build_protocol_case_event(case["type"], case["args"]).to_dict()
        validate_protocol_event(result, strict=False)
        validate_protocol_event(result, strict=True)
        expected = case["expected"]
        assert result == expected, (
            f"[protocol/{name}] deep equality failed:\n"
            f"  got:      {result}\n"
            f"  expected: {expected}"
        )


def test_protocol_stream_fixtures():
    for case in _load("protocol_streams.json"):
        name = case["name"]
        valid = case["valid"]
        try:
            validate_protocol_stream(case["events"], strict=False)
        except ValueError as exc:
            assert not valid, f"[protocol_streams/{name}] unexpected error: {exc}"
        else:
            assert valid, f"[protocol_streams/{name}] invalid stream unexpectedly passed"


def test_protocol_strict_fixtures():
    for case in _load("protocol_strict.json"):
        try:
            validate_protocol_stream(case["events"], strict=True)
        except ValueError:
            assert not case["valid"], case["name"]
        else:
            assert case["valid"], case["name"]


def test_error_builder_rejects_reserved_extension_fields():
    # 0.16 spec: reserved fields must not be writable, errors are collected at build()
    try:
        json_error("explicit", "message").fields({"code": "wrong", "message": "wrong", "hint": "wrong", "detail": 1}).build()
        assert False, "should have raised EventBuildError"
    except Exception as e:
        assert "cannot override reserved field" in str(e)


def test_error_builder_empty_code_deferred_to_build():
    # L1: ErrorBuilder is the only fallible builder. json_error("", ...) must not
    # fail eagerly at construction; the empty-code error is deferred to build().
    builder = json_error("", "message")
    with pytest.raises(EventBuildError, match="error code must not be empty"):
        builder.build()


def test_error_builder_empty_message_deferred_to_build():
    builder = json_error("code", "")
    with pytest.raises(EventBuildError, match="error message must not be empty"):
        builder.build()


@pytest.mark.parametrize(
    "make_builder",
    [
        lambda: json_result("ok"),
        lambda: json_progress({"message": "working"}),
        lambda: json_log({"level": "info", "message": "hi"}),
    ],
    ids=["result", "progress", "log"],
)
def test_non_error_builders_never_raise_on_build(make_builder):
    # L1: ResultBuilder/ProgressBuilder/LogBuilder.build() must never raise, even
    # after a non-dict trace() — the invalid trace is stored verbatim and only
    # surfaces later at validate_protocol_event, not at build time.
    event = make_builder().trace("not-an-object").build()
    assert event.to_dict()["trace"] == "not-an-object"

    # A dict trace still builds normally.
    event2 = make_builder().trace({"request_id": "abc"}).build()
    assert event2.to_dict()["trace"] == {"request_id": "abc"}


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
            elif name == "normalize_utc_offset":
                got = normalize_utc_offset(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "is_valid_rfc3339_date":
                got = is_valid_rfc3339_date(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "is_valid_rfc3339_time":
                got = is_valid_rfc3339_time(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "is_valid_bcp47":
                got = is_valid_bcp47(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"
            elif name == "is_valid_rfc3339":
                got = is_valid_rfc3339(inp)
                assert got == expected, f"[helpers/{name}({inp!r})] got {got!r}"


def test_output_format_fixtures():
    for case in _load("output_formats.json"):
        name = case["name"]
        inp = json.loads(json.dumps(case["input"]))

        got_json = json.loads(render(inp, OutputFormat.JSON))
        assert got_json == case["expected_json"], f"[output/{name}] json mismatch: {got_json}"

        got_yaml = render(inp, OutputFormat.YAML)
        assert got_yaml == case["expected_yaml"], f"[output/{name}] yaml mismatch: {got_yaml!r}"

        got_plain = render(inp, OutputFormat.PLAIN)
        assert got_plain == case["expected_plain"], f"[output/{name}] plain mismatch: {got_plain!r}"


def test_render_yaml_raw_keeps_suffix_keys_and_structure():
    options = OutputOptions(
        policy=RedactionPolicy.TraceOnly,
        style=PlainStyle.Raw,
    )
    out = render(
        {
            "code": "result",
            "rows": [{"api_key_secret": "sk-live-1", "duration_ms": 42}],
            "trace": {"request_secret": "top-secret"},
        },
        OutputFormat.YAML,
        options=options,
    )

    assert "rows:\n  -" in out
    assert 'api_key_secret: "sk-live-1"' in out
    assert "duration_ms: 42" in out
    assert 'request_secret: "***"' in out
    assert 'duration: "42ms"' not in out


def test_render_plain_raw_keeps_suffix_keys_and_redacts_trace():
    options = OutputOptions(
        policy=RedactionPolicy.TraceOnly,
        style=PlainStyle.Raw,
    )
    out = render(
        {
            "duration_ms": 42,
            "trace": {"request_secret": "top-secret"},
        },
        OutputFormat.PLAIN,
        options=options,
    )

    assert "duration_ms=42" in out
    assert "trace.request_secret=***" in out
    assert "duration=42ms" not in out


def test_render_yaml_ignores_output_style():
    """YAML no longer branches on PlainStyle: Readable and Raw produce identical,
    structure-preserving output (only `plain` still varies by style)."""
    value = {"duration_ms": 42, "api_key_secret": "sk-live-1"}
    readable = render(
        value,
        OutputFormat.YAML,
        options=OutputOptions(policy=RedactionPolicy.Off, style=PlainStyle.Readable),
    )
    raw = render(
        value,
        OutputFormat.YAML,
        options=OutputOptions(policy=RedactionPolicy.Off, style=PlainStyle.Raw),
    )
    assert readable == raw
    assert "duration_ms: 42" in readable
    assert 'api_key_secret: "sk-live-1"' in readable
    assert "duration:" not in readable


def test_render_json_exception_field_is_readable():
    out = render({"error": Exception("timeout")}, OutputFormat.JSON)
    parsed = json.loads(out)
    assert parsed["error"] == "timeout"


def test_render_json_unsupported_value_does_not_leak_secret():
    class SecretRepr:
        def __repr__(self) -> str:
            return "Secret(sk-live-123)"

    out = render({"meta": SecretRepr(), "api_key_secret": "sk-live-123"}, OutputFormat.JSON)
    assert "sk-live-123" not in out
    parsed = json.loads(out)
    assert parsed["api_key_secret"] == "***"
    assert parsed["meta"].startswith("<unsupported:")


def test_render_json_circular_reference():
    v = {}
    v["self"] = v
    out = render(v, OutputFormat.JSON)
    parsed = json.loads(out)
    assert parsed["self"] == "<unsupported:circular>"


def test_render_json_with_trace_only_redacts_only_trace():
    out = render(
        {
            "code": "ok",
            "result": {"api_key_secret": "sk-live-123"},
            "trace": {"request_secret": "top-secret"},
        },
        OutputFormat.JSON,
        options=OutputOptions.for_policy(RedactionPolicy.TraceOnly),
    )
    parsed = json.loads(out)
    assert parsed["trace"]["request_secret"] == "***"
    assert parsed["result"]["api_key_secret"] == "sk-live-123"


def test_render_json_with_none_keeps_secrets():
    out = render(
        {"api_key_secret": "sk-live-123"},
        OutputFormat.JSON,
        options=OutputOptions.for_policy(RedactionPolicy.Off),
    )
    parsed = json.loads(out)
    assert parsed["api_key_secret"] == "sk-live-123"


def test_output_options_for_policy_sets_only_redaction_policy():
    options = OutputOptions.for_policy(RedactionPolicy.Off)
    assert options.policy == RedactionPolicy.Off
    assert options.secret_names == ()
    assert options.style == PlainStyle.Readable


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


def test_max_depth_marker_is_not_secret_redaction_marker():
    inp = "leaf"
    for _ in range(300):
        inp = {"next": inp}
    out = render(inp, OutputFormat.JSON)
    assert "<afdata:max-depth>" in out
    assert "***" not in out


# --- decode_protocol_event ---


def test_decode_protocol_event_result():
    event = json_result({"rows": 2}).build()
    decoded = decode_protocol_event(render(event.to_dict(), OutputFormat.JSON))
    assert isinstance(decoded, DecodedResult)
    assert decoded.result == {"rows": 2}
    assert decoded.trace == {}


def test_decode_protocol_event_error():
    event = json_error("not_found", "missing").hint("check the id").field("id", "abc").retryable().build()
    decoded = decode_protocol_event(render(event.to_dict(), OutputFormat.JSON))
    assert isinstance(decoded, DecodedError)
    assert decoded.code == "not_found"
    assert decoded.message == "missing"
    assert decoded.retryable is True
    assert decoded.hint == "check the id"
    assert decoded.fields == {"id": "abc"}
    assert decoded.trace == {}


def test_decode_protocol_event_progress():
    event = json_progress({"message": "halfway", "percent": 50}).build()
    decoded = decode_protocol_event(render(event.to_dict(), OutputFormat.JSON))
    assert isinstance(decoded, DecodedProgress)
    assert decoded.progress == {"message": "halfway", "percent": 50}
    assert decoded.trace == {}


def test_decode_protocol_event_log():
    event = json_log({"level": "warn", "message": "slow query", "duration_ms": 900}).build()
    decoded = decode_protocol_event(render(event.to_dict(), OutputFormat.JSON))
    assert isinstance(decoded, DecodedLog)
    assert decoded.log == {"level": "warn", "message": "slow query", "duration_ms": 900}
    assert decoded.trace == {}


def test_decode_protocol_event_invalid_json_raises():
    import pytest

    with pytest.raises(EventDecodeError):
        decode_protocol_event("not json")


def test_decode_protocol_event_invalid_envelope_raises():
    import pytest

    with pytest.raises(EventDecodeError):
        decode_protocol_event(json.dumps({"kind": "result"}))


def test_decode_protocol_event_fails_strict_validation():
    import pytest

    # Missing trace fails the strict profile even though the shape is otherwise valid.
    with pytest.raises(EventDecodeError):
        decode_protocol_event(json.dumps({"kind": "result", "result": {}}))


# --- Number literal fidelity (shared spec/fixtures/number_fidelity.json) ---
#
# Regression guard for decode_protocol_event's parse_int/parse_float hooks
# (_RawNumber, only needed for floats and the "-0" integer edge case --
# every other integer is already arbitrary-precision via plain Python int)
# and for _encode_json_lossless/_yaml_scalar/_plain_scalar's _RawNumber
# handling in format.py.


def test_number_fidelity_fixtures():
    for case in _load("number_fidelity.json"):
        name = case["name"]
        decoded = decode_protocol_event(case["input_line"])
        assert isinstance(decoded, DecodedResult), f"[number_fidelity/{name}] expected DecodedResult"

        got_json = render(decoded.result, OutputFormat.JSON)
        assert got_json == case["expected_json"], f"[number_fidelity/{name}] json mismatch: {got_json!r}"

        if "expected_yaml" in case:
            got_yaml = render(decoded.result, OutputFormat.YAML)
            assert got_yaml == case["expected_yaml"], f"[number_fidelity/{name}] yaml mismatch: {got_yaml!r}"


def test_number_fidelity_does_not_regress_ordinary_decoded_numbers_in_plain_output():
    # decode_protocol_event wraps every decoded float (not ints -- those are
    # already arbitrary-precision Python int) in _RawNumber, including small
    # ordinary ones; _try_process_field must normalize back to a float for
    # Plain's suffix arithmetic or this would silently stop formatting a
    # decoded cpu_percent for any event with a float suffix field.
    line = json.dumps({"kind": "result", "result": {"duration_ms": 42, "size_bytes": 5242880, "cpu_percent": 85.5}, "trace": {}})
    decoded = decode_protocol_event(line)
    plain = render(decoded.result, OutputFormat.PLAIN)
    assert plain == "cpu=85.5% duration=42ms size=5.0MiB"
