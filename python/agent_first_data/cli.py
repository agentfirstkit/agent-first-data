"""AFDATA CLI helpers — output format parsing, log filter normalization, error building."""

from __future__ import annotations

import enum
from typing import Any, Callable, Mapping, Iterator

from agent_first_data.format import (
    Event,
    LogLevel,
    OutputOptions,
    json_error,
    json_log,
    json_progress,
    json_result,
    output_json,
    output_yaml,
    output_plain,
    validate_protocol_event,
)


class OutputFormat(enum.Enum):
    """Output format for CLI and pipe/MCP modes."""

    JSON = "json"
    YAML = "yaml"
    PLAIN = "plain"


class LogFilters:
    """Log event filter matcher."""

    def __init__(self, filters: list[str]) -> None:
        """Initialize with a normalized filter list."""
        self._filters = filters

    def enabled(self, event: str) -> bool:
        """Check if event should be logged.

        Empty filter list returns False.
        Filters containing 'all' or '*' return True.
        Otherwise returns True if event (lowercased) starts with any filter.
        """
        if not self._filters:
            return False
        if any(f in ("all", "*") for f in self._filters):
            return True
        event_lower = event.lower()
        return any(event_lower.startswith(f) for f in self._filters)

    def __bool__(self) -> bool:
        """True if the filter list is non-empty."""
        return bool(self._filters)

    def __iter__(self) -> Iterator[str]:
        """Iterate over filter entries."""
        return iter(self._filters)

    def __len__(self) -> int:
        """Return the number of filters."""
        return len(self._filters)

    def append(self, item: str) -> None:
        """Append a filter entry."""
        self._filters.append(item)


def cli_parse_output(s: str) -> OutputFormat:
    """Parse the --output flag value into an OutputFormat.

    Raises ValueError with a message suitable for build_cli_error on unknown values.

    >>> cli_parse_output("json")
    <OutputFormat.JSON: 'json'>
    >>> cli_parse_output("xml")
    Traceback (most recent call last):
        ...
    ValueError: invalid --output format 'xml': expected json, yaml, or plain
    """
    try:
        return OutputFormat(s)
    except ValueError:
        raise ValueError(
            f"invalid --output format {s!r}: expected json, yaml, or plain"
        )


def cli_parse_log_filters(entries: list[str]) -> LogFilters:
    """Normalize --log flag entries and return a LogFilters matcher.

    Trims, lowercases, deduplicates, and removes empty entries.
    Accepts pre-split entries (e.g. after splitting on comma).

    >>> filters = cli_parse_log_filters(["Query", " error ", "query"])
    >>> list(filters)
    ['query', 'error']
    """
    out: list[str] = []
    for entry in entries:
        s = entry.strip().lower()
        if s and s not in out:
            out.append(s)
    return LogFilters(out)


def cli_output(value: Any, format: OutputFormat, *, options: OutputOptions | None = None) -> str:
    """Dispatch output formatting by OutputFormat.

    Equivalent to calling output_json, output_yaml, or output_plain directly.
    JSON ignores OutputStyle and preserves original keys and values after redaction.

    >>> import json
    >>> v = {"code": "ok"}
    >>> cli_output(v, OutputFormat.JSON).startswith('{"code"')
    True
    """
    if format is OutputFormat.YAML:
        return output_yaml(value, options=options)
    if format is OutputFormat.PLAIN:
        return output_plain(value, options=options)
    return output_json(value, options=options)


class CliEmitter:
    """Stateful emitter for finite structured CLI executions."""

    def __init__(
        self,
        writer: Any,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
        log_fields: Callable[[], Mapping[str, Any]] | None = None,
    ) -> None:
        self._writer = writer
        self._format = format
        self._output_options = output_options or OutputOptions()
        self._terminal_emitted = False
        self._log_fields_provider = log_fields

    def with_log_fields(self, provider: Callable[[], Mapping[str, Any]]) -> CliEmitter:
        """Set a provider callable for default log event fields.

        Returns self for chaining. The provider is called for each log event
        and its fields are merged (with explicit fields taking precedence).
        """
        self._log_fields_provider = provider
        return self

    def emit(self, event: Event | dict) -> None:
        """Emit a typed Event or a dict (for compatibility)."""
        if isinstance(event, Event):
            envelope = event.to_dict()
        else:
            envelope = event

        validate_protocol_event(envelope, strict=False)
        kind = envelope["kind"]
        if kind in ("log", "progress"):
            if self._terminal_emitted:
                raise RuntimeError("cannot emit non-terminal event after terminal event")
        elif kind in ("result", "error"):
            if self._terminal_emitted:
                raise RuntimeError("cannot emit duplicate terminal event")
        else:
            raise ValueError(f"unsupported event kind {kind!r}")

        # Apply log fields provider if this is a log event
        if kind == "log" and self._log_fields_provider is not None:
            provider_fields = self._log_fields_provider()
            log_payload = envelope.get("log")
            if provider_fields and isinstance(log_payload, dict):
                # Merge provider fields, explicit fields take precedence
                merged_log = dict(provider_fields)
                merged_log.update(log_payload)
                envelope["log"] = merged_log

        self._writer.write(cli_output(envelope, self._format, options=self._output_options) + "\n")
        flush = getattr(self._writer, "flush", None)
        if flush is not None:
            flush()
        if kind in ("result", "error"):
            self._terminal_emitted = True

    def emit_validated_value(self, value: Any) -> None:
        """Emit a dynamic JSON value after strict validation."""
        try:
            validate_protocol_event(value, strict=True)
        except ValueError as e:
            raise ValueError(f"emit_validated_value failed validation: {e}") from e
        self.emit(value)

    def emit_result(self, result: Any) -> None:
        """Emit a result event from a payload."""
        event = json_result(result).build()
        self.emit(event)

    def emit_error(self, code: str, message: str) -> None:
        """Emit an error event with retryable defaulting to False."""
        event = json_error(code, message).build()
        self.emit(event)

    def emit_progress(self, message: str) -> None:
        """Emit a progress event."""
        if not message or not isinstance(message, str):
            raise ValueError("message must be a non-empty string")
        # Use the builder to get default trace: {}
        event = json_progress({"message": message}).build()
        self.emit(event)

    def emit_log(self, level: LogLevel | str, message: str) -> None:
        """Emit a log event."""
        if isinstance(level, str):
            level = LogLevel(level)
        if not message or not isinstance(message, str):
            raise ValueError("message must be a non-empty string")
        event = json_log({"level": level.value, "message": message}).build()
        self.emit(event)


def build_cli_version(version: str) -> dict:
    """Build a standard CLI version value."""
    return json_result({"version": version}).build().to_dict()


def cli_render_version(
    name: str,
    version: str,
    format: OutputFormat | None = None,
) -> str:
    """Render CLI version output.

    Pass an OutputFormat for AFDATA JSON/YAML/plain. Pass None to preserve
    conventional "<name> <version>" text.
    """
    rendered = (
        f"{name} {version}" if format is None else cli_output(build_cli_version(version), format)
    )
    return rendered.rstrip("\n") + "\n"


def cli_handle_version_or_continue(
    raw_args: list[str],
    name: str,
    version: str,
    default_output: OutputFormat | None = None,
    output_flag: str = "--output",
    allow_output_format: bool = True,
) -> str | None:
    """Render version output if --version/-V is present; otherwise return None.

    Raises ValueError for malformed version requests, for example
    ``--version --output xml``. The caller should convert that to a CLI error
    with ``build_cli_error``.
    """
    version_requested = False
    output_format: OutputFormat | None = None
    output_error: ValueError | None = None

    i = 0
    while i < len(raw_args):
        arg = raw_args[i]
        if arg == "--":
            break
        if arg in ("--version", "-V"):
            version_requested = True
            i += 1
            continue
        if allow_output_format and arg == "--json":
            if output_format is not None and output_format is not OutputFormat.JSON:
                output_error = ValueError(
                    "conflicting output formats: --json conflicts with previous output format"
                )
            else:
                output_format = OutputFormat.JSON
            i += 1
            continue
        if allow_output_format and (arg == output_flag or arg.startswith(f"{output_flag}=")):
            value: str | None
            if arg.startswith(f"{output_flag}="):
                value = arg.split("=", 1)[1]
                step = 1
            elif i + 1 < len(raw_args) and not raw_args[i + 1].startswith("-"):
                value = raw_args[i + 1]
                step = 2
            else:
                value = None
                step = 1
            if value is None:
                output_error = ValueError(
                    f"missing value for {output_flag}: expected json, yaml, or plain"
                )
            else:
                try:
                    parsed_output = cli_parse_output(value)
                    if output_format is not None and output_format is not parsed_output:
                        output_error = ValueError(
                            f"conflicting output formats: {output_flag} {value} conflicts with previous output format"
                        )
                    else:
                        output_format = parsed_output
                except ValueError as e:
                    output_error = e
            i += step
            continue
        i += 1

    if not version_requested:
        return None
    if output_error is not None:
        raise output_error
    return cli_render_version(
        name,
        version,
        output_format if allow_output_format and output_format is not None else default_output,
    )


def build_cli_error(message: str, hint: str | None = None) -> dict | Event:
    """Build a standard CLI parse error event.

    Use when argument parsing fails or a flag value is invalid.
    Print with output_json and exit with code 2.

    Returns the event envelope as a dict.
    """
    event = json_error("cli_error", message)
    if hint is not None:
        event = event.hint(hint)
    return event.build().to_dict()
