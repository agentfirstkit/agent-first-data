"""Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.

Demonstrates the correct use of: cli_parse_output, cli_parse_log_filters,
cli_output, and build_cli_error.

Run:  PYTHONPATH=. python3 examples/agent_cli.py echo --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --output yaml --log startup,request
Test: PYTHONPATH=. python3 -m pytest examples/agent_cli.py -v
"""

import argparse
import json
import sys

from agent_first_data import (
    OutputFormat,
    build_cli_error,
    cli_output,
    cli_parse_log_filters,
    cli_parse_output,
    output_json,
)


def main() -> None:
    parser = argparse.ArgumentParser(description="Minimal agent-first CLI example")
    parser.add_argument("action", help="Action to perform")
    parser.add_argument("--output", default="json", help="Output format: json, yaml, plain")
    parser.add_argument("--log", default="", help="Log categories (comma-separated)")

    args = parser.parse_args()

    # Step 1: parse --output with shared helper
    try:
        fmt = cli_parse_output(args.output)
    except ValueError as e:
        print(output_json(build_cli_error(str(e))))
        sys.exit(2)

    # Step 2: parse --log with shared helper (trim + lowercase + dedup)
    log = cli_parse_log_filters(args.log.split(",") if args.log else [])

    # Step 3: do work, emit JSONL
    result = {"code": "ok", "action": args.action, "log": log}
    print(cli_output(result, fmt))


# ── Tests (run via: pytest examples/agent_cli.py) ─────────────────────────────


def test_parse_output_all_variants():
    assert cli_parse_output("json") is OutputFormat.JSON
    assert cli_parse_output("yaml") is OutputFormat.YAML
    assert cli_parse_output("plain") is OutputFormat.PLAIN
    import pytest
    with pytest.raises(ValueError):
        cli_parse_output("xml")


def test_parse_log_normalizes():
    assert cli_parse_log_filters(["Startup", " REQUEST ", "startup"]) == ["startup", "request"]


def test_build_cli_error_structure():
    v = build_cli_error("--output: invalid value 'xml'")
    assert v["code"] == "error"
    assert v["error_code"] == "invalid_request"
    assert v["retryable"] is False
    assert v["trace"]["duration_ms"] == 0


def test_cli_output_all_formats():
    v = {"code": "ok"}
    json_out = cli_output(v, OutputFormat.JSON)
    yaml_out = cli_output(v, OutputFormat.YAML)
    plain_out = cli_output(v, OutputFormat.PLAIN)
    assert '"code"' in json_out
    assert yaml_out.startswith("---")
    assert "code=ok" in plain_out


def test_error_round_trip_is_valid_jsonl():
    v = build_cli_error("unknown flag: --foo")
    line = output_json(v)
    parsed = json.loads(line)
    assert parsed["code"] == "error"
    assert "\n" not in line


if __name__ == "__main__":
    main()
