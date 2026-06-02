"""AFDATA output formatting and protocol templates.

18 public APIs and 4 types: protocol builders, value redactors (copy and
in-place; cover _secret and _url fields), output formatters, URL-string
redactors (redact_url_secrets / _with_options), parse_size, RedactionPolicy,
RedactionOptions, OutputStyle, and OutputOptions.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Any, Sequence
from urllib.parse import unquote_plus


# ═══════════════════════════════════════════
# Public API: Protocol Builders
# ═══════════════════════════════════════════


def build_json_ok(result: Any, trace: Any = None) -> dict:
    """Build {code: "ok", result, trace?}."""
    m: dict = {"code": "ok", "result": result}
    if trace is not None:
        m["trace"] = trace
    return m


def build_json_error(message: str, hint: str | None = None, trace: Any = None) -> dict:
    """Build {code: "error", error: message, hint?, trace?}."""
    m: dict = {"code": "error", "error": message}
    if hint is not None:
        m["hint"] = hint
    if trace is not None:
        m["trace"] = trace
    return m


def build_json(code: str, fields: Any, trace: Any = None) -> dict:
    """Build {code: "<custom>", ...fields, trace?}."""
    result = dict(fields) if isinstance(fields, dict) else {}
    result["code"] = code
    if trace is not None:
        result["trace"] = trace
    return result


# ═══════════════════════════════════════════
# Public API: Output Formatters
# ═══════════════════════════════════════════

class RedactionPolicy(str, Enum):
    RedactionTraceOnly = "RedactionTraceOnly"
    RedactionNone = "RedactionNone"
    RedactionStrict = "RedactionStrict"


@dataclass(frozen=True)
class RedactionOptions:
    """Redaction options for legacy secret field names."""

    policy: RedactionPolicy | None = None
    # Exact field-name matches at any nesting level. The same list also matches
    # URL query-parameter names inside _url fields (see redact_url_secrets).
    secret_names: Sequence[str] = ()


class OutputStyle(str, Enum):
    """Rendering style for YAML and plain output."""

    Readable = "Readable"
    Raw = "Raw"


@dataclass(frozen=True)
class OutputOptions:
    """Output options combining redaction and rendering style."""

    redaction: RedactionOptions = field(default_factory=RedactionOptions)
    style: OutputStyle = OutputStyle.Readable


def output_json(value: Any) -> str:
    """Format as single-line JSON. Secrets redacted, original keys, raw values."""
    return json.dumps(redacted_value(value), ensure_ascii=False, separators=(",", ":"))


def output_json_with(value: Any, redaction_policy: RedactionPolicy) -> str:
    """Format as single-line JSON with explicit redaction policy."""
    return json.dumps(redacted_value_with(value, redaction_policy), ensure_ascii=False, separators=(",", ":"))


def output_json_with_options(value: Any, output_options: OutputOptions) -> str:
    """Format as single-line JSON with explicit output options."""
    return json.dumps(
        redacted_value_with_options(value, output_options.redaction),
        ensure_ascii=False,
        separators=(",", ":"),
    )


def output_yaml(value: Any) -> str:
    """Format as multi-line YAML. Keys stripped, values formatted, secrets redacted."""
    return output_yaml_with_options(value, OutputOptions())


def output_yaml_with_options(value: Any, output_options: OutputOptions) -> str:
    """Format as multi-line YAML with explicit output options."""
    value = redacted_value_with_options(value, output_options.redaction)
    lines = ["---"]
    if output_options.style is OutputStyle.Raw:
        _render_yaml_raw(value, 0, lines)
    else:
        _render_yaml_processed(value, 0, lines)
    return "\n".join(lines)


def output_plain(value: Any) -> str:
    """Format as single-line logfmt. Keys stripped, values formatted, secrets redacted."""
    return output_plain_with_options(value, OutputOptions())


def output_plain_with_options(value: Any, output_options: OutputOptions) -> str:
    """Format as single-line logfmt with explicit output options."""
    value = redacted_value_with_options(value, output_options.redaction)
    pairs: list[tuple[str, str]] = []
    if output_options.style is OutputStyle.Raw:
        _collect_plain_pairs_raw(value, "", pairs)
    else:
        _collect_plain_pairs(value, "", pairs)
    pairs.sort(key=lambda p: p[0].encode("utf-16-be"))
    parts = []
    for k, v in pairs:
        parts.append(f"{k}={_quote_logfmt_value(v)}")
    return " ".join(parts)


# ═══════════════════════════════════════════
# Public API: Redaction & Utility
# ═══════════════════════════════════════════


def internal_redact_secrets(value: Any) -> None:
    """Redact _secret fields in-place."""
    _redact_secrets(value)


def internal_redact_secrets_with_options(value: Any, redaction_options: RedactionOptions) -> None:
    """Redact secret fields in-place using explicit redaction options."""
    _apply_redaction_options(value, redaction_options)


def redacted_value(value: Any) -> Any:
    """Return a JSON-safe copy with default _secret redaction applied."""
    v = _sanitize_for_json(value)
    _redact_secrets(v)
    return v


def redacted_value_with(value: Any, redaction_policy: RedactionPolicy) -> Any:
    """Return a JSON-safe copy with an explicit redaction policy applied."""
    v = _sanitize_for_json(value)
    _apply_redaction_policy(v, redaction_policy)
    return v


def redacted_value_with_options(value: Any, redaction_options: RedactionOptions) -> Any:
    """Return a JSON-safe copy with explicit redaction options applied."""
    v = _sanitize_for_json(value)
    _apply_redaction_options(v, redaction_options)
    return v


def redact_url_secrets(url: str) -> str:
    """Redact secret components of a single URL string, using default options.

    Returns ``url`` with its userinfo password and any ``_secret``-suffixed
    query-parameter values replaced by ``***``. See
    :func:`redact_url_secrets_with_options`.
    """
    return redact_url_secrets_with_options(url, RedactionOptions())


def redact_url_secrets_with_options(url: str, redaction_options: RedactionOptions) -> str:
    """Redact secret components of a single URL string.

    A query parameter is redacted iff its (form-decoded) name ends in
    ``_secret``/``_SECRET`` or matches an exact entry in ``secret_names``. The
    userinfo password (``scheme://user:pass@host``) is always redacted as a
    structural rule. Only the secret spans are replaced with ``***``; every
    other byte is preserved. A string that is not a single, whitespace-free,
    scheme-prefixed URL (including a URL embedded in surrounding prose) is
    returned unchanged.
    """
    context = _RedactionContext.from_options(redaction_options)
    redacted = _redact_url_in_str(url, context)
    return redacted if redacted is not None else url


def _apply_redaction_policy(value: Any, redaction_policy: RedactionPolicy) -> None:
    _apply_redaction_policy_with_context(value, redaction_policy, _RedactionContext())


def _apply_redaction_options(value: Any, redaction_options: RedactionOptions) -> None:
    context = _RedactionContext.from_options(redaction_options)
    _apply_redaction_policy_with_context(value, redaction_options.policy, context)


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
    if redaction_policy == RedactionPolicy.RedactionStrict:
        _redact_secrets_strict(value, context)
        return
    # Empty/unknown policy falls back to default full redaction.
    _redact_secrets(value, context)


def parse_size(s: str) -> int | None:
    """Parse a human-readable size string into bytes.

    Accepts bare numbers or numbers followed by a unit letter (B/K/M/G/T).
    Case-insensitive. Trims whitespace. Returns None for invalid input.
    """
    _multipliers = {"b": 1, "k": 1024, "m": 1024**2, "g": 1024**3, "t": 1024**4}
    _max_u64 = (1 << 64) - 1
    s = s.strip()
    if not s:
        return None
    last = s[-1].lower()
    if last in _multipliers:
        num_str = s[:-1]
        mult = _multipliers[last]
    elif last.isdigit() or last == ".":
        num_str = s
        mult = 1
    else:
        return None
    if not num_str:
        return None
    try:
        n = int(num_str)
        if n < 0:
            return None
        if n > _max_u64 // mult:
            return None
        return n * mult
    except ValueError:
        pass
    try:
        f = float(num_str)
        if f < 0 or not math.isfinite(f):
            return None
        result = f * mult
        if not math.isfinite(result) or result > _max_u64:
            return None
        return int(result)
    except (ValueError, OverflowError):
        return None


# ═══════════════════════════════════════════
# Secret Redaction
# ═══════════════════════════════════════════


def _sanitize_for_json(value: Any, stack: set[int] | None = None) -> Any:
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
            out[key] = _sanitize_for_json(v, stack)
        stack.remove(obj_id)
        return out

    if isinstance(value, (list, tuple)):
        obj_id = id(value)
        if obj_id in stack:
            return "<unsupported:circular>"
        stack.add(obj_id)
        out = [_sanitize_for_json(item, stack) for item in value]
        stack.remove(obj_id)
        return out

    return f"<unsupported:{type(value).__name__}>"


@dataclass(frozen=True)
class _RedactionContext:
    """Internal redaction context: the secret-name set."""

    secret_names: frozenset[str] = frozenset()

    @classmethod
    def from_options(cls, redaction_options: RedactionOptions) -> _RedactionContext:
        return cls(secret_names=frozenset(redaction_options.secret_names))

    def is_secret_key(self, key: str) -> bool:
        return _key_has_secret_suffix(key) or key in self.secret_names


def _key_has_secret_suffix(key: str) -> bool:
    return key.endswith("_secret") or key.endswith("_SECRET")


def _key_has_url_suffix(key: str) -> bool:
    return key.endswith("_url") or key.endswith("_URL")


def _redact_secrets(value: Any, context: _RedactionContext = _RedactionContext()) -> None:
    if isinstance(value, dict):
        for k in list(value.keys()):
            v = value[k]
            if context.is_secret_key(k):
                if isinstance(v, (dict, list)):
                    _redact_secrets(v, context)
                else:
                    value[k] = "***"
            elif _key_has_url_suffix(k):
                if isinstance(v, str):
                    redacted = _redact_url_in_str(v, context)
                    if redacted is not None:
                        value[k] = redacted
                else:
                    _redact_secrets(v, context)
            else:
                _redact_secrets(v, context)
    elif isinstance(value, list):
        for item in value:
            _redact_secrets(item, context)


def _redact_secrets_strict(value: Any, context: _RedactionContext = _RedactionContext()) -> None:
    if isinstance(value, dict):
        for k in list(value.keys()):
            v = value[k]
            if context.is_secret_key(k):
                value[k] = "***"
            elif _key_has_url_suffix(k):
                if isinstance(v, str):
                    redacted = _redact_url_in_str(v, context)
                    if redacted is not None:
                        value[k] = redacted
                else:
                    _redact_secrets_strict(v, context)
            else:
                _redact_secrets_strict(v, context)
    elif isinstance(value, list):
        for item in value:
            _redact_secrets_strict(item, context)


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


def _redact_userinfo_password(authority: str) -> str:
    """Replace the userinfo password (``user:pass@``) with ``***``.

    Preserves the username. Authority without ``@``, or userinfo without ``:``,
    is unchanged.
    """
    at = authority.find("@")
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


def _is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _as_int(value: Any) -> int | None:
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    return None


def _as_non_neg_int(value: Any) -> int | None:
    n = _as_int(value)
    if n is not None and n >= 0:
        return n
    return None


def _try_process_field(key: str, value: Any) -> tuple[str, str] | None:
    """Try suffix-driven processing. Returns (stripped_key, formatted_value) or None."""
    # Group 1: compound timestamp suffixes
    stripped = _strip_suffix_ci(key, "_epoch_ms")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            return stripped, _format_rfc3339_ms(n)
        return None
    stripped = _strip_suffix_ci(key, "_epoch_s")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            return stripped, _format_rfc3339_ms(n * 1000)
        return None
    stripped = _strip_suffix_ci(key, "_epoch_ns")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            return stripped, _format_rfc3339_ms(n // 1_000_000)
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
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}msats"
        return None
    stripped = _strip_suffix_ci(key, "_sats")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}sats"
        return None
    stripped = _strip_suffix_ci(key, "_bytes")
    if stripped is not None:
        n = _as_int(value)
        if n is not None:
            return stripped, _format_bytes_human(n)
        return None
    stripped = _strip_suffix_ci(key, "_percent")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)}%"
        return None
    stripped = _strip_suffix_ci(key, "_secret")
    if stripped is not None:
        return stripped, "***"

    # Group 5: short suffixes (last to avoid false positives)
    stripped = _strip_suffix_ci(key, "_btc")
    if stripped is not None:
        if _is_number(value):
            return stripped, f"{_plain_scalar(value)} BTC"
        return None
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
    result_list.sort(key=lambda x: x[0].encode("utf-16-be"))
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


def _format_rfc3339_ms(ms: int) -> str:
    try:
        dt = datetime.fromtimestamp(ms / 1000, tz=timezone.utc)
        return dt.strftime("%Y-%m-%dT%H:%M:%S.") + f"{ms % 1000:03d}Z"
    except (OSError, OverflowError, ValueError):
        return str(ms)


def _format_bytes_human(n: int) -> str:
    KB = 1024.0
    MB = KB * 1024
    GB = MB * 1024
    TB = GB * 1024
    sign = "-" if n < 0 else ""
    b = float(abs(n))
    if b >= TB:
        return f"{sign}{b / TB:.1f}TB"
    if b >= GB:
        return f"{sign}{b / GB:.1f}GB"
    if b >= MB:
        return f"{sign}{b / MB:.1f}MB"
    if b >= KB:
        return f"{sign}{b / KB:.1f}KB"
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
    idx = without_cents.rfind("_")
    if idx < 0:
        return None
    code = without_cents[idx + 1 :]
    if not code:
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
            lines.append(f'{prefix}{display_key}: "{_escape_yaml_str(formatted)}"')
        elif isinstance(v, dict):
            if v:
                lines.append(f"{prefix}{display_key}:")
                _render_yaml_processed(v, indent + 1, lines)
            else:
                lines.append(f"{prefix}{display_key}: {{}}")
        elif isinstance(v, list):
            if not v:
                lines.append(f"{prefix}{display_key}: []")
            else:
                lines.append(f"{prefix}{display_key}:")
                for item in v:
                    if isinstance(item, dict):
                        lines.append(f"{prefix}  -")
                        _render_yaml_processed(item, indent + 2, lines)
                    else:
                        lines.append(f"{prefix}  - {_yaml_scalar(item)}")
        else:
            lines.append(f"{prefix}{display_key}: {_yaml_scalar(v)}")


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
            lines.append(f"{prefix}{key}:")
            _render_yaml_raw(value, indent + 1, lines)
        else:
            lines.append(f"{prefix}{key}: {{}}")
    elif isinstance(value, list):
        if value:
            lines.append(f"{prefix}{key}:")
            _render_yaml_array_raw(value, indent + 1, lines)
        else:
            lines.append(f"{prefix}{key}: []")
    else:
        lines.append(f"{prefix}{key}: {_yaml_scalar(value)}")


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
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def _yaml_scalar(value: Any) -> str:
    if isinstance(value, str):
        return f'"{_escape_yaml_str(value)}"'
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    escaped = str(value).replace('"', '\\"')
    return f'"{escaped}"'


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
        return str(value)
    return str(value)


def _plain_scalar_raw(value: Any) -> str:
    if isinstance(value, (dict, list)):
        return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
    return _plain_scalar(value)


def _quote_logfmt_value(value: str) -> str:
    if value == "":
        return ""
    needs_quote = any(c.isspace() or c in '="\\"' for c in value)
    if not needs_quote:
        return value
    escaped = (
        value.replace("\\", "\\\\")
        .replace('"', '\\"')
        .replace("\n", "\\n")
        .replace("\r", "\\r")
        .replace("\t", "\\t")
    )
    return f'"{escaped}"'


def _sorted_object_keys(d: dict) -> list[str]:
    return sorted(d.keys(), key=lambda k: k.encode("utf-16-be"))
