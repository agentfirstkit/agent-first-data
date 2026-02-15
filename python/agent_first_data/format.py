"""AFD output formatting: JSON, YAML, plain text with suffix-driven transforms."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from enum import Enum
from typing import Any


class OutputFormat(Enum):
    JSON = "json"
    YAML = "yaml"
    PLAIN = "plain"

    def format(self, value: Any) -> str:
        if self is OutputFormat.JSON:
            return json.dumps(value, ensure_ascii=False, separators=(",", ":"))
        if self is OutputFormat.YAML:
            return to_yaml(value)
        return to_plain(value)

    def format_pretty(self, value: Any) -> str:
        if self is OutputFormat.JSON:
            return json.dumps(value, ensure_ascii=False, indent=2)
        if self is OutputFormat.YAML:
            return to_yaml(value)
        return to_plain(value)


# ═══════════════════════════════════════════
# YAML
# ═══════════════════════════════════════════


def to_yaml(value: Any) -> str:
    lines = ["---"]
    _render_yaml(value, 0, lines)
    return "\n".join(lines)


def _render_yaml(value: Any, indent: int, lines: list[str]) -> None:
    prefix = "  " * indent
    if isinstance(value, dict):
        for k, v in sorted(value.items(), key=lambda kv: kv[0].encode("utf-16-be")):
            if isinstance(v, dict):
                if v:
                    lines.append(f"{prefix}{k}:")
                    _render_yaml(v, indent + 1, lines)
                else:
                    lines.append(f"{prefix}{k}: {{}}")
            elif isinstance(v, list):
                if not v:
                    lines.append(f"{prefix}{k}: []")
                else:
                    lines.append(f"{prefix}{k}:")
                    for item in v:
                        if isinstance(item, dict):
                            lines.append(f"{prefix}  -")
                            _render_yaml(item, indent + 2, lines)
                        else:
                            lines.append(f"{prefix}  - {_yaml_scalar(item)}")
            else:
                lines.append(f"{prefix}{k}: {_yaml_scalar(v)}")
    else:
        lines.append(f"{prefix}{_yaml_scalar(value)}")


def _yaml_scalar(value: Any) -> str:
    if isinstance(value, str):
        escaped = value.replace("\\", "\\\\").replace('"', '\\"')
        escaped = escaped.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
        return f'"{escaped}"'
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    escaped = str(value).replace('"', '\\"')
    return f'"{escaped}"'


# ═══════════════════════════════════════════
# Plain
# ═══════════════════════════════════════════


def to_plain(value: Any) -> str:
    lines: list[str] = []
    _render_plain(value, 0, lines)
    return "\n".join(lines)


def _render_plain(value: Any, indent: int, lines: list[str]) -> None:
    prefix = "  " * indent
    if isinstance(value, dict):
        for k, v in sorted(value.items(), key=lambda kv: kv[0].encode("utf-16-be")):
            if isinstance(v, dict):
                lines.append(f"{prefix}{k}:")
                _render_plain(v, indent + 1, lines)
            elif isinstance(v, list):
                if not v:
                    lines.append(f"{prefix}{k}: []")
                elif all(not isinstance(i, (dict, list)) for i in v):
                    lines.append(f"{prefix}{k}:")
                    for item in v:
                        lines.append(f"{prefix}  - {_plain_scalar(item)}")
                else:
                    lines.append(f"{prefix}{k}:")
                    for item in v:
                        if isinstance(item, dict):
                            lines.append(f"{prefix}  -")
                            _render_plain(item, indent + 2, lines)
                        else:
                            lines.append(f"{prefix}  - {_plain_scalar(item)}")
            else:
                lines.append(f"{prefix}{k}: {_format_plain_field(k, v)}")
    else:
        lines.append(f"{prefix}{_plain_scalar(value)}")


def _format_plain_field(key: str, value: Any) -> str:
    lower = key.lower()

    # Secret — always redact
    if lower.endswith("_secret"):
        return "***"

    # Timestamps → RFC 3339
    if lower.endswith("_epoch_ms"):
        if isinstance(value, int) and not isinstance(value, bool):
            return _format_rfc3339_ms(value)
    if lower.endswith("_epoch_s"):
        if isinstance(value, int) and not isinstance(value, bool):
            return _format_rfc3339_ms(value * 1000)
    if lower.endswith("_epoch_ns"):
        if isinstance(value, int) and not isinstance(value, bool):
            return _format_rfc3339_ms(value // 1_000_000)
    if lower.endswith("_rfc3339"):
        return _plain_scalar(value)

    # Size
    if lower.endswith("_bytes"):
        if isinstance(value, int) and not isinstance(value, bool):
            return _format_bytes_human(value)

    # Percentage
    if lower.endswith("_percent"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}%"

    # Currency — Bitcoin
    if lower.endswith("_msats"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}msats"
    if lower.endswith("_sats"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}sats"
    if lower.endswith("_btc"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)} BTC"

    # Currency — Fiat with symbol
    if lower.endswith("_usd_cents"):
        if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            return f"${value // 100}.{value % 100:02d}"
    if lower.endswith("_eur_cents"):
        if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            return f"\u20ac{value // 100}.{value % 100:02d}"
    if lower.endswith("_jpy"):
        if isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            return f"\u00a5{_format_with_commas(value)}"
    # Currency — Generic _{code}_cents
    if lower.endswith("_cents"):
        code = _extract_currency_code(lower)
        if code and isinstance(value, int) and not isinstance(value, bool) and value >= 0:
            return f"{value // 100}.{value % 100:02d} {code.upper()}"

    # Duration — long units
    if lower.endswith("_minutes"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)} minutes"
    if lower.endswith("_hours"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)} hours"
    if lower.endswith("_days"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)} days"

    # Duration — ms
    if lower.endswith("_ms") and not lower.endswith("_epoch_ms"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            if value >= 1000:
                return f"{value / 1000:.2f}s"
            return f"{_plain_scalar(value)}ms"

    # Duration — ns, us, s
    if lower.endswith("_ns") and not lower.endswith("_epoch_ns"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}ns"
    if lower.endswith("_us"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}\u03bcs"
    if lower.endswith("_s") and not lower.endswith("_epoch_s"):
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            return f"{_plain_scalar(value)}s"

    return _plain_scalar(value)


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


# ═══════════════════════════════════════════
# Secret redaction
# ═══════════════════════════════════════════


def redact_secrets(value: Any) -> Any:
    """Walk a dict/list tree and redact any key ending in '_secret'."""
    if isinstance(value, dict):
        for k in value:
            if k.lower().endswith("_secret") and isinstance(value[k], str):
                value[k] = "***"
            redact_secrets(value[k])
    elif isinstance(value, list):
        for item in value:
            redact_secrets(item)
    return value


# ═══════════════════════════════════════════
# AFD Protocol templates
# ═══════════════════════════════════════════


def ok(result: Any) -> dict:
    return {"code": "ok", "result": result}


def ok_trace(result: Any, trace: Any) -> dict:
    return {"code": "ok", "result": result, "trace": trace}


def error(message: str) -> dict:
    return {"code": "error", "error": message}


def error_trace(message: str, trace: Any) -> dict:
    return {"code": "error", "error": message, "trace": trace}


def startup(config: Any, args: Any, env: Any) -> dict:
    return {"code": "startup", "config": config, "args": args, "env": env}


def status(code: str, fields: dict | None = None) -> dict:
    result = dict(fields) if fields else {}
    result["code"] = code
    return result


# ═══════════════════════════════════════════
# Helpers
# ═══════════════════════════════════════════


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


def _extract_currency_code(key: str) -> str | None:
    without_cents = key.removesuffix("_cents")
    if without_cents == key:
        return None
    idx = without_cents.rfind("_")
    if idx < 0:
        return None
    return without_cents[idx + 1:]
