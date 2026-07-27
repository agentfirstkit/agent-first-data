<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# Naming, redaction, and output

Read this reference for field naming, suffix behavior, redaction, rendering,
environment variables, configs, and database columns. For exact machine
metadata, also inspect `registry.json`.

## Contents

- [Naming rule](#naming-rule)
- [Value constraints](#value-constraints)
- [Secret and URL redaction](#secret-and-url-redaction)
- [Rendering contract](#rendering-contract)
- [Other naming surfaces](#other-naming-surfaces)
- [Boundary checklist](#boundary-checklist)

## Naming rule

The field name is part of the schema. Encode a unit or strict format when the
raw JSON type does not make it unambiguous.

| Meaning | Suffixes |
|---|---|
| Duration | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` |
| Instant | `_epoch_s`, `_epoch_ms`, `_epoch_ns`, `_rfc3339` |
| Strict string | `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` |
| Size | `_bytes` |
| Percentage | `_percent` |
| Bitcoin | `_sats`, `_msats` |
| Fiat | `_usd_cents`, `_eur_cents`, `_{code}_cents`, `_{code}_micro`, `_jpy` |
| Sensitive value/subtree | `_secret` |
| Whole URL requiring internal scrubbing | `_url` |

Use no suffix when the field is already unambiguous, such as `retry_count`,
`method`, `model`, or a native `TIMESTAMPTZ` database column. URL fields are the
exception: use `_url` so embedded credentials can be scrubbed.

## Value constraints

- Duration, percentage, and Bitcoin fields are numbers. Large `_sats` or
  `_msats` values outside JSON's safe-integer range are decimal strings.
- `_epoch_s` and `_epoch_ms` are integers. `_epoch_ns` is a decimal string to
  avoid precision loss beyond 2^53-1.
- `_rfc3339` includes `Z` or an explicit `±HH:MM` offset.
- `_rfc3339_date` is `YYYY-MM-DD`; `_rfc3339_time` is
  `HH:MM:SS[.fraction]` with no timezone.
- `_utc_offset` is `UTC` or `±HH:MM`, not an IANA timezone name.
- `_bcp47` uses hyphen-separated language subtags such as `zh-CN`, not
  `zh_CN`.
- `_bytes` is a non-negative integer. Do not write unit-bearing strings such as
  `"10MiB"`.
- `_percent` is in percent units: `85` means 85%, not 0.85.
- Fiat suffixes carry integer minor units. Use `_{code}_micro` for millionths;
  never use floating cents.
- `null` means absent/unset and is exempt from suffix type checks.

Do not put magic values such as `"auto"` in strict-format fields. Define such
behavior in the owning tool's config semantics.

## Secret and URL redaction

`_secret` and `_SECRET` redact the entire value or subtree to `***`; mixed case
does not match. For legacy names that cannot change, configure exact
`secret_names` at the output boundary. Matching does not trim, case-fold,
normalize, glob, regex-match, or substring-match.

`***` means AFDATA redacted a secret. Do not reuse it for truncation,
serialization failure, or guessed sensitivity.

`_url` scrubs:

- the password in URL userinfo; and
- query parameters whose names end in `_secret` or match configured exact
  secret names.

It does not automatically scrub generic `access_token`, `api_key`, `code`, or
`sig` parameters. Rename parameters you control or configure their exact names.
A schemeless credential-looking or internally whitespace-containing URL fails
closed to `***`.

AFDATA never scans arbitrary prose. Before interpolating a URL into a message,
call `redact_url_secrets`.

A CLI that records its own invocation (startup diagnostics, audit trail, crash
report) must pass argv through `redact_argv` first, or a `--*-secret` flag's
value lands in the log verbatim. It redacts by flag name, not by value shape:
free text and positionals are left alone.

## Rendering contract

Use one `render(value, format, options)` boundary:

| Format | Contract |
|---|---|
| JSON | One line, original keys/types/order, redacted |
| YAML | Multi-line, original keys/types, JCS/UTF-16 key order, redacted |
| Plain | One-line logfmt, lossy suffix formatting, redacted |

Plain `Readable` strips recognized formatting suffixes and formats values:
durations gain units, epochs become RFC 3339, bytes become binary human sizes,
percent gets `%`, and currencies become readable amounts. Plain `Raw` preserves
keys and raw scalar values while still redacting. JSON and YAML never strip or
format suffix values.

If two Plain keys collide after suffix stripping, keep both original keys and
raw values. Redaction happens first, so collision fallback never restores a
secret.

Plain nested keys use dot paths. Quote/escape whitespace, `=`, `"`, and `\` so
one record remains one physical line. Arrays are comma-joined and null renders
as an empty value.

## Other naming surfaces

- Environment variables use the same suffixes in `UPPER_SNAKE_CASE`, for
  example `CACHE_TTL_S` and `DATABASE_URL_SECRET`.
- Config keys use the same suffix and type rules as emitted data.
- Generic database types retain suffixes (`duration_ms INTEGER`,
  `api_key_secret TEXT`). Native semantic types do not need redundant suffixes.
- ORM fields preserve the database/wire name; do not silently shorten
  `duration_ms` to `duration`.

## Boundary checklist

1. Confirm every ambiguous number carries a unit.
2. Confirm strict strings carry the correct format suffix.
3. Confirm every secret-bearing path is named or explicitly configured.
4. Apply redaction before every serializer, including HTTP/MCP/SSE.
5. Keep JSON/YAML schema-preserving.
6. Run `afdata lint` against representative serialized values.
