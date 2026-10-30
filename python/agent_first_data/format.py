"""AFDATA output formatting and protocol templates.

Protocol builders, value redactors (copy and in-place; cover _secret and
_url fields), URL-string redactors (redact_url_secrets),
normalize_utc_offset, is_valid_rfc3339_date,
is_valid_rfc3339_time, RedactionPolicy, PlainStyle, and
OutputOptions. Each redactor concept is a single function taking a
keyword-only ``options`` parameter. The single public render entry point
(``render``) lives in :mod:`agent_first_data.cli`; this module holds the
private per-format renderers it dispatches to.
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from enum import Enum
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import unquote_plus


# ═══════════════════════════════════════════
# Public API: Protocol v1 Event Type
# ═══════════════════════════════════════════


class LogLevel(str, Enum):
    """Log level enum for structured logging.

    A ``str`` mix-in rather than :class:`enum.StrEnum`, which only exists from
    Python 3.11 and would silently break the floor this package declares in
    ``pyproject.toml``. The two differ in ``str()``/``format()`` shape (a
    ``str``-mix-in member renders as ``LogLevel.INFO``, a ``StrEnum`` member as
    ``info``), and that shape has itself moved between releases — so the wire
    form is never taken from ``str()``: it is ``.value``, which every renderer
    reaches through the exact-``str`` normalization in
    :func:`_sanitize_for_json`.
    """
    DEBUG = "debug"
    INFO = "info"
    WARN = "warn"
    ERROR = "error"


class EventBuildError(Exception):
    """Exception raised when building an Event fails."""
    pass


class Event:
    """SDK event envelope; validate externally supplied trace values explicitly."""

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

    def trace(self, obj: Any) -> ResultBuilder:
        """Set trace context (merged at build time)."""
        self._trace = dict(obj) if isinstance(obj, dict) else obj
        return self

    def build(self) -> Event:
        """Build and return the Event. This builder cannot fail."""
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
        if not code:
            self._errors.append("error code must not be empty")
        if not message:
            self._errors.append("error message must not be empty")

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
        try:
            json.dumps(envelope, ensure_ascii=False, allow_nan=False)
        except (TypeError, ValueError, RecursionError) as exc:
            raise EventBuildError(f"event is not JSON serializable: {exc}") from exc
        return Event(envelope)


class ProgressBuilder:
    """Fluent builder for progress events."""

    def __init__(self, payload: Any) -> None:
        self._payload = payload
        self._trace: dict = {}

    def trace(self, obj: Any) -> ProgressBuilder:
        """Set trace context."""
        self._trace = dict(obj) if isinstance(obj, dict) else obj
        return self

    def build(self) -> Event:
        """Build and return the Event. This builder cannot fail."""
        envelope = {
            "kind": "progress",
            "progress": self._payload,
            "trace": self._trace,
        }
        return Event(envelope)


class LogBuilder:
    """Fluent builder for log events."""

    def __init__(self, payload: Any) -> None:
        self._payload = payload
        self._trace: dict = {}

    def trace(self, obj: Any) -> LogBuilder:
        """Set trace context."""
        self._trace = dict(obj) if isinstance(obj, dict) else obj
        return self

    def build(self) -> Event:
        """Build and return the Event. This builder cannot fail."""
        envelope = {
            "kind": "log",
            "log": self._payload,
            "trace": self._trace,
        }
        return Event(envelope)


def json_result(result: Any) -> ResultBuilder:
    """Create a fluent result builder."""
    return ResultBuilder(result)


def json_error(code: str, message: str) -> ErrorBuilder:
    """Create a fluent error builder.

    An empty ``code`` or ``message`` does not raise here; it seeds a deferred
    error that is surfaced when :meth:`ErrorBuilder.build` is called.
    """
    return ErrorBuilder(code, message)


def json_progress(payload: Any) -> ProgressBuilder:
    """Create a fluent progress builder."""
    return ProgressBuilder(payload)


def json_log(payload: Any) -> LogBuilder:
    """Create a fluent log builder."""
    return LogBuilder(payload)


def validate_protocol_event(event: Any, *, strict: bool = True) -> None:
    """Validate one protocol v1 event envelope.

    With ``strict=True`` (the default), also enforces the recommended strict
    protocol profile (required trace and required error.retryable). Pass
    ``strict=False`` for the plain, lenient envelope-shape check only.
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
        raise ValueError("event.error.retryable must be a boolean when present")
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


class _RawNumber:
    """Internal: a decoded JSON number whose exact source literal must
    survive re-emission (see decode_protocol_event's number-fidelity note).

    Deliberately not an int/float subclass. Serializers treat it as an opaque
    scalar; integer-required suffixes inspect its exact decimal digits, while
    unit scaling uses a bounded floating conversion only where needed.
    """

    __slots__ = ("literal",)

    def __init__(self, literal: str) -> None:
        self.literal = literal

    def __repr__(self) -> str:  # pragma: no cover - debugging aid only
        return f"_RawNumber({self.literal!r})"


def _parse_int_lossless(text: str) -> Any:
    """json.loads parse_int hook: int() is exact for every JSON integer
    literal except "-0" (int("-0") == 0, dropping the sign), so only that
    case and literals beyond Python's conversion limit need wrapping. Other
    integers stay plain arbitrary-precision Python ints."""
    if text == "-0":
        return _RawNumber(text)
    try:
        return int(text)
    except ValueError:
        # Python's configurable conversion limit is not a JSON grammar limit.
        return _RawNumber(text)


def _parse_float_lossless(text: str) -> Any:
    """json.loads parse_float hook: always wraps, since float64 can silently
    drop significant digits for any literal (there is no simple "is this one
    safe" test analogous to the integer case above)."""
    return _RawNumber(text)


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
    retryable: bool | None
    hint: str | None = None
    fields: dict[str, Any] = field(default_factory=dict)
    trace: dict | None = None


@dataclass(frozen=True)
class DecodedProgress:
    """Decoded protocol v1 progress event."""

    progress: Any
    trace: dict | None = None


@dataclass(frozen=True)
class DecodedLog:
    """Decoded protocol v1 log event."""

    log: Any
    trace: dict | None = None


def _reject_non_json_constant(name: str) -> Any:
    # ``json.loads`` accepts ``NaN`` and ``Infinity`` by default; they are not
    # JSON, and every other SDK's decoder refuses them.
    raise ValueError(f"{name} is not a JSON number")


# How many arrays and objects a protocol line may nest, counting the envelope
# object. Every SDK enforces the same bound.
_MAX_PROTOCOL_NESTING = 127
_JSON_STRING_OR_BRACKET = re.compile(r'"(?:[^"\\]+|\\.)*"|[\[\]{}]')
_SURROGATE = re.compile("[\ud800-\udfff]")


def _nesting_exceeds_limit(text: str) -> bool:
    """Whether brackets outside strings nest past the bound. Checked before
    ``json.loads``, which would otherwise recurse until RecursionError."""
    if text.count("[") + text.count("{") <= _MAX_PROTOCOL_NESTING:
        return False
    depth = 0
    for match in _JSON_STRING_OR_BRACKET.finditer(text):
        token = match.group()
        if token in "[{":
            depth += 1
            if depth > _MAX_PROTOCOL_NESTING:
                return True
        elif token in "]}":
            depth -= 1
    return False


def _has_lone_surrogate(value: Any) -> bool:
    """``json.loads`` joins an escaped surrogate pair into one character and
    keeps a lone surrogate (escaped or raw) as-is, so any surrogate code point
    left in a decoded key or string is a lone one."""
    if isinstance(value, str):
        return _SURROGATE.search(value) is not None
    if isinstance(value, dict):
        return any(_has_lone_surrogate(k) or _has_lone_surrogate(v) for k, v in value.items())
    if isinstance(value, list):
        return any(_has_lone_surrogate(item) for item in value)
    return False


def decode_protocol_event(
    text: str,
) -> DecodedResult | DecodedError | DecodedProgress | DecodedLog:
    """Parse and base-validate a single protocol v1 JSON line into a typed event.

    Raises EventDecodeError if ``text`` is not valid JSON or fails base
    validation.

    Number literal fidelity: ``json.loads``'s default integer parsing is
    already arbitrary-precision (exact for every integer except the single
    "-0" edge case, where ``int("-0") == 0`` silently drops the sign), but its
    default float parsing collapses any literal through float64 (30 significant
    digits become 15-17, "0.1000...055511..." becomes "0.1"). ``parse_int``/
    ``parse_float`` hooks below route both exceptions through ``_RawNumber``,
    which stores the exact source literal; JSON re-emits that literal
    verbatim, and YAML/plain render it in the spec's canonical number form
    without losing a digit. This is internal — ``_RawNumber`` is never part of the public API.
    """
    if _nesting_exceeds_limit(text):
        raise EventDecodeError(f"invalid JSON: nesting deeper than {_MAX_PROTOCOL_NESTING}")
    try:
        event = json.loads(
            text,
            parse_int=_parse_int_lossless,
            parse_float=_parse_float_lossless,
            parse_constant=_reject_non_json_constant,
        )
    except (ValueError, TypeError) as exc:
        raise EventDecodeError(f"invalid JSON: {exc}") from exc
    if _has_lone_surrogate(event):
        raise EventDecodeError("invalid JSON: lone surrogate in a string")

    try:
        validate_protocol_event(event, strict=False)
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
            retryable=error.get("retryable"),
            hint=error.get("hint"),
            fields=fields,
            trace=trace,
        )

    if kind == "progress":
        return DecodedProgress(progress=event["progress"], trace=trace)

    # kind == "log"
    return DecodedLog(log=event["log"], trace=trace)


# ═══════════════════════════════════════════
# Public API: Output Option Types
# ═══════════════════════════════════════════

class RedactionPolicy(str, Enum):
    """Which fields a redaction pass scrubs. Default (absent policy) is All.

    The policy selects a *scope* inside a structured value. A command line
    (:func:`redact_argv`) and a bare URL string (:func:`redact_url_secrets`)
    have no ``result``/``trace`` split to scope to, so on those two paths
    ``TraceOnly`` redacts in full like ``All``, and only ``Off`` turns
    redaction off.
    """
    All = "All"
    TraceOnly = "TraceOnly"
    Off = "Off"


def _redacts_unscoped_input(policy: RedactionPolicy | None) -> bool:
    """Whether ``policy`` redacts an input that carries no scope of its own — a
    command line or a single URL string, as opposed to a JSON value with a
    ``trace`` object to narrow to.

    ``TraceOnly`` narrows *where* redaction applies within a value; it does not
    weaken redaction. A standalone argv or URL has no non-``trace`` half to
    leave alone — and both are diagnostic material by construction, the very
    thing ``TraceOnly`` scrubs — so ``TraceOnly`` redacts them in full, exactly
    like ``All``. Only ``Off``, the caller explicitly asking for raw output,
    disables redaction, and it does so on all three paths alike.
    """
    return policy != RedactionPolicy.Off


class PlainStyle(str, Enum):
    """Rendering style for plain (logfmt) output only.

    JSON and YAML are structure-preserving and ignore this; only the plain
    renderer varies by style.
    """

    Readable = "Readable"
    Raw = "Raw"


@dataclass(frozen=True)
class OutputOptions:
    """Output options combining redaction and rendering style."""

    # Exact field-name matches at any nesting level. The same list also matches
    # URL query-parameter names inside _url fields (see redact_url_secrets).
    secret_names: Sequence[str] = ()
    # Exact legacy field names whose string leaves receive URL-aware redaction.
    url_names: Sequence[str] = ()
    # None or RedactionPolicy.All = full redaction (default).
    policy: RedactionPolicy | None = None
    style: PlainStyle = PlainStyle.Readable
    # Exact structured field names to reveal; URL parameters and argv stay redacted.
    reveal_names: Sequence[str] = ()

    @classmethod
    def for_policy(cls, policy: RedactionPolicy) -> OutputOptions:
        """Convenience constructor: output options with only a redaction policy set."""
        return cls(policy=policy)


def _format_json(value: Any, *, options: OutputOptions | None = None) -> str:
    """Internal: JSON rendering used by agent_first_data.cli.render.

    Single-line JSON. Secrets redacted, original keys, raw values.
    """
    output_options = options or OutputOptions()
    redacted = redacted_value(
        value,
        secret_names=output_options.secret_names,
        url_names=output_options.url_names,
        reveal_names=output_options.reveal_names,
        policy=output_options.policy,
    )
    return _encode_json_lossless(redacted)


def _encode_json_lossless(value: Any) -> str:
    """Compact JSON encoder, byte-identical to
    ``json.dumps(value, ensure_ascii=False, separators=(",", ":"))`` for every
    value that contains no ``_RawNumber``, and additionally correct for values
    that do: ``json.dumps`` has no hook to emit a raw, unquoted number literal
    (a ``_RawNumber`` is not int/float, so stdlib ``json`` would reject it, and
    subclassing float doesn't work either — ``json``'s float encoder calls the
    unbound ``float.__repr__`` directly, bypassing any subclass override).
    Recurses only through dict/list to find nested ``_RawNumber`` leaves;
    every other value (including plain int/float/str/bool/None) delegates to
    ``json.dumps`` so string escaping and number formatting stay exactly as
    they were before this function existed.
    """
    if isinstance(value, _RawNumber):
        return value.literal
    if isinstance(value, dict):
        return "{" + ",".join(
            f"{json.dumps(k, ensure_ascii=False)}:{_encode_json_lossless(v)}" for k, v in value.items()
        ) + "}"
    if isinstance(value, list):
        return "[" + ",".join(_encode_json_lossless(v) for v in value) + "]"
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def _format_yaml(value: Any, *, options: OutputOptions | None = None) -> str:
    """Internal: YAML rendering used by agent_first_data.cli.render.

    Multi-line YAML. Structure-preserving like JSON: original keys, raw scalar
    types, secrets redacted. `PlainStyle` has no effect on YAML.
    """
    output_options = options or OutputOptions()
    value = redacted_value(
        value,
        secret_names=output_options.secret_names,
        url_names=output_options.url_names,
        reveal_names=output_options.reveal_names,
        policy=output_options.policy,
    )
    lines = ["---"]
    _render_yaml_raw(value, 0, lines)
    return "\n".join(lines)


def _format_plain(value: Any, *, options: OutputOptions | None = None) -> str:
    """Internal: plain (logfmt) rendering used by agent_first_data.cli.render.

    Single-line logfmt. Keys stripped, values formatted, secrets redacted.
    """
    output_options = options or OutputOptions()
    value = redacted_value(
        value,
        secret_names=output_options.secret_names,
        url_names=output_options.url_names,
        reveal_names=output_options.reveal_names,
        policy=output_options.policy,
    )
    if not isinstance(value, dict):
        return _plain_scalar(value)
    pairs: list[tuple[str, str]] = []
    if output_options.style is PlainStyle.Raw:
        _collect_plain_pairs_raw(value, "", pairs)
    else:
        _collect_plain_pairs(value, "", pairs)
    if not pairs:
        return "{}"
    pairs.sort(key=lambda p: _utf16_sort_key(p[0]))
    parts = []
    for k, v in pairs:
        parts.append(f"{_quote_logfmt_key(k)}={_quote_logfmt_value(v)}")
    return " ".join(parts)


# ═══════════════════════════════════════════
# Public API: Redaction & Utility
# ═══════════════════════════════════════════


def redacted_value(
    value: Any,
    *,
    secret_names: Sequence[str] = (),
    url_names: Sequence[str] = (),
    reveal_names: Sequence[str] = (),
    policy: RedactionPolicy | None = None,
) -> Any:
    """Return a JSON-safe copy with redaction options applied.

    Redacts fields ending in _secret/_SECRET and those listed in secret_names.
    Redacts _url fields and exact ``url_names`` entries by the same rules.
    """
    v = _sanitize_for_json(value)
    _apply_redaction(v, secret_names, url_names, policy, reveal_names)
    return v


def redact_url_secrets(
    url: str,
    *,
    secret_names: Sequence[str] = (),
    policy: RedactionPolicy | None = None,
) -> str:
    """Redact secret components of a single URL string.

    Returns ``url`` with its userinfo password and any ``_secret``-suffixed
    query-parameter values replaced by ``***``. A query parameter is redacted
    iff its (form-decoded) name ends in ``_secret``/``_SECRET`` or matches an
    exact entry in ``secret_names``. A fragment written in the same ``k=v&k=v``
    shape — how an OAuth implicit-flow response carries its token — is redacted
    by that same rule; any other fragment passes through byte-for-byte. The
    userinfo password (``scheme://user:pass@host``) is always redacted as a
    structural rule. Only the secret spans are replaced with ``***``; every
    other byte is preserved. A string that is not a single, whitespace-free,
    scheme-prefixed URL (including a URL embedded in surrounding prose) is
    returned unchanged.

    A URL is unscoped input: ``RedactionPolicy.Off`` returns it unchanged,
    every other policy redacts it in full.
    """
    if not _redacts_unscoped_input(policy):
        return url
    context = _RedactionContext.from_names(secret_names)
    redacted = _redact_url_in_str(url, context)
    return redacted if redacted is not None else url


def redact_urls_in_text(
    text: str,
    *,
    secret_names: Sequence[str] = (),
    policy: RedactionPolicy | None = None,
) -> str:
    """Redact complete scheme-prefixed URL spans embedded in prose.

    This helper is explicit and URL-only: surrounding text is never scanned
    for secret-looking names or values.
    """
    if not _redacts_unscoped_input(policy):
        return text
    context = _RedactionContext.from_names(secret_names)
    return _redact_urls_in_text_with_context(text, context)


def redact_argv(
    args: Sequence[str],
    *,
    secret_names: Sequence[str] = (),
    policy: RedactionPolicy | None = None,
) -> list[str]:
    """Redact secret values out of a command line.

    A long flag whose name is secret by AFDATA naming (``--api-key-secret``, or
    an exact ``secret_names`` entry) has its value replaced by ``***``, in both
    ``--flag=value`` and ``--flag value`` spellings. Everything else is
    preserved byte-for-byte.

    Free text is deliberately never scanned: a bare ``api_key_secret=sk-live``
    positional, or a secret-looking token after a non-secret flag, is left
    alone. AFDATA decides sensitivity from the *field name*, and argv is no
    exception — rename the flag rather than pattern-matching values. A flag with
    no value (end of argv, or followed by another flag) is likewise left
    inspectable.

    Only long (``--``) flags are recognized, matching the convention's
    long-flags-only rule.

    Intended for CLIs that record their own invocation — startup diagnostics,
    audit trails, crash reports — where writing argv verbatim would put a
    credential in the log.

    A command line is unscoped input: ``RedactionPolicy.Off`` returns ``args``
    unchanged, every other policy redacts in full.
    """
    if not _redacts_unscoped_input(policy):
        return list(args)
    context = _RedactionContext.from_names(secret_names)
    out: list[str] = []
    redact_next = False
    for arg in args:
        if redact_next:
            redact_next = False
            if not arg.startswith("-"):
                out.append(REDACTED_MARKER)
                continue
        if arg.startswith("--"):
            rest = arg[2:]
            name, sep, _value = rest.partition("=")
            if sep:
                if _is_secret_flag_name(name, context):
                    out.append(f"--{name}={REDACTED_MARKER}")
                    continue
            elif _is_secret_flag_name(rest, context):
                redact_next = True
        out.append(arg)
    return out


def _is_secret_flag_name(flag_name: str, context: _RedactionContext) -> bool:
    """Whether a long flag's name is secret, normalizing kebab-case to snake_case."""
    normalized = flag_name.replace("-", "_")
    return context.is_secret_key(normalized) or context.is_secret_key(flag_name)


def _apply_redaction(
    value: Any,
    secret_names: Sequence[str],
    url_names: Sequence[str],
    policy: RedactionPolicy | None,
    reveal_names: Sequence[str] = (),
) -> None:
    context = _RedactionContext.from_names(secret_names, url_names, reveal_names)
    _apply_redaction_policy_with_context(value, policy, context)


def _apply_redaction_policy_with_context(
    value: Any,
    redaction_policy: RedactionPolicy | None,
    context: _RedactionContext,
) -> None:
    if redaction_policy == RedactionPolicy.TraceOnly:
        if isinstance(value, dict) and "trace" in value:
            _redact_secrets(value["trace"], context)
        return
    if redaction_policy == RedactionPolicy.Off:
        return
    # None (absent) or RedactionPolicy.All -> full redaction.
    _redact_secrets(value, context)


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
    fraction = value[9:]
    return (
        value[8] == "."
        and bool(fraction)
        and fraction.isascii()
        and fraction.isdigit()
    )


def is_valid_rfc3339(value: str) -> bool:
    """Return true when value is a complete RFC 3339 date-time.

    Composed from is_valid_rfc3339_date and is_valid_rfc3339_time: a full-date, a
    T/t separator, a partial-time (with optional fractional seconds), and a
    mandatory time-offset (Z/z or ±HH:MM with HH in 00..23 and MM in 00..59).
    The offset is required, so a bare 2026-02-14T10:30:00 is rejected; a space
    separator is rejected; and a leap second (:60) is rejected, matching
    is_valid_rfc3339_time. Non-ASCII input is rejected.
    """
    if not isinstance(value, str) or len(value) < 20 or not value.isascii():
        return False
    if not is_valid_rfc3339_date(value[0:10]):
        return False
    if value[10] not in ("T", "t"):
        return False
    rest = value[11:]
    if rest[-1] in ("Z", "z"):
        partial = rest[:-1]
    else:
        if len(rest) < 6 or not _is_rfc3339_numoffset(rest[-6:]):
            return False
        partial = rest[:-6]
    return is_valid_rfc3339_time(partial)


def _is_rfc3339_numoffset(offset: str) -> bool:
    if len(offset) != 6 or offset[0] not in ("+", "-") or offset[3] != ":":
        return False
    hours = _parse_ascii_int(offset[1:3])
    minutes = _parse_ascii_int(offset[4:6])
    if hours is None or minutes is None:
        return False
    return hours <= 23 and minutes <= 59


def is_valid_bcp47(value: str) -> bool:
    """Return true when value is a structurally well-formed BCP 47 language tag.

    A grammar-level check, not a registry lookup: hyphen-separated ASCII-alphanumeric
    subtags (each 1-8 chars) whose primary subtag is a 2-3 letter language code or the
    x/i privateuse/grandfathered lead. Rejects the POSIX underscore form (zh_CN), empty
    or misplaced hyphens, non-ASCII, and out-of-range primaries such as chinese. Does
    not verify that subtags are registered with IANA.
    """
    if not isinstance(value, str) or not value:
        return False
    for index, subtag in enumerate(value.split("-")):
        if not (1 <= len(subtag) <= 8) or not subtag.isascii() or not subtag.isalnum():
            return False
        if index == 0:
            is_language = 2 <= len(subtag) <= 3 and subtag.isalpha()
            is_special = subtag in ("x", "i")
            if not is_language and not is_special:
                return False
    return True


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
# The scalar every redacted span, value, and subtree is replaced with. Also the
# signal plain rendering reads to tell a hidden field from a live one.
REDACTED_MARKER = "***"
MIN_RFC3339_MS = -62135596800000
MAX_RFC3339_MS = 253402300799999


def _exact_scalar(value: Any) -> Any:
    """Normalize a str/int instance to the exact builtin type.

    An enum with a ``str``/``int`` mix-in (``class LogLevel(str, Enum)``) *is* a
    ``str``/``int``, so it flows through this module untouched — but its
    ``__str__``/``__format__`` render the member (``LogLevel.INFO``), not the
    value (``info``). JSON serialization reads the underlying value, so the same
    member would come out as ``"info"`` in JSON and ``LogLevel.INFO`` in plain
    logfmt, which f-string-interpolates it. Reading the raw payload through the
    *unbound* ``str.__str__``/``int`` (never the member's own override) settles
    that here, once, for every renderer downstream — and keeps the answer stable
    across Python versions, which have moved mix-in enum ``str()`` more than
    once.
    """
    if type(value) is str or type(value) is int:
        return value
    if isinstance(value, str):
        return str.__str__(value)
    return int(value)


def _sanitize_for_json(value: Any, stack: set[int] | None = None, depth: int = 0) -> Any:
    if depth >= MAX_DEPTH:
        return MAX_DEPTH_MARKER
    if stack is None:
        stack = set()

    # bool first: it is an int subclass, and must stay a bool rather than be
    # normalized to 1/0 by _exact_scalar.
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, (str, int)):
        return _exact_scalar(value)
    if isinstance(value, float):
        if math.isfinite(value):
            return value
        return "<unsupported:float>"
    if isinstance(value, _RawNumber):
        return value
    if isinstance(value, BaseException):
        return str(value)

    if isinstance(value, dict):
        obj_id = id(value)
        if obj_id in stack:
            return "<unsupported:circular>"
        stack.add(obj_id)
        out: dict[str, Any] = {}
        for k, v in value.items():
            key = _exact_scalar(k) if isinstance(k, str) else str(k)
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
    """Internal redaction context: exact secret-name and URL-name sets."""

    secret_names: frozenset[str] = frozenset()
    url_names: frozenset[str] = frozenset()
    reveal_names: frozenset[str] = frozenset()

    @classmethod
    def from_names(
        cls,
        secret_names: Sequence[str],
        url_names: Sequence[str] = (),
        reveal_names: Sequence[str] = (),
    ) -> _RedactionContext:
        return cls(
            secret_names=frozenset(secret_names),
            url_names=frozenset(url_names),
            reveal_names=frozenset(reveal_names),
        )

    def is_secret_key(self, key: str) -> bool:
        return _key_has_secret_suffix(key) or key in self.secret_names

    def is_url_key(self, key: str) -> bool:
        return _key_has_url_suffix(key) or key in self.url_names


def _key_has_secret_suffix(key: str) -> bool:
    return key.endswith("_secret") or key.endswith("_SECRET")


def _key_has_url_suffix(key: str) -> bool:
    return key.endswith("_url") or key.endswith("_URL")


def _redact_secrets(value: Any, context: _RedactionContext = _RedactionContext(), depth: int = 0) -> None:
    if depth >= MAX_DEPTH:
        return
    if isinstance(value, dict):
        for k in list(value.keys()):
            v = value[k]
            if context.is_secret_key(k) and k not in context.reveal_names:
                # A null secret is an absent secret. Masking it would
                # manufacture the appearance of a configured credential —
                # readers cannot tell "***"-because-set from
                # "***"-because-null. Redaction hides a value that exists;
                # it does not invent one.
                if v is not None:
                    value[k] = REDACTED_MARKER
            elif context.is_url_key(k):
                value[k] = _redact_url_value(v, context, depth + 1)
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


def _redact_url_value(value: Any, context: _RedactionContext, depth: int) -> Any:
    if depth >= MAX_DEPTH:
        return MAX_DEPTH_MARKER
    if isinstance(value, str):
        return _redact_url_field_value(value, context)
    if isinstance(value, list):
        for index, item in enumerate(value):
            value[index] = _redact_url_value(item, context, depth + 1)
        return value
    if isinstance(value, dict):
        for key in list(value.keys()):
            item = value[key]
            if context.is_secret_key(key) and key not in context.reveal_names:
                if item is not None:
                    value[key] = REDACTED_MARKER
            else:
                value[key] = _redact_url_value(item, context, depth + 1)
        return value
    return value


# ═══════════════════════════════════════════
# URL-aware Secret Redaction
# ═══════════════════════════════════════════


def _redact_url_in_str(s: str, context: _RedactionContext, depth: int = 0) -> str | None:
    """Redact secret components of a single URL string.

    Returns the redacted string when ``s`` is a processable URL, or None when it
    is not (so callers keep the original). Only secret spans change; every other
    byte is preserved.
    """
    # Fast path + precondition: a single, whitespace-free, scheme-prefixed URL.
    if "://" not in s or not _is_single_url(s):
        return None
    # Bound recursive redirect inspection, failing closed before stack growth.
    if depth >= 16:
        return REDACTED_MARKER
    scheme_sep = s.find("://")
    scheme = s[:scheme_sep]
    rest = s[scheme_sep + 3 :]

    # Authority runs from after "://" to the first '/', '?', or '#'.
    auth_end = len(rest)
    for i, c in enumerate(rest):
        if c in "/?#":
            auth_end = i
            break
    # An unencoded '?' or '#' inside a password ends that authority early and
    # leaves the rest of the password in the query or fragment. When a ':'
    # precedes that early end and an '@' follows it before the first '/', the
    # userinfo runs to that '@' — how libpq reads such a connection string.
    first_slash = rest.find("/")
    if first_slash == -1:
        first_slash = len(rest)
    at = rest.rfind("@", 0, first_slash)
    if ":" in rest[:auth_end] and at > auth_end:
        auth_end = len(rest)
        for i in range(at, len(rest)):
            if rest[i] in "/?#":
                auth_end = i
                break
    authority = rest[:auth_end]
    remainder = rest[auth_end:]

    # An unencoded '/' inside a password ends the authority at a "port" that
    # is not a number, with the '@' somewhere after it. Where the password
    # ends cannot be known, so the whole value is replaced.
    if "@" not in authority and _has_non_numeric_port(authority) and "@" in remainder:
        return REDACTED_MARKER

    new_authority = _redact_userinfo_password(authority)

    # `remainder` is `path[?query][#fragment]`; '#' ends the query, so split the
    # fragment off first and the query out of what is left.
    before_fragment, hash_sep, fragment = remainder.partition("#")
    path, question_sep, query = before_fragment.partition("?")
    if question_sep:
        new_before_fragment = f"{path}?{_redact_query(query, context, depth)}"
    else:
        new_before_fragment = before_fragment
    # A fragment gets the same treatment as the query: `k=v&k=v` after the '#'
    # is exactly how an OAuth implicit-flow response hands back a token, so a
    # secret-named fragment parameter must not survive where the identically
    # named query parameter would not. A fragment that is not in that shape has
    # no '=' in its segments and passes through byte-for-byte.
    # A hash route (`#/cb?k=v`) carries its parameters after its first '?',
    # while a form reader keeps that '?' inside a value; redact both readings:
    # the fragment as parameters first, so a secret value absorbs its '?',
    # then whatever follows the first '?' that survives.
    if not hash_sep:
        new_fragment = ""
    else:
        params = _redact_query(fragment, context, depth)
        route, route_sep, route_query = params.partition("?")
        if route_sep:
            new_fragment = f"#{route}?{_redact_query(route_query, context, depth)}"
        else:
            new_fragment = f"#{params}"

    return f"{scheme}://{new_authority}{new_before_fragment}{new_fragment}"


def _redact_urls_in_text_with_context(text: str, context: _RedactionContext) -> str:
    output: list[str] = []
    copied_through = 0
    search_from = 0
    while True:
        span = _next_scheme_url_span(text, search_from)
        if span is None:
            break
        start, end = span
        output.append(text[copied_through:start])
        url = text[start:end]
        output.append(_redact_url_in_str(url, context) or url)
        copied_through = end
        search_from = end
    if not output:
        return text
    output.append(text[copied_through:])
    return "".join(output)


def _next_scheme_url_span(text: str, start_at: int) -> tuple[int, int] | None:
    start = start_at
    while start < len(text):
        # Only consider the head of a maximal scheme-character run: a scheme
        # cannot begin mid-run, so a position whose predecessor is a scheme
        # character was already covered by the run that contains it.
        if start == 0 or not _is_scheme_character(text[start - 1]):
            scheme_end = start
            while scheme_end < len(text) and _is_scheme_character(text[scheme_end]):
                scheme_end += 1
            # A scheme must start with a letter, but the run need not: prose like
            # "2https://h/?token_secret=x" glues a digit onto the URL. Retry from
            # the first letter *inside* the run instead of abandoning the span --
            # giving up there fails open and leaks the whole query.
            scheme_start = next(
                (
                    i
                    for i in range(start, scheme_end)
                    if text[i].isascii() and text[i].isalpha()
                ),
                None,
            )
            if scheme_start is not None and text.startswith("://", scheme_end):
                end = scheme_end + 3
                while (
                    end < len(text)
                    and not _is_span_terminator_whitespace(text[end])
                    and not _is_text_url_delimiter(text[end])
                ):
                    end += 1
                while (
                    end > scheme_end + 3
                    and _is_trailing_text_url_punctuation(text[end - 1])
                ):
                    end -= 1
                if end > scheme_end + 3:
                    return scheme_start, end
        start += 1
    return None


def _is_scheme_character(character: str) -> bool:
    return character.isascii() and (character.isalnum() or character in "+-.")


# Characters that end a URL span in prose: the Unicode White_Space property,
# written out here instead of delegated to ``str.isspace()``.
#
# The four AFDATA implementations must cut the span at the same byte, and each
# language's native predicate covers a different set -- ``str.isspace()`` counts
# U+001C-U+001F, JS ``\s`` counts U+FEFF but not U+0085, Rust and Go count
# neither. Divergence is a defect in both directions: ending a span early leaves
# the rest of the query readable, and ending it late pulls the following prose
# into the URL, where redacting the parameter it lands in deletes it. So the set
# is enumerated, and it is generous -- a redaction helper exists to keep a log
# line readable, so anything a human reads as a space must end the URL.
#
# Two exclusions are deliberate, not oversights:
#   * U+FEFF (zero-width no-break space) is not White_Space. Treating it as one
#     would cut a URL carrying it short and leave the secret after it in the
#     clear -- the exact leak JS ``\s`` used to cause here.
#   * U+001C-U+001F (the file/group/record/unit separators) are not White_Space
#     either; only ``str.isspace()`` counted them.
#
# This is a different question from :func:`_is_single_url`, which asks whether a
# whole string is one URL and stays ASCII-only. A span this scanner produces
# contains no whitespace at all, so it satisfies that stricter gate either way.
_SPAN_TERMINATOR_WHITESPACE = frozenset(
    "\u0009\u000a\u000b\u000c\u000d\u0020"  # TAB, LF, VT, FF, CR, SPACE
    "\u0085\u00a0\u1680"  # NEL, NBSP, OGHAM SPACE MARK
    "\u2000\u2001\u2002\u2003\u2004\u2005"  # EN QUAD .. FOUR-PER-EM SPACE
    "\u2006\u2007\u2008\u2009\u200a"  # SIX-PER-EM SPACE .. HAIR SPACE
    "\u2028\u2029"  # LINE SEPARATOR, PARAGRAPH SEPARATOR
    "\u202f\u205f\u3000"  # NARROW NBSP, MEDIUM MATH SPACE, IDEOGRAPHIC SPACE
)


def _is_span_terminator_whitespace(character: str) -> bool:
    return character in _SPAN_TERMINATOR_WHITESPACE


def _is_text_url_delimiter(character: str) -> bool:
    return character in "\"'<>`，。；！？）》】」』"


def _is_trailing_text_url_punctuation(character: str) -> bool:
    return character in ".,;!)]}，。；！）》】"


def _redact_url_field_value(s: str, context: _RedactionContext) -> str:
    redacted = _redact_url_in_str(s, context)
    if redacted is not None:
        return redacted
    trimmed = s.strip("".join(_SPAN_TERMINATOR_WHITESPACE))
    if trimmed != s:
        redacted = _redact_url_in_str(trimmed, context)
        if redacted is not None:
            return redacted
    # Fail closed: a _url value we could not parse as a clean scheme-prefixed
    # URL, yet which carries a credential sigil ('@' userinfo), internal
    # whitespace, or a '://' scheme separator, is redacted wholesale rather than
    # passed through. A schemeless connection string like user:pass@host/db
    # has no scheme anchor for the surgical span logic above, and a URL behind
    # a non-whitespace prefix (a BOM, a zero-width space, a label) has one the
    # span logic cannot reach, so blanket redaction is the safe default.
    if any(_is_span_terminator_whitespace(c) for c in s) or "@" in s or "://" in s:
        return REDACTED_MARKER
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
    return f"{authority[:colon]}:{REDACTED_MARKER}{authority[at:]}"


def _redact_query(query: str, context: _RedactionContext, depth: int) -> str:
    """Redact the values of secret-named query parameters.

    Preserves the raw bytes of every other segment (keys, benign values,
    encoding, ordering, separators). Also drives fragment bodies, which carry
    the same ``k=v&k=v`` shape.
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
            segments.append(f"{raw_key}={REDACTED_MARKER}")
            continue
        # A redirect target carries its own parameters: redact a nested URL in
        # place, or the whole value when it arrived encoded.
        raw_value = segment[eq + 1 :]
        value = unquote_plus(raw_value)
        redacted = _redact_url_in_str(value, context, depth + 1)
        if redacted is not None and redacted != value:
            if value == raw_value:
                segments.append(f"{raw_key}={redacted}")
            else:
                segments.append(f"{raw_key}={REDACTED_MARKER}")
        else:
            segments.append(segment)
    return "&".join(segments)


def _has_non_numeric_port(authority: str) -> bool:
    """True when the authority's text after its last ':' is not a port number.

    A bracketed IPv6 literal without a port ends in ']' and has none.
    """
    colon = authority.rfind(":")
    if colon == -1 or authority.endswith("]"):
        return False
    return not all("0" <= c <= "9" for c in authority[colon + 1 :])


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
    if isinstance(value, float):
        return _number_literal_str(repr(value))
    return str(value)


def _number_literal_str(literal: str) -> str:
    """Render a number's literal canonically for YAML/plain output: a float
    literal whose value is an integer below 1e21 in magnitude renders as that
    integer (``3.0`` -> ``3``, ``-0.0`` -> ``-0``); anything else keeps its
    digits, placed by ``_place_decimal_point``. Native floats (via ``repr``)
    and decoded literals share this, and it works on the text so a digit is
    never lost to a float64 round trip."""
    integral = _integral_float_text(literal)
    if integral is not None:
        return integral
    placed = _place_decimal_point(literal)
    return placed if placed is not None else _normalize_exponent(literal)


_I64_MIN = -(2**63)
_I64_MAX = 2**63 - 1


def _place_decimal_point(literal: str) -> str | None:
    """Spell a non-integral float literal with the notation ECMAScript's
    ``Number#toString`` picks: positional when the value lies in [1e-6, 1e21),
    exponent form otherwise. ``repr`` switches below 1e-4 and the other SDKs'
    native formatters elsewhere, so the choice is made here, on the digits as
    written. ``None`` for integer literals and exponents beyond i64, which keep
    their spelling."""
    if not any(c in literal for c in ".eE"):
        return None
    sign = "-" if literal.startswith("-") else ""
    unsigned = literal[1:] if sign else literal
    mantissa, *exponent = re.split("[eE]", unsigned, maxsplit=1)
    whole, _, fraction = mantissa.partition(".")
    digits = whole + fraction
    significant = digits.lstrip("0")
    if not significant:
        return None
    exponent_text = exponent[0] if exponent else "0"
    shape = re.fullmatch(r"([+-]?)0*([0-9]*)", exponent_text)
    if shape is None or not exponent_text.lstrip("+-") or len(shape.group(2)) > 19:
        return None
    exp = int(shape.group(1) + (shape.group(2) or "0"))
    # point: how many significant digits precede the decimal point.
    point = len(whole) - (len(digits) - len(significant)) + exp
    if not (_I64_MIN <= exp <= _I64_MAX and _I64_MIN <= point - 1 and point <= _I64_MAX):
        return None
    if 1 <= point <= 21:
        integer, rest = significant[:point], significant[point:]
        if not rest:
            return None
        text = f"{integer}.{rest}"
    elif -5 <= point <= 0:
        text = "0." + "0" * -point + significant
    else:
        scale = point - 1
        rest = f".{significant[1:]}" if len(significant) > 1 else ""
        text = f"{significant[0]}{rest}e{'-' if scale < 0 else '+'}{abs(scale)}"
    return sign + text


def _integral_float_text(literal: str) -> str | None:
    if not any(c in literal for c in ".eE"):
        return None
    negative = literal.startswith("-")
    unsigned = literal[1:] if negative else literal
    mantissa, *exponent = re.split("[eE]", unsigned, maxsplit=1)
    whole, _, fraction = mantissa.partition(".")
    digits = whole + fraction
    if not digits.strip("0"):
        return "-0" if negative else "0"
    try:
        exp = int(exponent[0]) if exponent else 0
    except ValueError:
        return None
    point = len(whole) + exp
    split = min(max(point, 0), len(digits))
    if any(d != "0" for d in digits[split:]):
        return None
    padding = min(max(point - len(digits), 0), 22)
    integer = (digits[:split] + "0" * padding).lstrip("0")
    if len(integer) > 21:
        return None
    return ("-" if negative else "") + (integer or "0")


def _normalize_exponent(s: str) -> str:
    if "e" not in s and "E" not in s:
        return s
    mantissa, exp = re.split("[eE]", s, maxsplit=1)
    # An unsigned exponent is positive; spell it like repr() does (e+23).
    sign = "+"
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
    # A float is read through its shortest spelling (``repr``), the text the
    # canonical number form prints, so 2.0**60 counts as 1152921504606847000
    # exactly as a decoded literal of that spelling does in every SDK; int()
    # of the binary value would give ...846976 and disagree with Rust.
    if isinstance(value, float) and math.isfinite(value):
        literal = repr(value)
        integer = _integral_float_text(literal) if re.search(r"[.eE]", literal) else literal
        return int(integer) if integer is not None else None
    return None


def _raw_as_int(value: Any) -> int | None:
    if isinstance(value, _RawNumber):
        if re.fullmatch(r"-?[0-9]+", value.literal):
            return int(value.literal) if len(value.literal.lstrip("-")) <= 20 else None
        integer = _integral_float_text(value.literal)
        return int(integer) if integer is not None else None
    return _as_int(value)


def _as_u64(value: Any) -> int | None:
    n = _raw_as_int(value)
    if n is not None and 0 <= n <= 18_446_744_073_709_551_615:
        return n
    return None


def _as_i64(value: Any) -> int | None:
    n = _raw_as_int(value)
    if n is not None and -9_223_372_036_854_775_808 <= n <= 9_223_372_036_854_775_807:
        return n
    return None


def _signed_magnitude(value: int) -> tuple[str, int]:
    return ("-", -value) if value < 0 else ("", value)


def _as_decimal_int(value: Any) -> int | None:
    text = _decimal_int_text(value)
    return int(text) if text is not None else None


def _decimal_int_text(value: Any) -> str | None:
    """The exact digits of an integer-valued number or decimal integer string,
    of any magnitude. Bitcoin amounts and ``_epoch_ns`` only append a unit or
    divide, so unlike money suffixes they are not bounded by i64: an integer
    literal keeps its digits, and a float counts when its value is an integer
    below 1e21 (the canonical number form's integer range). A numeric ``-0``
    is zero."""
    if isinstance(value, str):
        return value if re.fullmatch(r"-?[0-9]+", value) else None
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        if not math.isfinite(value):
            return None
        literal = repr(value)
    elif isinstance(value, _RawNumber):
        literal = value.literal
    else:
        return None
    integer = _integral_float_text(literal) if re.search(r"[.eE]", literal) else literal
    if integer is None:
        return None
    return "0" if integer == "-0" else integer


def _try_process_field(key: str, value: Any) -> tuple[str, str] | None:
    """Try suffix-driven processing. Returns (stripped_key, formatted_value) or None."""
    # Adding a unit does no arithmetic, so it must not round decoded digits.
    is_number = _is_number(value) or isinstance(value, _RawNumber)

    # Group 1: compound timestamp suffixes
    stripped = _strip_suffix_ci(key, "_epoch_ms")
    if stripped is not None:
        n = _as_i64(value)
        if n is not None:
            formatted = _format_rfc3339_ms(n)
            if formatted is not None:
                return stripped, formatted
        return None
    stripped = _strip_suffix_ci(key, "_epoch_s")
    if stripped is not None:
        n = _as_i64(value)
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
        n = _as_i64(value)
        if n is not None:
            sign, magnitude = _signed_magnitude(n)
            return stripped, f"{sign}${magnitude // 100}.{magnitude % 100:02d}"
        return None
    stripped = _strip_suffix_ci(key, "_eur_cents")
    if stripped is not None:
        n = _as_i64(value)
        if n is not None:
            sign, magnitude = _signed_magnitude(n)
            return stripped, f"{sign}\u20ac{magnitude // 100}.{magnitude % 100:02d}"
        return None
    gc = _try_strip_generic_cents(key)
    if gc is not None:
        stripped, code = gc
        n = _as_i64(value)
        if n is not None:
            sign, magnitude = _signed_magnitude(n)
            return (
                stripped,
                f"{sign}{magnitude // 100}.{magnitude % 100:02d} {code.upper()}",
            )
        return None
    gm = _try_strip_generic_micro(key)
    if gm is not None:
        stripped, code = gm
        n = _as_i64(value)
        if n is not None:
            sign, magnitude = _signed_magnitude(n)
            return (
                stripped,
                f"{sign}{magnitude // 1_000_000}.{magnitude % 1_000_000:06d} {code.upper()}",
            )
        return None

    # Group 3: multi-char suffixes
    stripped = _strip_suffix_ci(key, "_rfc3339")
    if stripped is not None:
        if isinstance(value, str):
            return stripped, value
        return None
    stripped = _strip_suffix_ci(key, "_minutes")
    if stripped is not None:
        if is_number:
            return stripped, f"{_plain_scalar(value)} minutes"
        return None
    stripped = _strip_suffix_ci(key, "_hours")
    if stripped is not None:
        if is_number:
            return stripped, f"{_plain_scalar(value)} hours"
        return None
    stripped = _strip_suffix_ci(key, "_days")
    if stripped is not None:
        if is_number:
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
        n = _as_u64(value)
        if n is not None:
            return stripped, _format_bytes_human(n)
        return None
    stripped = _strip_suffix_ci(key, "_percent")
    if stripped is not None:
        if is_number:
            return stripped, f"{_plain_scalar(value)}%"
        return None
    # Group 5: short suffixes (last to avoid false positives)
    stripped = _strip_suffix_ci(key, "_jpy")
    if stripped is not None:
        n = _as_i64(value)
        if n is not None:
            sign, magnitude = _signed_magnitude(n)
            return stripped, f"{sign}\u00a5{_format_with_commas(magnitude)}"
        return None
    stripped = _strip_suffix_ci(key, "_ns")
    if stripped is not None:
        if is_number:
            return stripped, f"{_plain_scalar(value)}ns"
        return None
    stripped = _strip_suffix_ci(key, "_us")
    if stripped is not None:
        if is_number:
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
        if is_number:
            return stripped, f"{_plain_scalar(value)}s"
        return None

    return None


def _is_redacted(value: Any) -> bool:
    """True when a field already carries the redaction marker, i.e. redaction
    ran over it and hid the value."""
    return isinstance(value, str) and value == REDACTED_MARKER


def _process_object_fields(d: dict) -> list[tuple[str, Any, str | None]]:
    """Process fields: strip keys, format values, detect collisions.

    Returns list of (display_key, value, formatted_value_or_None).
    """
    entries: list[tuple[str, str, Any, str | None]] = []
    for k, v in d.items():
        stripped_secret = _strip_suffix_ci(k, "_secret")
        if stripped_secret is not None:
            # Dropping `_secret` is the readable half of an actual redaction:
            # the marker has done its job once the value reads `***`. When the
            # value was *not* redacted — policy `Off`, or a field outside
            # `trace` under `TraceOnly` — the suffix is the only thing telling a
            # downstream reader that `api_key_secret=sk-live-xxx` is a
            # credential, so it stays.
            display_key = stripped_secret if _is_redacted(v) else k
            entries.append((display_key, k, v, None))
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
    if not _is_number(value) and not isinstance(value, _RawNumber):
        return None
    n = float(value.literal) if isinstance(value, _RawNumber) else float(value)
    if not math.isfinite(n):
        return None
    if abs(n) >= 1000:
        return _format_ms_as_seconds(n)
    return f"{_plain_scalar(value)}ms"


def _format_rfc3339_ms(ms: int) -> str | None:
    if ms < MIN_RFC3339_MS or ms > MAX_RFC3339_MS:
        return None
    try:
        dt = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=ms)
        # Not strftime: glibc's `%Y` does not pad a year below 1000, so year 1
        # printed as `1-01-01` on Linux while macOS printed `0001-01-01`.
        return (
            f"{dt.year:04d}-{dt.month:02d}-{dt.day:02d}T"
            f"{dt.hour:02d}:{dt.minute:02d}:{dt.second:02d}.{ms % 1000:03d}Z"
        )
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
    escaped = []
    short = {
        "\\": "\\\\",
        '"': '\\"',
        "\n": "\\n",
        "\r": "\\r",
        "\t": "\\t",
        "\b": "\\b",
        "\f": "\\f",
        "\v": "\\v",
        "\0": "\\0",
    }
    for character in s:
        if character in short:
            escaped.append(short[character])
        elif ord(character) <= 0x1F:
            escaped.append(f"\\u{ord(character):04x}")
        else:
            escaped.append(character)
    return "".join(escaped)


def _yaml_key(key: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_.-]+", key) and not _is_ambiguous_yaml_key(key):
        return key
    return f'"{_escape_yaml_str(key)}"'


def _is_ambiguous_yaml_key(key: str) -> bool:
    if key.lower() in {"true", "false", "null", "~", ".nan", ".inf", "+.inf", "-.inf"}:
        return True
    unsigned = key[1:] if key.startswith(("+", "-")) else key
    return unsigned.lower() in {"inf", "infinity", "nan"} or re.fullmatch(
        r"[-+]?(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:[eE][-+]?[0-9]+)?", key
    ) is not None


def _yaml_scalar(value: Any) -> str:
    if isinstance(value, str):
        return f'"{_escape_yaml_str(value)}"'
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return _number_str(value)
    if isinstance(value, _RawNumber):
        # Decoded number: its exact digits, in the same canonical form a
        # native number gets.
        return _number_literal_str(value.literal)
    if isinstance(value, (dict, list)):
        return f'"{_escape_yaml_str(_embedded_json(value))}"'
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
            if v:
                _collect_plain_pairs(v, full_key, pairs)
            else:
                pairs.append((full_key, "{}"))
        elif isinstance(v, list):
            if v:
                joined = ",".join(_plain_scalar(item) for item in v)
                pairs.append((full_key, joined))
            else:
                pairs.append((full_key, "[]"))
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
            if v:
                _collect_plain_pairs_raw(v, full_key, pairs)
            else:
                pairs.append((full_key, "{}"))
        elif isinstance(v, list):
            if v:
                joined = ",".join(_plain_scalar_raw(item) for item in v)
                pairs.append((full_key, joined))
            else:
                pairs.append((full_key, "[]"))
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
    if isinstance(value, _RawNumber):
        # Decoded number: its exact digits (Plain is documented lossy only
        # for arithmetic-derived formatting), in the same canonical form a
        # native number gets.
        return _number_literal_str(value.literal)
    if isinstance(value, (dict, list)):
        return _embedded_json(value)
    return str(value)


def _plain_scalar_raw(value: Any) -> str:
    if isinstance(value, (dict, list)):
        return _embedded_json(value)
    return _plain_scalar(value)


def _quote_logfmt_value(value: str) -> str:
    if value == "":
        return ""
    needs_quote = any(c in ' \t\n\r\f\v\u00a0="\\' for c in value)
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


def _embedded_json(value: Any) -> str:
    """A container nested inside a Plain array (or in a YAML scalar position)
    is written as compact JSON with sorted keys. Its number leaves take the
    canonical number form, the same text the value prints as a scalar, so a
    nested ``1.0`` reads ``1`` and a decoded ``1e5`` reads ``100000`` in every
    SDK rather than in each runtime's own JSON spelling."""
    if isinstance(value, dict):
        return "{" + ",".join(
            f"{json.dumps(k, ensure_ascii=False)}:{_embedded_json(value[k])}"
            for k in _sorted_object_keys(value)
        ) + "}"
    if isinstance(value, list):
        return "[" + ",".join(_embedded_json(item) for item in value) + "]"
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if value is None or isinstance(value, (bool, int, float, _RawNumber)):
        return _plain_scalar(value)
    return json.dumps(str(value), ensure_ascii=False)
