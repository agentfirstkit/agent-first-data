"""Tests for AFDATA output formatting — driven by shared spec/fixtures."""

import json
import os

from agent_first_data import (
    build_json_ok,
    build_json_error,
    build_json,
    internal_redact_secrets,
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


# --- Redact fixtures ---


def test_redact_fixtures():
    for case in _load("redact.json"):
        name = case["name"]
        inp = json.loads(json.dumps(case["input"]))  # deep copy
        internal_redact_secrets(inp)
        assert inp == case["expected"], f"[redact/{name}] got {inp}"


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
            result = build_json_error(args["message"], args["trace"])
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
