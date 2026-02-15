"""Tests for AFD output formatting — driven by shared spec/fixtures."""

import json
import os

from agent_first_data import (
    to_yaml,
    to_plain,
    redact_secrets,
    ok,
    ok_trace,
    error,
    error_trace,
    startup,
    status,
    OutputFormat,
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


# --- Plain fixtures ---


def test_plain_fixtures():
    for case in _load("plain.json"):
        name = case["name"]
        plain = to_plain(case["input"])
        for s in case["contains"]:
            assert s in plain, f"[plain/{name}] expected {s!r} in {plain!r}"
        for s in case.get("not_contains", []):
            assert s not in plain, f"[plain/{name}] unexpected {s!r} in {plain!r}"


# --- YAML fixtures ---


def test_yaml_fixtures():
    for case in _load("yaml.json"):
        name = case["name"]
        yaml = to_yaml(case["input"])
        if "starts_with" in case:
            assert yaml.startswith(case["starts_with"]), f"[yaml/{name}] starts_with failed"
        for s in case.get("contains", []):
            assert s in yaml, f"[yaml/{name}] expected {s!r} in {yaml!r}"


# --- Redact fixtures ---


def test_redact_fixtures():
    for case in _load("redact.json"):
        name = case["name"]
        inp = json.loads(json.dumps(case["input"]))  # deep copy
        redact_secrets(inp)
        assert inp == case["expected"], f"[redact/{name}] got {inp}"


# --- Protocol fixtures ---


def test_protocol_fixtures():
    for case in _load("protocol.json"):
        name = case["name"]
        typ = case["type"]
        args = case["args"]
        if typ == "ok":
            result = ok(args["result"])
        elif typ == "ok_trace":
            result = ok_trace(args["result"], args["trace"])
        elif typ == "error":
            result = error(args["message"])
        elif typ == "error_trace":
            result = error_trace(args["message"], args["trace"])
        elif typ == "startup":
            result = startup(args["config"], args["args"], args["env"])
        elif typ == "status":
            result = status(args["code"], args.get("fields"))
        else:
            raise ValueError(f"unknown type: {typ}")

        if "expected" in case:
            assert result == case["expected"], f"[protocol/{name}] got {result}"
        if "expected_contains" in case:
            for k, v in case["expected_contains"].items():
                assert result[k] == v, f"[protocol/{name}] key {k}: got {result.get(k)}"


# --- Exact fixtures ---


def test_exact_fixtures():
    for case in _load("exact.json"):
        name = case["name"]
        fmt = case["format"]
        expected = case["expected"]
        if fmt == "plain":
            got = to_plain(case["input"])
        elif fmt == "yaml":
            got = to_yaml(case["input"])
        else:
            raise ValueError(f"unknown format: {fmt}")
        assert got == expected, f"[exact/{name}]\ngot:  {got!r}\nwant: {expected!r}"


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


# --- OutputFormat (not in fixtures — format-specific) ---


def test_output_format_json():
    assert OutputFormat.JSON.format({"status": "ok"}) == '{"status":"ok"}'


def test_output_format_yaml():
    out = OutputFormat.YAML.format({"status": "ok"})
    assert out.startswith("---\n")
    assert 'status: "ok"' in out


def test_output_format_plain():
    assert OutputFormat.PLAIN.format({"status": "ok"}) == "status: ok"
