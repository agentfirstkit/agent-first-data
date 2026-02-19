"""AFDATA-compliant structured logging.

Outputs log events using agent-first-data formatting functions:
- JSON: single-line JSONL via output_json (secrets redacted, original keys)
- Plain: single-line logfmt via output_plain (keys stripped, values formatted)
- YAML: multi-line via output_yaml (keys stripped, values formatted)

Span fields are carried via contextvars (async-safe).

Usage:
    from agent_first_data.afdata_logging import init_json, init_plain, init_yaml, span
    import logging

    init_json("INFO")   # or init_plain("INFO") or init_yaml("DEBUG")
    logger = logging.getLogger("myapp")

    with span(request_id="abc-123"):
        logger.info("Processing")
"""

import logging
import sys
from contextvars import ContextVar, Token
from typing import Any

from agent_first_data.format import output_json, output_plain, output_yaml

_span_fields: ContextVar[dict[str, Any]] = ContextVar("afdata_span", default={})

_LEVEL_TO_CODE = {
    "CRITICAL": "error",
    "ERROR": "error",
    "WARNING": "warn",
    "WARN": "warn",
    "INFO": "info",
    "DEBUG": "debug",
    "NOTSET": "trace",
}


class AfdataHandler(logging.Handler):
    """Logging handler that outputs AFDATA-compliant log lines to stdout.

    Formats output using the library's own output_json/output_plain/output_yaml.
    """

    def __init__(self, format: str = "json") -> None:
        super().__init__()
        if format not in ("json", "plain", "yaml"):
            raise ValueError(f"Unknown format: {format!r}, expected json/plain/yaml")
        self._format = format

    def emit(self, record: logging.LogRecord) -> None:
        entry: dict[str, Any] = {
            "timestamp_epoch_ms": int(record.created * 1000),
            "message": record.getMessage(),
            "target": record.name,
        }

        # Span fields (from contextvars, async-safe)
        span_data = _span_fields.get()
        if span_data:
            entry.update(span_data)

        # Event fields (passed via extra= in logging calls)
        has_code = False
        extra = getattr(record, "_afdata_fields", None)
        if extra:
            for k, v in extra.items():
                if k == "code":
                    has_code = True
                entry[k] = v

        # Default code from level
        if not has_code:
            entry["code"] = _LEVEL_TO_CODE.get(record.levelname, "info")

        # Format using the library's own output functions
        if self._format == "plain":
            line = output_plain(entry)
        elif self._format == "yaml":
            line = output_yaml(entry)
        else:
            line = output_json(entry)

        sys.stdout.write(line + "\n")
        sys.stdout.flush()


# Keep old name as alias for backwards compat
AfdataJsonHandler = AfdataHandler


class _AfdataLoggerAdapter(logging.LoggerAdapter):
    """Logger adapter that passes extra fields to AfdataHandler."""

    def process(self, msg: str, kwargs: Any) -> tuple[str, Any]:
        extra = kwargs.get("extra", {})
        if self.extra:
            merged = {**self.extra, **extra}
        else:
            merged = extra
        kwargs["extra"] = {"_afdata_fields": merged}
        return msg, kwargs


def _init_with_format(format: str, level: str = "INFO") -> None:
    handler = AfdataHandler(format=format)
    root = logging.getLogger()
    root.handlers = [handler]
    root.setLevel(getattr(logging, level.upper(), logging.INFO))


def init_json(level: str = "INFO") -> None:
    """Initialize the root logger with AFDATA JSON output to stdout."""
    _init_with_format("json", level)


def init_plain(level: str = "INFO") -> None:
    """Initialize the root logger with AFDATA plain/logfmt output to stdout."""
    _init_with_format("plain", level)


def init_yaml(level: str = "INFO") -> None:
    """Initialize the root logger with AFDATA YAML output to stdout."""
    _init_with_format("yaml", level)


def get_logger(name: str, **fields: Any) -> logging.LoggerAdapter:
    """Get a logger with optional default fields.

    Fields passed here appear on every log line from this logger.
    Use for per-module or per-component fields.
    """
    base = logging.getLogger(name)
    return _AfdataLoggerAdapter(base, fields)


class span:
    """Context manager that adds fields to all log events within the block.

    Spans nest: inner spans inherit and can override outer span fields.
    Works with both sync and async code (uses contextvars).

    Usage:
        with span(request_id="abc-123", method="GET"):
            logger.info("Handling request")
    """

    def __init__(self, **fields: Any) -> None:
        self.fields = fields
        self._token: Token[dict[str, Any]] | None = None

    def __enter__(self) -> "span":
        current = _span_fields.get()
        self._token = _span_fields.set({**current, **self.fields})
        return self

    def __exit__(self, *_: Any) -> None:
        if self._token is not None:
            _span_fields.reset(self._token)
