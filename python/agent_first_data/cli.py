"""AFDATA CLI helpers — output format parsing, log filter normalization, error building."""

from __future__ import annotations

import enum
from typing import Any

from agent_first_data.format import output_json, output_yaml, output_plain


class OutputFormat(enum.Enum):
    """Output format for CLI and pipe/MCP modes."""

    JSON = "json"
    YAML = "yaml"
    PLAIN = "plain"


def cli_parse_output(s: str) -> OutputFormat:
    """Parse the --output flag value into an OutputFormat.

    Raises ValueError with a message suitable for build_cli_error on unknown values.

    >>> cli_parse_output("json")
    <OutputFormat.JSON: 'json'>
    >>> cli_parse_output("xml")
    Traceback (most recent call last):
        ...
    ValueError: invalid --output format 'xml': expected json, yaml, or plain
    """
    try:
        return OutputFormat(s)
    except ValueError:
        raise ValueError(
            f"invalid --output format {s!r}: expected json, yaml, or plain"
        )


def cli_parse_log_filters(entries: list[str]) -> list[str]:
    """Normalize --log flag entries: trim, lowercase, deduplicate, remove empty.

    Accepts pre-split entries (e.g. after splitting on comma).

    >>> cli_parse_log_filters(["Query", " error ", "query"])
    ['query', 'error']
    """
    out: list[str] = []
    for entry in entries:
        s = entry.strip().lower()
        if s and s not in out:
            out.append(s)
    return out


def cli_output(value: Any, format: OutputFormat) -> str:
    """Dispatch output formatting by OutputFormat.

    Equivalent to calling output_json, output_yaml, or output_plain directly.

    >>> import json
    >>> v = {"code": "ok"}
    >>> cli_output(v, OutputFormat.JSON).startswith('{"code"')
    True
    """
    if format is OutputFormat.YAML:
        return output_yaml(value)
    if format is OutputFormat.PLAIN:
        return output_plain(value)
    return output_json(value)


def build_cli_error(message: str) -> dict:
    """Build a standard CLI parse error value.

    Use when argument parsing fails or a flag value is invalid.
    Print with output_json and exit with code 2.

    >>> v = build_cli_error("--output: invalid value 'xml'")
    >>> v["code"]
    'error'
    >>> v["error_code"]
    'invalid_request'
    >>> v["retryable"]
    False
    """
    return {
        "code": "error",
        "error_code": "invalid_request",
        "error": message,
        "retryable": False,
        "trace": {"duration_ms": 0},
    }
