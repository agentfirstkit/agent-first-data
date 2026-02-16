"""Agent-First Data (AFD) — suffix-driven output formatting and protocol templates."""

from agent_first_data.format import (
    build_json_startup,
    build_json_ok,
    build_json_error,
    build_json,
    output_json,
    output_yaml,
    output_plain,
    internal_redact_secrets,
    parse_size,
)

__all__ = [
    "build_json_startup",
    "build_json_ok",
    "build_json_error",
    "build_json",
    "output_json",
    "output_yaml",
    "output_plain",
    "internal_redact_secrets",
    "parse_size",
]
