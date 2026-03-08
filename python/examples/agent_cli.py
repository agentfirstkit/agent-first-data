"""Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.

Demonstrates the correct use of: cli_parse_output, cli_parse_log_filters,
cli_output, build_cli_error, --dry-run, and error hints.

Run:  PYTHONPATH=. python3 examples/agent_cli.py echo --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --dry-run --output yaml
      PYTHONPATH=. python3 examples/agent_cli.py ping --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --output yaml --log startup,request
Test: PYTHONPATH=. python3 -m pytest examples/agent_cli.py -v
"""

import argparse
import json
import sys

from agent_first_data import (
    OutputFormat,
    build_cli_error,
    build_json,
    build_json_error,
    build_json_ok,
    cli_output,
    cli_parse_log_filters,
    cli_parse_output,
    output_json,
)

VALID_ACTIONS = ["echo", "ping"]


def main() -> None:
    parser = argparse.ArgumentParser(description="Minimal agent-first CLI example")
    parser.add_argument("action", help="Action to perform (echo, ping)")
    parser.add_argument("--output", default="json", help="Output format: json, yaml, plain")
    parser.add_argument("--dry-run", action="store_true", help="Preview without executing")
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

    # Step 3: validate action — demonstrate build_cli_error with hint
    if args.action not in VALID_ACTIONS:
        msg = f"unknown action: {args.action}"
        hint = f"valid actions: {', '.join(VALID_ACTIONS)}"
        print(output_json(build_cli_error(msg, hint=hint)))
        sys.exit(2)

    # Step 4: --dry-run → preview without executing
    if args.dry_run:
        preview = build_json("dry_run", {"action": args.action, "log": log}, trace={"duration_ms": 0})
        print(cli_output(preview, fmt))
        return

    # Step 5: do work — demonstrate build_json_error with hint on failure
    if args.action == "ping":
        err = build_json_error("ping target not configured", hint="set PING_HOST or pass --host", trace={"duration_ms": 0})
        print(cli_output(err, fmt))
        sys.exit(1)

    result = build_json_ok({"action": args.action, "log": log})
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


def test_build_cli_error_with_hint():
    v = build_cli_error("unknown action: foo", hint="valid actions: echo, ping")
    assert v["code"] == "error"
    assert v["hint"] == "valid actions: echo, ping"


def test_build_json_error_with_hint():
    v = build_json_error("not configured", hint="set PING_HOST")
    assert v["code"] == "error"
    assert v["error"] == "not configured"
    assert v["hint"] == "set PING_HOST"


def test_build_json_error_without_hint_has_no_hint_key():
    v = build_json_error("something failed")
    assert "hint" not in v


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
