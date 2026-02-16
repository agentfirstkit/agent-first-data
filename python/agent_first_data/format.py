"""AFD output formatting and protocol templates.

9 public APIs: 4 protocol builders + 3 output formatters + 1 redaction + 1 utility.
"""

from __future__ import annotations

import copy
import json
from datetime import datetime, timezone
from typing import Any


# ═══════════════════════════════════════════
# Public API: Protocol Builders
# ═══════════════════════════════════════════


def build_json_startup(config: Any, args: Any, env: Any) -> dict:
    """Build {code: "startup", config, args, env}."""
    return {"code": "startup", "config": config, "args": args, "env": env}


def build_json_ok(result: Any, trace: Any = None) -> dict:
    """Build {code: "ok", result, trace?}."""
    m: dict = {"code": "ok", "result": result}
    if trace is not None:
        m["trace"] = trace
    return m


def build_json_error(message: str, trace: Any = None) -> dict:
    """Build {code: "error", error: message, trace?}."""
    m: dict = {"code": "error", "error": message}
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


def output_json(value: Any) -> str:
    """Format as single-line JSON. Secrets redacted, original keys, raw values."""
    v = copy.deepcopy(value)
    _redact_secrets(v)
    return json.dumps(v, ensure_ascii=False, separators=(",", ":"))


def output_yaml(value: Any) -> str:
    """Format as multi-line YAML. Keys stripped, values formatted, secrets redacted."""
    lines = ["---"]
    _render_yaml_processed(value, 0, lines)
    return "\n".join(lines)


def output_plain(value: Any) -> str:
    """Format as single-line logfmt. Keys stripped, values formatted, secrets redacted."""
    pairs: list[tuple[str, str]] = []
    _collect_plain_pairs(value, "", pairs)
    pairs.sort(key=lambda p: p[0].encode("utf-16-be"))
    parts = []
    for k, v in pairs:
        if " " in v:
            parts.append(f'{k}="{v}"')
        else:
            parts.append(f"{k}={v}")
    return " ".join(parts)


# ═══════════════════════════════════════════
# Public API: Redaction & Utility
# ═══════════════════════════════════════════


def internal_redact_secrets(value: Any) -> None:
    """Redact _secret fields in-place."""
    _redact_secrets(value)


def parse_size(s: str) -> int | None:
    """Parse a human-readable size string into bytes.

    Accepts bare numbers or numbers followed by a unit letter (B/K/M/G/T).
    Case-insensitive. Trims whitespace. Returns None for invalid input.
    """
    _multipliers = {"b": 1, "k": 1024, "m": 1024**2, "g": 1024**3, "t": 1024**4}
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
        return n * mult
    except ValueError:
        pass
    try:
        f = float(num_str)
        if f < 0 or f != f:  # NaN check
            return None
        return int(f * mult)
    except (ValueError, OverflowError):
        return None


# ═══════════════════════════════════════════
# Secret Redaction
# ═══════════════════════════════════════════


def _redact_secrets(value: Any) -> None:
    if isinstance(value, dict):
        for k in list(value.keys()):
            if k.endswith("_secret") or k.endswith("_SECRET"):
                if isinstance(value[k], (dict, list)):
                    _redact_secrets(value[k])
                else:
                    value[k] = "***"
            else:
                _redact_secrets(value[k])
    elif isinstance(value, list):
        for item in value:
            _redact_secrets(item)


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
