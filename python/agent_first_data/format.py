"""AFDATA output formatting and protocol templates.

Protocol builders, value redactors (copy and in-place; cover _secret and
_url fields), output formatters, URL-string redactors (redact_url_secrets),
parse_size, normalize_utc_offset, is_valid_rfc3339_date,
is_valid_rfc3339_time, RedactionPolicy, OutputStyle, and
OutputOptions. Each formatter/redactor concept is a single function taking
a keyword-only ``options`` parameter.
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from enum import Enum, StrEnum
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import unquote_plus


# ═══════════════════════════════════════════
# Public API: Protocol v1 Event Type
# ═══════════════════════════════════════════


class LogLevel(StrEnum):
    """Log level enum for structured logging."""
    DEBUG = "debug"
    INFO = "info"
    WARN = "warn"
    ERROR = "error"


class EventBuildError(Exception):
    """Exception raised when building an Event fails."""
    pass


class Event:
    """Opaque typed event envelope wrapping a validated protocol dict."""

    def __init__(self, envelope: dict) -> None:
        """Private: only builders and validators construct Events."""
        self._envelope = envelope

    def to_dict(self) -> dict:
        """Return the event as a JSON-serializable dict."""
        return self._envelope


# ═══════════════════════════════════════════
# Public API: Fluent Builders
# ═══════════════════════════════════════════


class ResultBuilder:
    """Fluent builder for result events."""

    def __init__(self, result: Any) -> None:
        self._result = result
        self._trace: dict = {}
        self._errors: list[str] = []

    def trace(self, obj: Any) -> ResultBuilder:
        """Set trace context (merged at build time)."""
        if not isinstance(obj, dict):
            self._errors.append("trace must be a JSON object")
        else:
            self._trace = dict(obj)
        return self

    def build(self) -> Event:
        """Build and return the Event, raising EventBuildError if there are errors."""
        if self._errors:
            raise EventBuildError("; ".join(self._errors))
        envelope = {
            "kind": "result",
            "result": self._result,
            "trace": self._trace,
        }
        return Event(envelope)


class ErrorBuilder:
    """Fluent builder for error events."""

    def __init__(self, code: str, message: str) -> None:
        self._code = code
        self._message = message
        self._retryable = False
        self._hint: str | None = None
        self._trace: dict = {}
        self._fields: dict[str, Any] = {}
        self._errors: list[str] = []

    def retryable(self) -> ErrorBuilder:
        """Mark this error as retryable."""
        self._retryable = True
        return self

    def retryable_if(self, flag: bool) -> ErrorBuilder:
        """Set retryable based on a flag."""
        self._retryable = bool(flag)
        return self

    def hint(self, text: str) -> ErrorBuilder:
        """Set a hint string."""
        if text and isinstance(text, str):
            self._hint = text
        return self

    def hint_if_some(self, optional: str | None) -> ErrorBuilder:
        """Set hint only if the optional value is not None/empty."""
        if optional and isinstance(optional, str):
            self._hint = optional
        return self

    def field(self, name: str, value: Any) -> ErrorBuilder:
        """Add a single extension field."""
        if name in ("code", "message", "hint", "retryable"):
            self._errors.append(f"cannot override reserved field '{name}'")
        else:
            self._fields[name] = value
        return self

    def fields(self, mapping: Mapping[str, Any]) -> ErrorBuilder:
        """Add multiple extension fields from a mapping."""
        if not isinstance(mapping, Mapping):
            self._errors.append("fields must be a mapping")
        else:
            for k, v in mapping.items():
                if k in ("code", "message", "hint", "retryable"):
                    self._errors.append(f"cannot override reserved field '{k}'")
                else:
                    self._fields[k] = v
        return self

    def extend(self, value: Any) -> ErrorBuilder:
        """Add extension fields from a dataclass or mapping."""
        if hasattr(value, "__dataclass_fields__"):
            from dataclasses import asdict
            mapping = asdict(value)
        elif isinstance(value, Mapping):
            mapping = value
        else:
            self._errors.append("extend() requires a dataclass or mapping")
            return self

        # Flatten fields the same way as .fields()
        for k, v in mapping.items():
            if k in ("code", "message", "hint", "retryable"):
                self._errors.append(f"cannot override reserved field '{k}'")
            else:
                self._fields[k] = v
        return self

    def trace(self, obj: Any) -> ErrorBuilder:
        """Set trace context."""
        if not isinstance(obj, dict):
            self._errors.append("trace must be a JSON object")
        else:
            self._trace = dict(obj)
        return self

    def build(self) -> Event:
        """Build and return the Event, raising EventBuildError if there are errors."""
        if self._errors:
            raise EventBuildError("; ".join(self._errors))
        error = {"code": self._code, "message": self._message, "retryable": self._retryable}
        if self._hint is not None:
            error["hint"] = self._hint
        error.update(self._fields)
        envelope = {
            "kind": "error",
            "error": error,
            "trace": self._trace,
        }
        return Event(envelope)


class ProgressBuilder:
    """Fluent builder for progress events."""

    def __init__(self, message: str) -> None:
        self._message = message
        self._trace: dict = {}
        self._fields: dict[str, Any] = {}
        self._errors: list[str] = []

    def field(self, name: str, value: Any) -> ProgressBuilder:
        """Add a single extension field."""
        if name == "message":
            self._errors.append(f"cannot override reserved field '{name}'")
        else:
            self._fields[name] = value
        return self

    def fields(self, mapping: Mapping[str, Any]) -> ProgressBuilder:
        """Add multiple extension fields from a mapping."""
        if not isinstance(mapping, Mapping):
            self._errors.append("fields must be a mapping")
        else:
            for k, v in mapping.items():
                if k == "message":
                    self._errors.append(f"cannot override reserved field '{k}'")
                else:
                    self._fields[k] = v
        return self

    def extend(self, value: Any) -> ProgressBuilder:
        """Add extension fields from a dataclass or mapping."""
        if hasattr(value, "__dataclass_fields__"):
            from dataclasses import asdict
            mapping = asdict(value)
        elif isinstance(value, Mapping):
            mapping = value
        else:
            self._errors.append("extend() requires a dataclass or mapping")
            return self

        for k, v in mapping.items():
            if k == "message":
                self._errors.append(f"cannot override reserved field '{k}'")
            else:
                self._fields[k] = v
        return self

    def trace(self, obj: Any) -> ProgressBuilder:
        """Set trace context."""
        if not isinstance(obj, dict):
            self._errors.append("trace must be a JSON object")
        else:
            self._trace = dict(obj)
        return self

    def build(self) -> Event:
        """Build and return the Event, raising EventBuildError if there are errors."""
        if self._errors:
            raise EventBuildError("; ".join(self._errors))
        progress = {"message": self._message}
        progress.update(self._fields)
        envelope = {
            "kind": "progress",
            "progress": progress,
            "trace": self._trace,
        }
        return Event(envelope)


class LogBuilder:
    """Fluent builder for log events."""

    def __init__(self, level: LogLevel | str, message: str) -> None:
        if isinstance(level, str):
            level = LogLevel(level)
        self._level = level
        self._message = message
        self._trace: dict = {}
        self._fields: dict[str, Any] = {}
        self._errors: list[str] = []

    def field(self, name: str, value: Any) -> LogBuilder:
        """Add a single extension field."""
        if name in ("message", "level", "code"):
            self._errors.append(f"cannot override reserved field '{name}'")
        else:
            self._fields[name] = value
        return self

    def fields(self, mapping: Mapping[str, Any]) -> LogBuilder:
        """Add multiple extension fields from a mapping."""
        if not isinstance(mapping, Mapping):
            self._errors.append("fields must be a mapping")
        else:
            for k, v in mapping.items():
                if k in ("message", "level", "code"):
                    self._errors.append(f"cannot override reserved field '{k}'")
                else:
                    self._fields[k] = v
        return self

    def extend(self, value: Any) -> LogBuilder:
        """Add extension fields from a dataclass or mapping."""
        if hasattr(value, "__dataclass_fields__"):
            from dataclasses import asdict
            mapping = asdict(value)
        elif isinstance(value, Mapping):
            mapping = value
        else:
            self._errors.append("extend() requires a dataclass or mapping")
            return self

        for k, v in mapping.items():
            if k in ("message", "level", "code"):
                self._errors.append(f"cannot override reserved field '{k}'")
            else:
                self._fields[k] = v
        return self

    def trace(self, obj: Any) -> LogBuilder:
        """Set trace context."""
        if not isinstance(obj, dict):
            self._errors.append("trace must be a JSON object")
        else:
            self._trace = dict(obj)
        return self

    def build(self) -> Event:
        """Build and return the Event, raising EventBuildError if there are errors."""
        if self._errors:
            raise EventBuildError("; ".join(self._errors))
        log = {"level": self._level.value, "message": self._message}
        log.update(self._fields)
        envelope = {
            "kind": "log",
            "log": log,
            "trace": self._trace,
        }
        return Event(envelope)


def json_result(result: Any) -> ResultBuilder:
    """Create a fluent result builder."""
    return ResultBuilder(result)


def json_error(code: str, message: str) -> ErrorBuilder:
    """Create a fluent error builder."""
    return ErrorBuilder(code, message)


def json_progress(message: str) -> ProgressBuilder:
    """Create a fluent progress builder."""
    return ProgressBuilder(message)


def json_log(level: LogLevel | str, message: str) -> LogBuilder:
    """Create a fluent log builder."""
    return LogBuilder(level, message)


def validate_protocol_event(event: Any, *, strict: bool = True) -> None:
    """Validate one protocol v1 event envelope.

    With ``strict=True`` (the default), also enforces the recommended strict
    protocol profile (required trace, required error.retryable, log/progress
    message and level rules). Pass ``strict=False`` for the plain, lenient
    envelope-shape check only.
    """
    if not isinstance(event, dict):
        raise ValueError("event must be a JSON object")
    kind = event.get("kind")
    if kind not in ("result", "error", "progress", "log"):
        raise ValueError("event.kind must be one of result, error, progress, log")
    if kind not in event:
        raise ValueError(f"event payload field {kind!r} is required")
    for key in event:
        if key not in ("kind", kind, "trace"):
            raise ValueError(f"unexpected top-level field {key!r}")
    if "trace" in event and not isinstance(event["trace"], dict):
        raise ValueError("event.trace must be a JSON object when present")
    if kind == "error":
        _validate_error_payload(event.get("error"))

    if not strict:
        return
    if not isinstance(event.get("trace"), dict):
        raise ValueError("event.trace is required by the strict profile")
    if kind == "error":
        _validate_strict_error_payload(event["error"])
    elif kind == "log":
        _validate_strict_log_payload(event["log"])
    elif kind == "progress":
        _validate_strict_progress_payload(event["progress"])


def _validate_error_payload(error: Any) -> None:
    if not isinstance(error, dict):
        raise ValueError("event.error must be a JSON object")
    code = error.get("code")
    if not isinstance(code, str) or code == "":
        raise ValueError("event.error.code must be a non-empty string")
    message = error.get("message")
    if not isinstance(message, str) or message == "":
        raise ValueError("event.error.message must be a non-empty string")
    if "retryable" in error and not isinstance(error["retryable"], bool):
        raise ValueError("event.error.retryable must be a boolean")
    if "hint" in error and not isinstance(error["hint"], str):
        raise ValueError("event.error.hint must be a string when present")


def validate_protocol_stream(events: Sequence[Any], *, strict: bool = True) -> None:
    """Validate finite CLI lifecycle: (log | progress)* -> exactly one terminal.

    With ``strict=True`` (the default), each event is also checked against the
    recommended strict protocol profile. Pass ``strict=False`` for the plain,
    lenient lifecycle check only.
    """
    terminal_seen = False
    for idx, event in enumerate(events):
        try:
            validate_protocol_event(event, strict=strict)
        except ValueError as exc:
            raise ValueError(f"event {idx}: {exc}") from exc
        kind = event["kind"]
        if kind in ("log", "progress"):
            if terminal_seen:
                raise ValueError(f"event {idx}: non-terminal event after terminal")
        elif kind in ("result", "error"):
            if terminal_seen:
                raise ValueError(f"event {idx}: duplicate terminal event")
            terminal_seen = True
    if not terminal_seen:
        raise ValueError("event stream must contain exactly one terminal result or error")


def _validate_strict_error_payload(error: Any) -> None:
    if not isinstance(error, dict):
        raise ValueError("event.error must be a JSON object in the strict profile")
    # error.code and error.message already validated by validate_protocol_event
    if "retryable" not in error:
        raise ValueError("event.error.retryable is required by the strict profile")
    if not isinstance(error["retryable"], bool):
        raise ValueError("event.error.retryable must be a boolean in the strict profile")


def _validate_strict_log_payload(log: Any) -> None:
    if not isinstance(log, dict):
        raise ValueError("event.log must be a JSON object in the strict profile")
    if "code" in log:
        raise ValueError("event.log must not contain 'code' field in the strict profile")
    _require_non_empty_string(log, "message", "event.log")
    if "level" not in log:
        raise ValueError("event.log.level is required by the strict profile")
    if log.get("level") not in ("debug", "info", "warn", "error"):
        raise ValueError(
            "event.log.level must be one of debug, info, warn, error in the strict profile"
        )


def _validate_strict_progress_payload(progress: Any) -> None:
    if not isinstance(progress, dict):
        raise ValueError("event.progress must be a JSON object in the strict profile")
    _require_non_empty_string(progress, "message", "event.progress")


def _require_non_empty_string(payload: dict, field: str, path: str) -> None:
    value = payload.get(field)
    if not isinstance(value, str) or value == "":
        raise ValueError(
            f"{path}.{field} must be a non-empty string in the strict profile"
        )


# ═══════════════════════════════════════════
# Public API: Reader
# ═══════════════════════════════════════════


class EventDecodeError(Exception):
    """Exception raised when decoding a protocol v1 event line fails."""
    pass


@dataclass(frozen=True)
class DecodedResult:
    """Decoded protocol v1 result event."""

    result: Any
    trace: dict | None = None


@dataclass(frozen=True)
class DecodedError:
    """Decoded protocol v1 error event. ``fields`` holds extension fields
    beyond code/message/retryable/hint."""

    code: str
    message: str
    retryable: bool
    hint: str | None = None
    fields: dict[str, Any] = field(default_factory=dict)
    trace: dict | None = None


@dataclass(frozen=True)
class DecodedProgress:
    """Decoded protocol v1 progress event. ``fields`` holds extension fields
    beyond message."""

    message: str
    fields: dict[str, Any] = field(default_factory=dict)
    trace: dict | None = None


@dataclass(frozen=True)
class DecodedLog:
    """Decoded protocol v1 log event. ``fields`` holds extension fields
    beyond level/message."""

    level: LogLevel
    message: str
    fields: dict[str, Any] = field(default_factory=dict)
    trace: dict | None = None


def decode_protocol_event(
    text: str,
) -> DecodedResult | DecodedError | DecodedProgress | DecodedLog:
    """Parse and strict-validate a single protocol v1 JSON line into a typed event.

    Raises EventDecodeError if ``text`` is not valid JSON or fails strict
    validation.
    """
    try:
        event = json.loads(text)
    except (ValueError, TypeError) as exc:
        raise EventDecodeError(f"invalid JSON: {exc}") from exc

    try:
        validate_protocol_event(event, strict=True)
    except ValueError as exc:
        raise EventDecodeError(str(exc)) from exc

    trace = event.get("trace")
    kind = event["kind"]

    if kind == "result":
        return DecodedResult(result=event["result"], trace=trace)

    if kind == "error":
        error = event["error"]
        extension_keys = ("code", "message", "retryable", "hint")
        fields = {k: v for k, v in error.items() if k not in extension_keys}
        return DecodedError(
            code=error["code"],
            message=error["message"],
            retryable=error["retryable"],
            hint=error.get("hint"),
            fields=fields,
            trace=trace,
        )

    if kind == "progress":
        progress = event["progress"]
        fields = {k: v for k, v in progress.items() if k != "message"}
        return DecodedProgress(message=progress["message"], fields=fields, trace=trace)

    # kind == "log"
    log = event["log"]
    fields = {k: v for k, v in log.items() if k not in ("level", "message")}
    return DecodedLog(
        level=LogLevel(log["level"]), message=log["message"], fields=fields, trace=trace
    )


# ═══════════════════════════════════════════
# Public API: Output Formatters
# ═══════════════════════════════════════════

class RedactionPolicy(str, Enum):
    RedactionTraceOnly = "RedactionTraceOnly"
    RedactionNone = "RedactionNone"


class OutputStyle(str, Enum):
    """Rendering style for YAML and plain output."""

    Readable = "Readable"
    Raw = "Raw"


@dataclass(frozen=True)
class OutputOptions:
    """Output options combining redaction and rendering style."""

    # Exact field-name matches at any nesting level. The same list also matches
    # URL query-parameter names inside _url fields (see redact_url_secrets).
    secret_names: Sequence[str] = ()
    policy: RedactionPolicy | None = None
    style: OutputStyle = OutputStyle.Readable

    @classmethod
    def for_policy(cls, policy: RedactionPolicy) -> OutputOptions:
        """Convenience constructor: output options with only a redaction policy set."""
        return cls(policy=policy)


def output_json(value: Any, *, options: OutputOptions | None = None) -> str:
    """Format as single-line JSON. Secrets redacted, original keys, raw values."""
    output_options = options or OutputOptions()
    return json.dumps(
        redacted_value(value, secret_names=output_options.secret_names, policy=output_options.policy), ensure_ascii=False, separators=(",", ":")
    )


def output_yaml(value: Any, *, options: OutputOptions | None = None) -> str:
    """Format as multi-line YAML. Keys stripped, values formatted, secrets redacted."""
    output_options = options or OutputOptions()
    value = redacted_value(value, secret_names=output_options.secret_names, policy=output_options.policy)
    lines = ["---"]
    if output_options.style is OutputStyle.Raw:
        _render_yaml_raw(value, 0, lines)
    else:
        _render_yaml_processed(value, 0, lines)
    return "\n".join(lines)


def output_plain(value: Any, *, options: OutputOptions | None = None) -> str:
    """Format as single-line logfmt. Keys stripped, values formatted, secrets redacted."""
    output_options = options or OutputOptions()
    value = redacted_value(value, secret_names=output_options.secret_names, policy=output_options.policy)
    pairs: list[tuple[str, str]] = []
    if output_options.style is OutputStyle.Raw:
        _collect_plain_pairs_raw(value, "", pairs)
    else:
        _collect_plain_pairs(value, "", pairs)
    pairs.sort(key=lambda p: _utf16_sort_key(p[0]))
    parts = []
    for k, v in pairs:
        parts.append(f"{_quote_logfmt_key(k)}={_quote_logfmt_value(v)}")
    return " ".join(parts)


# ═══════════════════════════════════════════
# Public API: Redaction & Utility
# ═══════════════════════════════════════════


def redacted_value(value: Any, *, secret_names: Sequence[str] = (), policy: RedactionPolicy | None = None) -> Any:
    """Return a JSON-safe copy with redaction options applied.

    Redacts fields ending in _secret/_SECRET and those listed in secret_names.
    Redacts _url fields' query parameters by the same rules.
    """
    v = _sanitize_for_json(value)
    _apply_redaction(v, secret_names, policy)
    return v


def redact_url_secrets(url: str, *, secret_names: Sequence[str] = ()) -> str:
    """Redact secret components of a single URL string.

    Returns ``url`` with its userinfo password and any ``_secret``-suffixed
    query-parameter values replaced by ``***``. A query parameter is redacted
    iff its (form-decoded) name ends in ``_secret``/``_SECRET`` or matches an
    exact entry in ``secret_names``. The userinfo password
    (``scheme://user:pass@host``) is always redacted as a structural rule.
    Only the secret spans are replaced with ``***``; every other byte is
    preserved. A string that is not a single, whitespace-free,
    scheme-prefixed URL (including a URL embedded in surrounding prose) is
    returned unchanged.
    """
    context = _RedactionContext.from_names(secret_names)
    redacted = _redact_url_in_str(url, context)
    return redacted if redacted is not None else url


def _apply_redaction(value: Any, secret_names: Sequence[str], policy: RedactionPolicy | None) -> None:
    context = _RedactionContext.from_names(secret_names)
    _apply_redaction_policy_with_context(value, policy, context)


def _apply_redaction_policy_with_context(
    value: Any,
    redaction_policy: RedactionPolicy | None,
    context: _RedactionContext,
) -> None:
    if redaction_policy == RedactionPolicy.RedactionTraceOnly:
        if isinstance(value, dict) and "trace" in value:
            _redact_secrets(value["trace"], context)
        return
    if redaction_policy == RedactionPolicy.RedactionNone:
        return
    # Empty/unknown policy falls back to default full redaction.
    _redact_secrets(value, context)


def parse_size(s: str) -> int | None:
    """Parse a human-readable size string into bytes.

    Accepts numbers followed by explicit units. Decimal units are
    B/kB/MB/GB/TB; binary units are KiB/MiB/GiB/TiB. Trims whitespace and
    rejects ambiguous K/M/G/T units.
    """
    _multipliers = {
        "KiB": 1024,
        "MiB": 1024**2,
        "GiB": 1024**3,
        "TiB": 1024**4,
        "kB": 1000,
        "MB": 1000**2,
        "GB": 1000**3,
        "TB": 1000**4,
        "B": 1,
    }
    _max_safe_integer = (1 << 53) - 1
    s = s.strip()
    if not s:
        return None
    matched = next(((unit, mult) for unit, mult in _multipliers.items() if s.endswith(unit)), None)
    if matched is None:
        return None
    unit, mult = matched
    num_str = s[: -len(unit)]
    if not num_str:
        return None
    if not re.fullmatch(r"(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?", num_str):
        return None
    try:
        n = int(num_str)
        if n < 0:
            return None
        if n > _max_safe_integer // mult:
            return None
        return n * mult
    except ValueError:
        pass
    try:
        f = float(num_str)
        if f < 0 or not math.isfinite(f):
            return None
        result = f * mult
        if not math.isfinite(result) or result > _max_safe_integer:
            return None
        return int(result)
    except (ValueError, OverflowError):
        return None


def normalize_utc_offset(value: str) -> str | None:
    """Normalize a fixed UTC offset string to "UTC" or ±HH:MM.

    This helper handles fixed offsets only; IANA timezone names and DST rules
    are intentionally out of scope.
    """
    s = value.strip()
    if s.lower() in ("utc", "z"):
        return "UTC"
    if not s or s[0] not in "+-":
        return None
    parsed = _parse_utc_offset_body(s[1:])
    if parsed is None:
        return None
    hours, minutes = parsed
    if hours > 23 or minutes > 59:
        return None
    if hours == 0 and minutes == 0:
        return "UTC"
    return f"{s[0]}{hours:02d}:{minutes:02d}"


def is_valid_rfc3339_date(value: str) -> bool:
    """Return true when value is an RFC 3339 full-date (YYYY-MM-DD)."""
    if not isinstance(value, str):
        return False
    if len(value) != 10 or value[4] != "-" or value[7] != "-":
        return False
    year = _parse_ascii_int(value[0:4])
    month = _parse_ascii_int(value[5:7])
    day = _parse_ascii_int(value[8:10])
    if year is None or month is None or day is None:
        return False
    return 1 <= month <= 12 and 1 <= day <= _days_in_month(year, month)


def is_valid_rfc3339_time(value: str) -> bool:
    """Return true when value is an RFC 3339 partial-time (HH:MM:SS[.fraction])."""
    if not isinstance(value, str):
        return False
    if len(value) < 8 or value[2] != ":" or value[5] != ":":
        return False
    hour = _parse_ascii_int(value[0:2])
    minute = _parse_ascii_int(value[3:5])
    second = _parse_ascii_int(value[6:8])
    if hour is None or minute is None or second is None:
        return False
    if hour > 23 or minute > 59 or second > 59:
        return False
    if len(value) == 8:
        return True
    return value[8] == "." and len(value) > 9 and value[9:].isdigit()


def _parse_utc_offset_body(body: str) -> tuple[int, int] | None:
    if not body:
        return None
    if ":" in body:
        parts = body.split(":")
        if len(parts) != 2:
            return None
        hours, minutes = parts
        if not hours or len(hours) > 2 or len(minutes) != 2:
            return None
        if not (hours.isascii() and minutes.isascii() and hours.isdigit() and minutes.isdigit()):
            return None
        return int(hours), int(minutes)
    if not (body.isascii() and body.isdigit()):
        return None
    if len(body) in (1, 2):
        return int(body), 0
    if len(body) == 4:
        return int(body[:2]), int(body[2:])
    return None


def _parse_ascii_int(value: str) -> int | None:
    if not value or not (value.isascii() and value.isdigit()):
        return None
    return int(value)


def _days_in_month(year: int, month: int) -> int:
    if month in (1, 3, 5, 7, 8, 10, 12):
        return 31
    if month in (4, 6, 9, 11):
        return 30
    if month == 2:
        return 29 if _is_leap_year(year) else 28
    return 0


def _is_leap_year(year: int) -> bool:
    return year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)


# ═══════════════════════════════════════════
# Secret Redaction
# ═══════════════════════════════════════════


MAX_DEPTH = 256
MAX_DEPTH_MARKER = "<afdata:max-depth>"
MIN_RFC3339_MS = -62135596800000
MAX_RFC3339_MS = 253402300799999


def _sanitize_for_json(value: Any, stack: set[int] | None = None, depth: int = 0) -> Any:
    if depth >= MAX_DEPTH:
        return MAX_DEPTH_MARKER
    if stack is None:
        stack = set()

    if value is None or isinstance(value, (str, bool, int)):
        return value
    if isinstance(value, float):
        if math.isfinite(value):
            return value
        return "<unsupported:float>"
    if isinstance(value, BaseException):
        return str(value)

    if isinstance(value, dict):
        obj_id = id(value)
        if obj_id in stack:
            return "<unsupported:circular>"
        stack.add(obj_id)
        out: dict[str, Any] = {}
        for k, v in value.items():
            key = k if isinstance(k, str) else str(k)
            out[key] = _sanitize_for_json(v, stack, depth + 1)
        stack.remove(obj_id)
        return out

    if isinstance(value, (list, tuple)):
        obj_id = id(value)
        if obj_id in stack:
            return "<unsupported:circular>"
        stack.add(obj_id)
        out = [_sanitize_for_json(item, stack, depth + 1) for item in value]
        stack.remove(obj_id)
        return out

    return f"<unsupported:{type(value).__name__}>"


@dataclass(frozen=True)
class _RedactionContext:
    """Internal redaction context: the secret-name set."""

    secret_names: frozenset[str] = frozenset()

    @classmethod
    def from_names(cls, secret_names: Sequence[str]) -> _RedactionContext:
        return cls(secret_names=frozenset(secret_names))

    def is_secret_key(self, key: str) -> bool:
        return _key_has_secret_suffix(key) or key in self.secret_names


def _key_has_secret_suffix(key: str) -> bool:
    return key.endswith("_secret") or key.endswith("_SECRET")


def _key_has_url_suffix(key: str) -> bool:
    return key.endswith("_url") or key.endswith("_URL")


def _is_secret_flag_name(flag_name: str, context: _RedactionContext) -> bool:
    normalized = flag_name.replace("-", "_")
    return context.is_secret_key(normalized) or context.is_secret_key(flag_name)


def _redact_secrets(value: Any, context: _RedactionContext = _RedactionContext(), depth: int = 0) -> None:
    if depth >= MAX_DEPTH:
        return
    if isinstance(value, dict):
        for k in list(value.keys()):
            v = value[k]
            if context.is_secret_key(k):
                value[k] = "***"
            elif _key_has_url_suffix(k):
                if isinstance(v, str):
                    value[k] = _redact_url_field_value(v, context)
                elif depth + 1 >= MAX_DEPTH:
                    value[k] = MAX_DEPTH_MARKER
                else:
                    _redact_secrets(v, context, depth + 1)
            elif depth + 1 >= MAX_DEPTH:
                value[k] = MAX_DEPTH_MARKER
            else:
                _redact_secrets(v, context, depth + 1)
    elif isinstance(value, list):
        for i, item in enumerate(value):
            if depth + 1 >= MAX_DEPTH:
                value[i] = MAX_DEPTH_MARKER
            else:
                _redact_secrets(item, context, depth + 1)


# ═══════════════════════════════════════════
# URL-aware Secret Redaction
# ═══════════════════════════════════════════


def _redact_url_in_str(s: str, context: _RedactionContext) -> str | None:
    """Redact secret components of a single URL string.

    Returns the redacted string when ``s`` is a processable URL, or None when it
    is not (so callers keep the original). Only secret spans change; every other
    byte is preserved.
    """
    # Fast path + precondition: a single, whitespace-free, scheme-prefixed URL.
    if "://" not in s or not _is_single_url(s):
        return None
    scheme_sep = s.find("://")
    scheme = s[:scheme_sep]
    rest = s[scheme_sep + 3 :]

    # Authority runs from after "://" to the first '/', '?', or '#'.
    auth_end = len(rest)
    for i, c in enumerate(rest):
        if c in "/?#":
            auth_end = i
            break
    authority = rest[:auth_end]
    remainder = rest[auth_end:]

    new_authority = _redact_userinfo_password(authority)

    # Query runs from the first '?' to the first '#' (or end).
    q = remainder.find("?")
    if q == -1:
        new_remainder = remainder
    else:
        path = remainder[:q]
        query_body = remainder[q + 1 :]
        h = query_body.find("#")
        if h == -1:
            query, fragment = query_body, ""
        else:
            query, fragment = query_body[:h], query_body[h:]
        new_remainder = f"{path}?{_redact_query(query, context)}{fragment}"

    return f"{scheme}://{new_authority}{new_remainder}"


def _redact_url_field_value(s: str, context: _RedactionContext) -> str:
    redacted = _redact_url_in_str(s, context)
    if redacted is not None:
        return redacted
    trimmed = s.strip()
    if trimmed != s:
        redacted = _redact_url_in_str(trimmed, context)
        if redacted is not None:
            return redacted
    # Fail closed: a _url value we could not parse as a clean scheme-prefixed
    # URL, yet which carries a credential sigil ('@' userinfo) or internal
    # whitespace, is redacted wholesale rather than passed through. A schemeless
    # connection string like user:pass@host/db has no scheme anchor for the
    # surgical span logic above, so blanket redaction is the safe default.
    if any(c.isspace() for c in s) or "@" in s:
        return "***"
    return s


def _redact_userinfo_password(authority: str) -> str:
    """Replace the userinfo password (``user:pass@``) with ``***``.

    Preserves the username. Authority without ``@``, or userinfo without ``:``,
    is unchanged.
    """
    at = authority.rfind("@")
    if at == -1:
        return authority
    userinfo = authority[:at]
    colon = userinfo.find(":")
    if colon == -1:
        return authority
    return f"{authority[:colon]}:***{authority[at:]}"


def _redact_query(query: str, context: _RedactionContext) -> str:
    """Redact the values of secret-named query parameters.

    Preserves the raw bytes of every other segment (keys, benign values,
    encoding, ordering, separators).
    """
    segments = []
    for segment in query.split("&"):
        eq = segment.find("=")
        if eq == -1:
            segments.append(segment)
            continue
        raw_key = segment[:eq]
        # Form-decode the name ('+' -> space, percent-decode) for the check.
        name = unquote_plus(raw_key)
        if context.is_secret_key(name):
            segments.append(f"{raw_key}=***")
        else:
            segments.append(segment)
    return "&".join(segments)


def _is_single_url(s: str) -> bool:
    """True when ``s`` is a single bare URL, not a URL embedded in prose.

    It must begin with a URL scheme (ALPHA *(ALPHA / DIGIT / "+" / "-" / ".")
    "://") and contain no ASCII whitespace.
    """
    if any(c in " \t\n\r\f\v" for c in s):
        return False
    if not s or not (s[0].isascii() and s[0].isalpha()):
        return False
    i = 1
    n = len(s)
    while i < n:
        c = s[i]
        if c.isascii() and (c.isalnum() or c in "+-."):
            i += 1
        else:
            break
    return s[i:].startswith("://")


# ═══════════════════════════════════════════
# Suffix Processing
# ═══════════════════════════════════════════


def _strip_suffix_ci(key: str, suffix_lower: str) -> str | None:
    """Strip a suffix matching exact lowercase or exact uppercase only."""
    if key.endswith(suffix_lower):
        return key[: -len(suffix_lower)]
    suffix_upper = suffix_lower.upper()
    if key.endswith(suffix_upper):
        return key[: -len(suffix_upper)]
    return None


def _try_strip_generic_cents(key: str) -> tuple[str, str] | None:
    """Extract currency code from _{code}_cents / _{CODE}_CENTS."""
    code = _extract_currency_code(key)
    if code is None:
        return None
    suffix_len = len(code) + len("_cents") + 1  # _{code}_cents
    stripped = key[:-suffix_len]
    if not stripped:
        return None
    return stripped, code


def _try_strip_generic_micro(key: str) -> tuple[str, str] | None:
    """Extract currency code from _{code}_micro / _{CODE}_MICRO."""
    code = _extract_currency_code_micro(key)
    if code is None:
        return None
    suffix_len = len(code) + len("_micro") + 1  # _{code}_micro
    stripped = key[:-suffix_len]
    if not stripped:
        return None
    return stripped, code


def _is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _number_str(value: int | float) -> str:
    """Render a number canonically for YAML/plain output."""
    if isinstance(value, float) and math.isfinite(value) and value.is_integer() and abs(value) < 1e21:
        return str(int(value))
    s = repr(value) if isinstance(value, float) else str(value)
    return _normalize_exponent(s)


def _normalize_exponent(s: str) -> str:
    if "e" not in s and "E" not in s:
        return s
    mantissa, exp = re.split("[eE]", s, maxsplit=1)
    sign = ""
    if exp.startswith(("+", "-")):
        sign, exp = exp[0], exp[1:]
    exp = exp.lstrip("0") or "0"
    return f"{mantissa}e{sign}{exp}"


def _as_int(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    # Accept integral-valued floats (3.0 -> 3): JS/TS cannot distinguish 3 from
    # 3.0 after JSON parsing, so integrality (not lexical form) gates integer
    # suffixes, keeping the four implementations consistent.
    if isinstance(value, float) and math.isfinite(value) and value.is_integer():
        return int(value)
    return None


def _as_non_neg_int(value: Any) -> int | None:
    n = _as_int(value)
    if n is not None and n >= 0:
        return n
    return None


def _as_decimal_int(value: Any) -> int | None:
    if isinstance(value, str) and re.fullmatch(r"-?\d+", value):
        return int(value)
    return _as_int(value)


def _decimal_int_text(value: Any) -> str | None:
    if isinstance(value, str) and re.fullmatch(r"-?\d+", value):
        return value
    n = _as_int(value)
    if n is None:
        return None
    return str(n)


def _try_process_field(key: str, value: Any) -> tuple[str, str] | None:
    """Try suffix-driven processing. Returns (stripped_key, formatted_value) or None."""
    # Group 1: compound timestamp suffixes
    stripped = _strip_suffix_ci(key, "_epoch_ms")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            formatted = _format_rfc3339_ms(n)
            if formatted is not None:
                return stripped, formatted
        return None
    stripped = _strip_suffix_ci(key, "_epoch_s")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            formatted = _format_rfc3339_ms(n * 1000)
            if formatted is not None:
                return stripped, formatted
        return None
    stripped = _strip_suffix_ci(key, "_epoch_ns")
    if stripped is not None:
        n = _as_decimal_int(value)
        if n is not None:
            formatted = _format_rfc3339_ms(n // 1_000_000)
            if formatted is not None:
                return stripped, formatted
        return None

    # Group 2: compound currency suffixes
    stripped = _strip_suffix_ci(key, "_usd_cents")
    if stripped is not None:
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, f"${n // 100}.{n % 100:02d}"
        return None
    stripped = _strip_suffix_ci(key, "_eur_cents")
    if stripped is not None:
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, f"\u20ac{n // 100}.{n % 100:02d}"
        return None
    gc = _try_strip_generic_cents(key)
    if gc is not None:
        stripped, code = gc
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, f"{n // 100}.{n % 100:02d} {code.upper()}"
        return None
    gm = _try_strip_generic_micro(key)
    if gm is not None:
        stripped, code = gm
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, f"{n // 1_000_000}.{n % 1_000_000:06d} {code.upper()}"
        return None

    # Group 3: multi-char suffixes
    stripped = _strip_suffix_ci(key, "_rfc3339")
    if stripped is not None:
        if isinstance(value, str):
            return stripped, value
        return None
    stripped = _strip_suffix_ci(key, "_minutes")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)} minutes"
        return None
    stripped = _strip_suffix_ci(key, "_hours")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)} hours"
        return None
    stripped = _strip_suffix_ci(key, "_days")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)} days"
        return None

    # Group 4: single-unit suffixes
    stripped = _strip_suffix_ci(key, "_msats")
    if stripped is not None:
        text = _decimal_int_text(value)
        if text is not None:
            return stripped, f"{text}msats"
        return None
    stripped = _strip_suffix_ci(key, "_sats")
    if stripped is not None:
        text = _decimal_int_text(value)
        if text is not None:
            return stripped, f"{text}sats"
        return None
    stripped = _strip_suffix_ci(key, "_bytes")
    if stripped is not None:
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, _format_bytes_human(n)
        return None
    stripped = _strip_suffix_ci(key, "_percent")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}%"
        return None
    # Group 5: short suffixes (last to avoid false positives)
    stripped = _strip_suffix_ci(key, "_jpy")
    if stripped is not None:
        n = _as_non_neg_int(value)
        if n is not None:
            return stripped, f"\u00a5{_format_with_commas(n)}"
        return None
    stripped = _strip_suffix_ci(key, "_ns")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}ns"
        return None
    stripped = _strip_suffix_ci(key, "_us")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}\u03bcs"
        return None
    stripped = _strip_suffix_ci(key, "_ms")
    if stripped is not None:
        fv = _format_ms_value(value)
        if fv is not None:
            return stripped, fv
        return None
    stripped = _strip_suffix_ci(key, "_s")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}s"
        return None

    return None


def _process_object_fields(d: dict) -> list[tuple[str, Any, str | None]]:
    """Process fields: strip keys, format values, detect collisions.

    Returns list of (display_key, value, formatted_value_or_None).
    """
    entries: list[tuple[str, str, Any, str | None]] = []
    for k, v in d.items():
        stripped_secret = _strip_suffix_ci(k, "_secret")
        if stripped_secret is not None:
            entries.append((stripped_secret, k, v, None))
            continue
        result = _try_process_field(k, v)
        if result is not None:
            stripped, formatted = result
            entries.append((stripped, k, v, formatted))
        else:
            entries.append((k, k, v, None))

    # Detect collisions
    counts: dict[str, int] = {}
    for stripped, _, _, _ in entries:
        counts[stripped] = counts.get(stripped, 0) + 1

    # Resolve collisions: revert both key and formatted value
    result_list: list[tuple[str, Any, str | None]] = []
    for stripped, original, value, formatted in entries:
        display_key = stripped
        if counts.get(stripped, 0) > 1 and original != stripped:
            display_key = original
            formatted = None
        result_list.append((display_key, value, formatted))

    # Sort by display key (JCS order = UTF-16 code unit order)
    result_list.sort(key=lambda x: _utf16_sort_key(x[0]))
    return result_list


# ═══════════════════════════════════════════
# Formatting Helpers
# ═══════════════════════════════════════════


def _format_ms_as_seconds(ms: float) -> str:
    """Format ms as seconds: 3 decimal places, trim trailing zeros, min 1 decimal."""
    formatted = f"{ms / 1000:.3f}"
    trimmed = formatted.rstrip("0")
    if trimmed.endswith("."):
        return trimmed + "0s"
    return trimmed + "s"


def _format_ms_value(value: Any) -> str | None:
    """Format _ms value: < 1000 -> {n}ms, >= 1000 -> seconds."""
    if not _is_number(value):
        return None
    n = float(value)
    if abs(n) >= 1000:
        return _format_ms_as_seconds(n)
    return f"{_plain_scalar(value)}ms"


def _format_rfc3339_ms(ms: int) -> str | None:
    if ms < MIN_RFC3339_MS or ms > MAX_RFC3339_MS:
        return None
    try:
        dt = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=ms)
        return dt.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"
    except (OverflowError, ValueError):
        return None


def _format_bytes_human(n: int) -> str:
    KiB = 1024.0
    MiB = KiB * 1024
    GiB = MiB * 1024
    TiB = GiB * 1024
    b = float(n)
    if b >= TiB:
        return f"{b / TiB:.1f}TiB"
    if b >= GiB:
        return f"{b / GiB:.1f}GiB"
    if b >= MiB:
        return f"{b / MiB:.1f}MiB"
    if b >= KiB:
        return f"{b / KiB:.1f}KiB"
    return f"{n}B"


def _format_with_commas(n: int) -> str:
    return f"{n:,}"


def _extract_currency_code(key: str) -> str | None:
    """Extract currency code from _{code}_cents / _{CODE}_CENTS suffix."""
    if key.endswith("_cents"):
        without_cents = key[:-6]
    elif key.endswith("_CENTS"):
        without_cents = key[:-6]
    else:
        return None
    return _extract_currency_code_from_stem(without_cents)


def _extract_currency_code_micro(key: str) -> str | None:
    """Extract currency code from _{code}_micro / _{CODE}_MICRO suffix."""
    if key.endswith("_micro"):
        without_micro = key[:-6]
    elif key.endswith("_MICRO"):
        without_micro = key[:-6]
    else:
        return None
    return _extract_currency_code_from_stem(without_micro)


def _extract_currency_code_from_stem(stem: str) -> str | None:
    idx = stem.rfind("_")
    if idx < 0:
        return None
    code = stem[idx + 1 :]
    if not code:
        return None
    if len(code) not in (3, 4) or not code.isascii() or not code.isalpha():
        return None
    return code


# ═══════════════════════════════════════════
# YAML Rendering
# ═══════════════════════════════════════════


def _render_yaml_processed(value: Any, indent: int, lines: list[str]) -> None:
    prefix = "  " * indent
    if not isinstance(value, dict):
        lines.append(f"{prefix}{_yaml_scalar(value)}")
        return

    for display_key, v, formatted in _process_object_fields(value):
        if formatted is not None:
            lines.append(f'{prefix}{_yaml_key(display_key)}: "{_escape_yaml_str(formatted)}"')
        elif isinstance(v, dict):
            if v:
                lines.append(f"{prefix}{_yaml_key(display_key)}:")
                _render_yaml_processed(v, indent + 1, lines)
            else:
                lines.append(f"{prefix}{_yaml_key(display_key)}: {{}}")
        elif isinstance(v, list):
            if not v:
                lines.append(f"{prefix}{_yaml_key(display_key)}: []")
            else:
                lines.append(f"{prefix}{_yaml_key(display_key)}:")
                for item in v:
                    if isinstance(item, dict):
                        lines.append(f"{prefix}  -")
                        _render_yaml_processed(item, indent + 2, lines)
                    else:
                        lines.append(f"{prefix}  - {_yaml_scalar(item)}")
        else:
            lines.append(f"{prefix}{_yaml_key(display_key)}: {_yaml_scalar(v)}")


def _render_yaml_raw(value: Any, indent: int, lines: list[str]) -> None:
    prefix = "  " * indent
    if isinstance(value, dict):
        for key in _sorted_object_keys(value):
            _render_yaml_field_raw(prefix, key, value[key], indent, lines)
    elif isinstance(value, list):
        _render_yaml_array_raw(value, indent, lines)
    else:
        lines.append(f"{prefix}{_yaml_scalar(value)}")


def _render_yaml_field_raw(prefix: str, key: str, value: Any, indent: int, lines: list[str]) -> None:
    if isinstance(value, dict):
        if value:
            lines.append(f"{prefix}{_yaml_key(key)}:")
            _render_yaml_raw(value, indent + 1, lines)
        else:
            lines.append(f"{prefix}{_yaml_key(key)}: {{}}")
    elif isinstance(value, list):
        if value:
            lines.append(f"{prefix}{_yaml_key(key)}:")
            _render_yaml_array_raw(value, indent + 1, lines)
        else:
            lines.append(f"{prefix}{_yaml_key(key)}: []")
    else:
        lines.append(f"{prefix}{_yaml_key(key)}: {_yaml_scalar(value)}")


def _render_yaml_array_raw(arr: list[Any], indent: int, lines: list[str]) -> None:
    prefix = "  " * indent
    for item in arr:
        if isinstance(item, dict):
            if item:
                lines.append(f"{prefix}-")
                _render_yaml_raw(item, indent + 1, lines)
            else:
                lines.append(f"{prefix}- {{}}")
        elif isinstance(item, list):
            if item:
                lines.append(f"{prefix}-")
                _render_yaml_array_raw(item, indent + 1, lines)
            else:
                lines.append(f"{prefix}- []")
        else:
            lines.append(f"{prefix}- {_yaml_scalar(item)}")


def _escape_yaml_str(s: str) -> str:
    return (
        s.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
        .replace("\f", "\\f")
        .replace("\v", "\\v")
    )


def _yaml_key(key: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_.-]+", key):
        return key
    return f'"{_escape_yaml_str(key)}"'


def _yaml_scalar(value: Any) -> str:
    if isinstance(value, str):
        return f'"{_escape_yaml_str(value)}"'
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return _number_str(value)
    if isinstance(value, (dict, list)):
        return f'"{_escape_yaml_str(_canonical_json(value))}"'
    return f'"{_escape_yaml_str(str(value))}"'


# ═══════════════════════════════════════════
# Plain Rendering (logfmt)
# ═══════════════════════════════════════════


def _collect_plain_pairs(value: Any, prefix: str, pairs: list[tuple[str, str]]) -> None:
    if not isinstance(value, dict):
        return
    for display_key, v, formatted in _process_object_fields(value):
        full_key = f"{prefix}.{display_key}" if prefix else display_key
        if formatted is not None:
            pairs.append((full_key, formatted))
        elif isinstance(v, dict):
            _collect_plain_pairs(v, full_key, pairs)
        elif isinstance(v, list):
            joined = ",".join(_plain_scalar(item) for item in v)
            pairs.append((full_key, joined))
        elif v is None:
            pairs.append((full_key, ""))
        else:
            pairs.append((full_key, _plain_scalar(v)))


def _collect_plain_pairs_raw(value: Any, prefix: str, pairs: list[tuple[str, str]]) -> None:
    if not isinstance(value, dict):
        return
    for key in _sorted_object_keys(value):
        v = value[key]
        full_key = f"{prefix}.{key}" if prefix else key
        if isinstance(v, dict):
            _collect_plain_pairs_raw(v, full_key, pairs)
        elif isinstance(v, list):
            joined = ",".join(_plain_scalar_raw(item) for item in v)
            pairs.append((full_key, joined))
        elif v is None:
            pairs.append((full_key, ""))
        else:
            pairs.append((full_key, _plain_scalar(v)))


def _plain_scalar(value: Any) -> str:
    if isinstance(value, str):
        return value
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return _number_str(value)
    if isinstance(value, (dict, list)):
        return _canonical_json(value)
    return str(value)


def _plain_scalar_raw(value: Any) -> str:
    if isinstance(value, (dict, list)):
        return _canonical_json(value)
    return _plain_scalar(value)


def _quote_logfmt_value(value: str) -> str:
    if value == "":
        return ""
    needs_quote = any(c.isspace() or c in '="\\' for c in value)
    if not needs_quote:
        return value
    escaped = (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
        .replace("\f", "\\f")
        .replace("\v", "\\v")
    )
    return f'"{escaped}"'


def _quote_logfmt_key(key: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_.-]+", key):
        return key
    return _quote_logfmt_value(key)


def _sorted_object_keys(d: dict) -> list[str]:
    return sorted(d.keys(), key=_utf16_sort_key)


def _utf16_sort_key(s: str) -> bytes:
    return s.encode("utf-16-be", "surrogatepass")


def _canonical_json(value: Any) -> str:
    return json.dumps(_sort_json_value(value), ensure_ascii=False, separators=(",", ":"))


def _sort_json_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: _sort_json_value(value[k]) for k in _sorted_object_keys(value)}
    if isinstance(value, list):
        return [_sort_json_value(item) for item in value]
    return value
