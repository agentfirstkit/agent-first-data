"""Agent-First Data (AFD) — suffix-driven output formatting and protocol templates."""

from agent_first_data.format import (
    OutputFormat,
    to_yaml,
    to_plain,
    redact_secrets,
    ok,
    ok_trace,
    error,
    error_trace,
    startup,
    status,
    parse_size,
)

__all__ = [
    "OutputFormat",
    "to_yaml",
    "to_plain",
    "redact_secrets",
    "ok",
    "ok_trace",
    "error",
    "error_trace",
    "startup",
    "status",
    "parse_size",
]
