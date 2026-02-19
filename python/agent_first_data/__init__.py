"""Agent-First Data (AFDATA) — suffix-driven output formatting and protocol templates."""

from agent_first_data.format import (
    build_json_ok,
    build_json_error,
    build_json,
    output_json,
    output_yaml,
    output_plain,
    internal_redact_secrets,
    parse_size,
)

from agent_first_data.afdata_logging import (
    AfdataHandler,
    AfdataJsonHandler,
    init_json as init_logging_json,
    init_plain as init_logging_plain,
    init_yaml as init_logging_yaml,
    get_logger,
    span,
)

__all__ = [
    "build_json_ok",
    "build_json_error",
    "build_json",
    "output_json",
    "output_yaml",
    "output_plain",
    "internal_redact_secrets",
    "parse_size",
    "AfdataHandler",
    "AfdataJsonHandler",
    "init_logging_json",
    "init_logging_plain",
    "init_logging_yaml",
    "get_logger",
    "span",
]
