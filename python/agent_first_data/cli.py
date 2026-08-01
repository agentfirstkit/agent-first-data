"""AFDATA CLI helpers — output format parsing, log filter normalization, error building."""

from __future__ import annotations

import enum
import sys
from typing import Any, Iterator

from agent_first_data.format import (
    Event,
    LogLevel,
    OutputOptions,
    json_error,
    json_log,
    json_progress,
    json_result,
    _format_json,
    _format_yaml,
    _format_plain,
    validate_protocol_event,
)


class OutputFormat(enum.Enum):
    """Output format for CLI and pipe/MCP modes."""

    JSON = "json"
    YAML = "yaml"
    PLAIN = "plain"


class OutputTo(enum.Enum):
    """Where a CliEmitter sends its events, selected by ``--output-to``.

    The stream an event lands on follows the program's *consumption mode*, not
    the event's shape (see the spec's CLI Event Framing):

    - ``OutputTo.SPLIT`` (the default) is finite one-shot mode: ``result`` goes
      to stdout, while ``error``/``progress``/``log`` go to stderr. stdout
      therefore carries only successful payloads, so a shell capture or pipe
      never mistakes a failure for data.
    - ``OutputTo.STDOUT`` / ``OutputTo.STDERR`` are event-stream mode: every
      event, including ``error``, is collapsed onto that one stream so a consumer
      reading it in order (branching on ``kind``) sees preserved ordering.
    """

    SPLIT = "split"
    STDOUT = "stdout"
    STDERR = "stderr"

    @classmethod
    def parse(cls, value: str) -> OutputTo:
        """Parse an ``--output-to`` value: ``split`` (default), ``stdout``, or ``stderr``.

        Raises ValueError with a message suitable for build_cli_error on unknown values.

        >>> OutputTo.parse("split")
        <OutputTo.SPLIT: 'split'>
        >>> OutputTo.parse("xml")
        Traceback (most recent call last):
            ...
        ValueError: unsupported --output-to 'xml'; expected split, stdout, or stderr
        """
        try:
            return cls(value)
        except ValueError:
            raise ValueError(
                f"unsupported --output-to {value!r}; expected split, stdout, or stderr"
            )


class LogFilters:
    """Log event filter matcher."""

    def __init__(self, filters: list[str]) -> None:
        """Initialize with a normalized filter list."""
        self._filters = filters

    def enabled(self, event: str) -> bool:
        """Check if event should be logged.

        An empty filter list returns False (filtering is opt-in). The single
        wildcard word 'all' returns True ('*' is not special — one wildcard
        spelling, not two). Otherwise returns True iff event (lowercased) starts
        with any filter (prefix match); a mistyped filter simply matches nothing
        and silently emits no output.
        """
        if not self._filters:
            return False
        if "all" in self._filters:
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


def render(value: Any, format: OutputFormat, *, options: OutputOptions | None = None) -> str:
    """Render a value as JSON, YAML, or plain (logfmt) text for OutputFormat.

    The single public render entry point: value x format x options -> str.
    JSON ignores PlainStyle and preserves original keys and values after redaction.

    >>> import json
    >>> v = {"code": "ok"}
    >>> render(v, OutputFormat.JSON).startswith('{"code"')
    True
    """
    if format is OutputFormat.YAML:
        return _format_yaml(value, options=options)
    if format is OutputFormat.PLAIN:
        return _format_plain(value, options=options)
    return _format_json(value, options=options)


class CliEmitter:
    """Stateful emitter for structured CLI executions.

    The output format, redaction policy, and stream routing are fixed when the
    emitter is created. Emitting after a terminal event, emitting a repeated
    terminal event, and writer failures all raise.

    Routing follows the consumption mode (:class:`OutputTo`):

    - :meth:`finite` / :meth:`finite_with` — finite one-shot: ``result`` → the
      primary writer (stdout), ``error``/``progress``/``log`` → the diagnostic
      writer (stderr). The recommended default for a one-shot CLI, so shell
      capture and pipelines never treat a failure as data.
    - :meth:`stream` (and the plain ``CliEmitter(writer, ...)`` constructor) —
      event stream: every event, including ``error``, goes to the single writer,
      preserving interleaved ordering.
    - :meth:`from_output_to` builds either shape from a parsed :class:`OutputTo`
      selector.
    """

    def __init__(
        self,
        writer: Any,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
        *,
        diagnostic: Any | None = None,
        strict_protocol: bool = False,
    ) -> None:
        """Create an event-stream emitter: every event goes to ``writer``.

        This is the unified/stream form (alias :meth:`stream`). Pass
        ``diagnostic`` — or use :meth:`finite`/:meth:`finite_with` — for finite
        one-shot mode, which routes ``result`` to ``writer`` and every diagnostic
        (``error``/``progress``/``log``) to ``diagnostic``.
        """
        self._writer = writer
        self._diagnostic = diagnostic
        self._format = format
        self._output_options = output_options or OutputOptions()
        self._strict_protocol = strict_protocol
        self._terminal_emitted = False

    def with_strict_protocol(self) -> CliEmitter:
        """Opt this emitter into the AFDATA recommended strict profile."""
        self._strict_protocol = True
        return self

    @classmethod
    def stream(
        cls,
        writer: Any,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
    ) -> CliEmitter:
        """Create an event-stream emitter: every event, including ``error``, goes
        to the single ``writer``, preserving interleaved ordering. Pick this when
        the consumer reads one ordered stream and branches on ``kind``.
        """
        return cls(writer, format, output_options)

    @classmethod
    def finite_with(
        cls,
        result_writer: Any,
        diagnostic: Any,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
    ) -> CliEmitter:
        """Create a finite one-shot emitter with explicit sinks: ``result`` goes
        to ``result_writer``, while ``error``/``progress``/``log`` go to
        ``diagnostic``.
        """
        return cls(result_writer, format, output_options, diagnostic=diagnostic)

    @classmethod
    def finite(
        cls,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
    ) -> CliEmitter:
        """Create a finite one-shot emitter wired to the process streams:
        ``result`` → ``sys.stdout``, ``error``/``progress``/``log`` →
        ``sys.stderr``. The recommended default for a one-shot CLI.
        """
        return cls.finite_with(sys.stdout, sys.stderr, format, output_options)

    @classmethod
    def from_output_to(
        cls,
        selector: OutputTo,
        format: OutputFormat,
        output_options: OutputOptions | None = None,
    ) -> CliEmitter:
        """Build an emitter from a parsed :class:`OutputTo` selector, wired to the
        process streams: ``SPLIT`` is finite mode (``result`` → stdout,
        everything else → stderr); ``STDOUT``/``STDERR`` are event-stream mode
        onto that one stream.
        """
        if selector is OutputTo.SPLIT:
            return cls.finite_with(sys.stdout, sys.stderr, format, output_options)
        if selector is OutputTo.STDOUT:
            return cls.stream(sys.stdout, format, output_options)
        if selector is OutputTo.STDERR:
            return cls.stream(sys.stderr, format, output_options)
        raise ValueError(f"unsupported OutputTo selector: {selector!r}")

    def emit(self, event: Event | dict) -> None:
        """Emit a typed Event or a dict (for compatibility)."""
        if isinstance(event, Event):
            envelope = event.to_dict()
        else:
            envelope = event

        validate_protocol_event(envelope, strict=self._strict_protocol)
        kind = envelope["kind"]
        if kind in ("log", "progress"):
            if self._terminal_emitted:
                raise RuntimeError("cannot emit non-terminal event after terminal event")
        elif kind in ("result", "error"):
            if self._terminal_emitted:
                raise RuntimeError("cannot emit duplicate terminal event")
        else:
            raise ValueError(f"unsupported event kind {kind!r}")

        # Finite mode (a diagnostic sink is present) splits by kind: `result`
        # stays on the primary writer (stdout), while `error`/`progress`/`log`
        # are diagnostics routed to the diagnostic writer (stderr). Event-stream
        # mode (no diagnostic sink) keeps every event on the single writer.
        use_diagnostic = kind != "result" and self._diagnostic is not None
        sink = self._diagnostic if use_diagnostic else self._writer
        sink.write(render(envelope, self._format, options=self._output_options) + "\n")
        flush = getattr(sink, "flush", None)
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

    def finish(self, event: Event | dict, success_code: int) -> int:
        """Emit ``event`` as the terminal event and resolve the outcome to a
        process exit code, so a one-shot CLI need not hand-roll the
        emit-then-exit dance.

        Returns ``success_code`` on a successful write; ``0`` if the write failed
        because the reader hung up (broken pipe); ``4`` on any other emit,
        validation, or write failure. A library never calls :func:`sys.exit`
        itself — return this code from the program's entry point.
        """
        try:
            self.emit(event)
        except BrokenPipeError:
            return 0
        except Exception:
            return 4
        return success_code

    def finish_result(self, payload: Any) -> int:
        """Convenience over :meth:`finish`: emit a ``result`` payload and return
        ``0`` on success.

        Errors have no matching convenience: build the event through the error
        builder — ``json_error(code, message).hint_if_some(hint)...build()``, which
        also carries ``retryable`` and extra fields — and pass it to
        :meth:`finish` with the desired exit code."""
        return self.finish(json_result(payload).build(), 0)



def build_cli_version(
    name: str,
    display_name: str | None,
    version: str,
    build: str | None,
) -> dict:
    """Build a standard CLI version result event.

    The result payload always carries ``code: "version"`` alongside ``name`` and
    ``version``, so structured consumers can dispatch on ``code`` the same way
    they would for any other result shape. ``name`` is the short/bin identity
    (e.g. ``"afdata"``); ``display_name`` is an optional human-facing product
    name (e.g. ``"Agent-First Data"``); ``build`` is an opaque caller-supplied
    identifier (a git commit SHA, for example) — its meaning is entirely up to
    the caller. Both ``display_name`` and ``build`` are ``None`` when
    unavailable, and simply absent from the payload.
    """
    payload: dict[str, Any] = {"code": "version", "name": name, "version": version}
    if display_name is not None:
        payload["display_name"] = display_name
    if build is not None:
        payload["build"] = build
    return json_result(payload).build().to_dict()


def cli_render_version(
    name: str,
    display_name: str | None,
    version: str,
    build: str | None,
    format: OutputFormat,
) -> str:
    """Render a CLI version response as a protocol-v1 event in ``format``.

    There is no conventional ``"<name> <version>"`` path: ``--version`` always
    answers with the structured event (see :func:`build_cli_version`).
    """
    rendered = render(build_cli_version(name, display_name, version, build), format)
    return rendered.rstrip("\n") + "\n"



def build_cli_error(message: str, hint: str | None = None) -> dict | Event:
    """Build a standard CLI parse error event.

    Use when argument parsing fails or a flag value is invalid.
    Print with render(..., OutputFormat.JSON) and exit with code 2.

    Always returns a strict-valid ``kind: "error"`` event with code
    ``"cli_error"``. Never raises: an empty ``message`` is replaced with a
    generic placeholder so the returned event stays strict-valid.

    Returns the event envelope as a dict.
    """
    if not message:
        message = "unspecified error"
    event = json_error("cli_error", message)
    if hint is not None:
        event = event.hint(hint)
    return event.build().to_dict()
