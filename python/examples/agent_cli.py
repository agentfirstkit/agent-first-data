"""Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.

Demonstrates: output-aware --help (one-level) plus orthogonal --recursive scope
and --output plain|json|yaml|markdown format for full surface export, cli_parse_output,
cli_parse_log_filters, render, build_cli_error, --dry-run, and error hints.

Run:  PYTHONPATH=. python3 examples/agent_cli.py --help
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive --output json
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive --output markdown
      PYTHONPATH=. python3 examples/agent_cli.py --version --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --help
      PYTHONPATH=. python3 examples/agent_cli.py echo --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --dry-run --output yaml
      PYTHONPATH=. python3 examples/agent_cli.py ping --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --output yaml --log startup,request
      PYTHONPATH=. python3 examples/agent_cli.py --log all ping   # or --verbose
Test: PYTHONPATH=. python3 -m pytest examples/agent_cli.py -v
"""

import argparse
import json
import os
import signal
import sys

from agent_first_data import (
    CliEmitter,
    LogFilters,
    OutputFormat,
    LogLevel,
    build_cli_error,
    json_error,
    json_log,
    json_result,
    render,
    cli_handle_version_or_continue,
    cli_parse_log_filters,
    cli_parse_output,
    redact_argv,
)

AGENT_CLI_VERSION = "0.13.0"
HELP_DEFAULT_API_KEY_SECRET = "sk-help-default"
PING_HOST_ENV = "PING_HOST"


class ArgumentParserError(ValueError):
    pass


class StrictArgumentParser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ArgumentParserError(message)

    def exit(self, status: int = 0, message: str | None = None) -> None:
        if status:
            raise ArgumentParserError((message or "").strip())
        raise SystemExit(status)


def build_parser() -> argparse.ArgumentParser:
    parser = StrictArgumentParser(
        prog="agent-cli",
        description="Minimal agent-first CLI example",
        add_help=False,  # we handle --help ourselves
    )
    parser.add_argument("--help", action="store_true", help="Show this help (one-level)")
    parser.add_argument("--version", action="store_true", help="Print version")
    parser.add_argument("--recursive", action="store_true", help="With --help, expand the full command tree (a bare --recursive is ignored)")
    parser.add_argument("--output", default="json", help="Output format: json, yaml, plain; help also accepts markdown")
    parser.add_argument("--log", default="", help="Log categories (comma-separated); --log all (or --verbose) enables every category")
    parser.add_argument("--verbose", action="store_true", help="Enable all log categories (shorthand for --log all)")
    parser.add_argument("--api-key-secret", default=HELP_DEFAULT_API_KEY_SECRET, help=f"API key used by examples (default: {redact_help_default('--api-key-secret', HELP_DEFAULT_API_KEY_SECRET)})")

    subs = parser.add_subparsers(dest="command", parser_class=StrictArgumentParser)

    echo_p = subs.add_parser("echo", add_help=False, help="Echo back the input as structured output")
    echo_p.add_argument("--help", action="store_true", help="Show help for echo")
    echo_p.add_argument("--dry-run", action="store_true", help="Preview without executing")

    ping_p = subs.add_parser("ping", add_help=False, help="Ping a remote target")
    ping_p.add_argument("--help", action="store_true", help="Show help for ping")
    ping_p.add_argument("--host", help="Target host to ping")

    cancel_p = subs.add_parser("cancel", add_help=False, help="Return a tool-defined cancellation error")
    cancel_p.add_argument("--help", action="store_true", help="Show help for cancel")

    return parser


def leaf_global_options_note() -> str:
    """Note appended to a leaf --help target so it still advertises the global
    --output formats. Only added for the help *target*, never for descendants in
    a recursive dump (the root already documented the modifiers once)."""
    return (
        "\nGlobal options:\n"
        "  --output <FORMAT>  Output format: json, yaml, plain; help also accepts markdown\n"
    )


def format_complete_help(parser: argparse.ArgumentParser) -> str:
    """Format help for a parser and all its subcommands recursively."""
    lines = [parser.format_help()]
    # Walk subparsers
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            for name, sub in action.choices.items():
                lines.append(f"\n{'=' * 60}")
                lines.append(f"{parser.prog} {name}")
                lines.append("=" * 60)
                lines.append(sub.format_help())
    return "\n".join(lines) + "\n"


def subcommand_about(parser: argparse.ArgumentParser, name: str) -> str:
    """Return the one-line `about` (the add_parser help=) for a subcommand."""
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            for choice in action._choices_actions:
                if choice.dest == name:
                    return choice.help or ""
    return ""


def help_without_description(parser: argparse.ArgumentParser) -> str:
    """argparse's format_help() with the description omitted. The Markdown
    heading already carries the about, so the fenced block must not repeat it."""
    saved = parser.description
    parser.description = None
    try:
        return parser.format_help()
    finally:
        parser.description = saved


def markdown_heading(prefix: str, prog: str, command: str | None, about: str) -> str:
    """`# prog [command] - about`, dropping the `- about` tail when empty."""
    title = f"{prog} {command}" if command else prog
    return f"{prefix} {title} - {about}" if about else f"{prefix} {title}"


def format_markdown_help(parser: argparse.ArgumentParser, command: str | None, recursive: bool) -> str:
    """Format Markdown docs for the selected command; expand the tree if recursive."""
    sub = find_subparser(parser, command)
    if sub is not None:
        heading = markdown_heading("#", parser.prog, command, subcommand_about(parser, command))
        return f"{heading}\n\n```text\n{sub.format_help()}{leaf_global_options_note()}```\n"

    root_heading = markdown_heading("#", parser.prog, None, parser.description or "")
    lines = [root_heading, "", "```text", help_without_description(parser).rstrip(), "```"]
    if not recursive:
        return "\n".join(lines) + "\n"
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            for name, choice in action.choices.items():
                sub_heading = markdown_heading("##", parser.prog, name, subcommand_about(parser, name))
                lines.extend(["", sub_heading, "", "```text", choice.format_help().rstrip(), "```"])
    return "\n".join(lines) + "\n"


def find_subparser(parser: argparse.ArgumentParser, command: str | None) -> argparse.ArgumentParser | None:
    if not command:
        return None
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            return action.choices.get(command)
    return None


def output_explicit(raw: list[str]) -> bool:
    return "--output" in raw or any(arg.startswith("--output=") for arg in raw)


def output_missing(raw: list[str]) -> bool:
    for index, arg in enumerate(raw):
        if arg == "--output":
            return index + 1 >= len(raw) or raw[index + 1].startswith("-")
        if arg.startswith("--output="):
            return arg.split("=", 1)[1] == ""
    return False


def output_value(raw: list[str], default: str | None = None) -> str | None:
    for arg in raw:
        if arg.startswith("--output="):
            return arg.split("=", 1)[1]
    if "--output" in raw:
        idx = raw.index("--output")
        if idx + 1 < len(raw) and not raw[idx + 1].startswith("-"):
            return raw[idx + 1]
    return default


def parse_cli_args(parser: argparse.ArgumentParser, raw: list[str]):
    args, unknown = parser.parse_known_args(raw)
    if unknown:
        raise ArgumentParserError(f"unexpected argument: {unknown[0]}")
    return args


def cli_error_format_from_raw(raw: list[str]) -> OutputFormat:
    value = output_value(raw, "json")
    if value is None:
        return OutputFormat.JSON
    try:
        return cli_parse_output(value)
    except ValueError:
        return OutputFormat.JSON


def bootstrap_error(fmt: OutputFormat, message: str, hint: str | None = None, exit_code: int = 2) -> int:
    """Resolve a pre-emitter/bootstrap CLI error to a process exit code.

    Bootstrap failures (bad flags, format conflicts) happen before main()'s
    finite emitter exists, so each builds a standard ``cli_error`` event through
    the error builder (:func:`build_cli_error`) and hands it to a fresh
    ``CliEmitter.finite(fmt).finish``: the ``error`` envelope goes to stderr
    (never stdout, so shell capture and pipelines never treat a usage failure as
    data), and finish folds in broken-pipe (``0``) and write-failure (``4``)
    handling. Callers ``sys.exit`` the returned code.
    """
    return CliEmitter.finite(fmt).finish(build_cli_error(message, hint=hint), exit_code)


def help_requested(raw: list[str]) -> bool:
    return "--help" in raw


def recursive_requested(raw: list[str]) -> bool:
    # A help *modifier*: only consulted when --help is present, so a bare
    # --recursive never affects normal command parsing.
    return "--recursive" in raw


def log_enabled(filters: LogFilters, category: str) -> bool:
    """Delegate to the library matcher.

    `LogFilters.enabled` owns the semantics — opt-in when empty, `all` as the
    single wildcard word (what --verbose expands to), and prefix matching.
    Reimplementing it here would silently drift from the other three SDKs.
    """
    return filters.enabled(category)


def build_request_log(command: str | None) -> dict:
    return json_log({"level": "info", "message": "request", "category": "request", "command": command or "none"}).build().to_dict()


def build_startup_log(raw: list[str], args, log: LogFilters) -> dict:
    return json_log({
        "level": "info",
        "message": "startup",
        "category": "startup",
        "event": "startup",
        "argv": redact_argv(raw),
        "parsed": {
            "command": args.command or "none",
            "output": args.output,
            "log": list(log),
            "verbose": args.verbose,
        },
        "effective_config": {
            "output": args.output,
            "log": list(log),
        },
        "env": startup_env_snapshot(),
    }).build().to_dict()


def startup_env_snapshot() -> list[dict]:
    snapshot = []
    for key in (PING_HOST_ENV,):
        item = {"key": key, "present": key in os.environ}
        if key in os.environ:
            item["value"] = os.environ[key]
        snapshot.append(item)
    return snapshot


def redact_help_default(name: str, value: str) -> str:
    normalized = name.lstrip("-").replace("-", "_")
    if normalized.endswith("_secret") or normalized.endswith("_SECRET"):
        return "***"
    return value


def compact_usage(parser: argparse.ArgumentParser) -> str | None:
    """Return compact usage relative to parser.prog; command_path carries the prefix.

    Built from the parser's own actions rather than scraped from
    ``format_usage()``: argparse wraps its usage line at terminal width and pads
    the continuations, which would put newlines and column alignment inside a
    single-line schema field. Every option is already described in ``arguments``,
    so usage collapses them to ``[OPTIONS]`` — the same compact shape the Rust,
    Go, and TypeScript examples emit.
    """
    parts: list[str] = []
    if any(
        action.option_strings and action.help is not argparse.SUPPRESS
        for action in parser._actions
    ):
        parts.append("[OPTIONS]")
    for action in parser._actions:
        if action.option_strings:
            continue
        if isinstance(action, argparse._SubParsersAction):
            parts.append("<COMMAND>")
            continue
        name = action.metavar or action.dest.upper()
        parts.append(f"<{name}>" if action.required else f"[{name}]")
    return " ".join(parts) or None


def argument_schema(action: argparse.Action) -> dict | None:
    if isinstance(action, argparse._SubParsersAction):
        return None
    option_strings = action.option_strings
    if option_strings:
        long_name = next((name for name in option_strings if name.startswith("--")), None)
        name = long_name or option_strings[0]
    else:
        name = action.metavar or action.dest.upper()

    out: dict = {"name": name}
    short = next(
        (option for option in option_strings if option.startswith("-") and not option.startswith("--")),
        None,
    )
    if short is not None and short != name:
        out["short"] = short
    if action.help and action.help is not argparse.SUPPRESS:
        out["help"] = action.help
    if action.required:
        out["required"] = True
    if isinstance(action, (argparse._AppendAction, argparse._CountAction)):
        out["repeatable"] = True

    takes_value = option_strings and action.nargs != 0
    if takes_value:
        out["value"] = action.metavar or action.dest.upper()

    default = action.default
    if (
        default is not None
        and default is not False
        and default != ""
        and default != argparse.SUPPRESS
    ):
        out["default"] = (
            "***" if action.dest.endswith("_secret") else str(default)
        )
    return out


def command_arguments(
    parser: argparse.ArgumentParser,
    *,
    enrich_target_help: bool = False,
) -> list[dict]:
    arguments = [
        schema
        for action in parser._actions
        if (schema := argument_schema(action)) is not None
    ]
    if enrich_target_help:
        for argument in arguments:
            if argument["name"] == "--help":
                argument["help"] = (
                    "Show help; add --output plain|json|yaml|markdown "
                    "to choose the format"
                )
    return arguments


def command_schema(
    parser: argparse.ArgumentParser,
    *,
    name: str,
    about: str,
    recursive: bool,
    enrich_target_help: bool = False,
) -> dict:
    out: dict = {"name": name}
    if about:
        out["about"] = about
    if usage := compact_usage(parser):
        out["usage"] = usage
    arguments = command_arguments(
        parser,
        enrich_target_help=enrich_target_help,
    )
    if arguments:
        out["arguments"] = arguments
    if recursive:
        subcommands = subcommand_schemas(parser, recursive=True)
        if subcommands:
            out["subcommands"] = subcommands
    return out


def subcommand_schemas(parser: argparse.ArgumentParser, *, recursive: bool) -> list[dict]:
    commands: list[dict] = []
    if parser._subparsers is None:
        return commands
    for action in parser._subparsers._actions:
        if not isinstance(action, argparse._SubParsersAction):
            continue
        for name, choice in action.choices.items():
            if recursive:
                commands.append(
                    command_schema(
                        choice,
                        name=name,
                        about=subcommand_about(parser, name),
                        recursive=True,
                    )
                )
            else:
                item: dict = {"name": name}
                if about := subcommand_about(parser, name):
                    item["about"] = about
                commands.append(item)
    return commands


def help_schema(parser: argparse.ArgumentParser, command: str | None, scope: str) -> dict:
    sub = find_subparser(parser, command)
    if sub is not None:
        model = command_schema(
            sub,
            name=command,
            about=subcommand_about(parser, command),
            recursive=scope == "recursive",
            enrich_target_help=True,
        )
        return {
            "scope": scope,
            "command_path": f"{parser.prog} {command}",
            **model,
        }
    model = command_schema(
        parser,
        name=parser.prog,
        about=parser.description or "",
        recursive=False,
    )
    model["subcommands"] = subcommand_schemas(
        parser,
        recursive=scope == "recursive",
    )
    return {
        "scope": scope,
        "command_path": parser.prog,
        **model,
    }


def print_help(parser: argparse.ArgumentParser, args, raw: list[str]) -> None:
    explicit = output_explicit(raw)
    value = output_value(raw, args.output)
    sub = find_subparser(parser, args.command)
    # Scope (--recursive) and format (--output) are orthogonal. A specific
    # subcommand is leaf-level here, so its scope is the same either way.
    recursive = recursive_requested(raw)
    scope = "recursive" if recursive else "one_level"

    if output_missing(raw) or (explicit and value is None):
        sys.exit(bootstrap_error(OutputFormat.JSON, "missing value for --output: expected plain, json, yaml, or markdown", hint="valid help output formats: plain, markdown, json, yaml"))

    if value == "plain":
        if sub is not None:
            text = sub.format_help() + leaf_global_options_note()
        elif recursive:
            text = format_complete_help(parser)
        else:
            text = parser.format_help()
        print(text, end="" if text.endswith("\n") else "\n")
        return

    if value == "markdown":
        text = format_markdown_help(parser, args.command, recursive)
        print(text, end="" if text.endswith("\n") else "\n")
        return

    try:
        fmt = cli_parse_output(value)
    except ValueError as e:
        sys.exit(bootstrap_error(OutputFormat.JSON, str(e)))
    event = (
        json_result({"code": "help", "help": help_schema(parser, args.command, scope)})
        .trace({})
        .build()
    )
    print(render(event.to_dict(), fmt))


def main() -> None:
    parser = build_parser()
    raw = sys.argv[1:]

    # --version and structured help are protocol-v1 result envelopes (JSON by
    # default). Explicit plain/markdown help remains raw text.
    try:
        # This example's own value-taking global flags: their space-separated
        # value must not be mistaken for the subcommand boundary that stops the
        # top-level version scan. (--output/--output-to/--version are
        # recognized by the pre-parser itself.)
        version = cli_handle_version_or_continue(
            raw,
            ["--log", "--api-key-secret"],
            "agent-cli",
            "Agent CLI Example",
            AGENT_CLI_VERSION,
            None,
            default_format=OutputFormat.JSON,
        )
    except ValueError as e:
        sys.exit(bootstrap_error(OutputFormat.JSON, str(e), hint="valid version output formats: json, yaml, plain"))
    if version is not None:
        print(version, end="")
        return

    if output_missing(raw):
        if help_requested(raw):
            sys.exit(bootstrap_error(OutputFormat.JSON, "missing value for --output: expected plain, json, yaml, or markdown", hint="valid help output formats: plain, markdown, json, yaml"))
        else:
            sys.exit(bootstrap_error(OutputFormat.JSON, "missing value for --output: expected json, yaml, or plain", hint="valid output formats: json, yaml, plain"))
    try:
        args = parse_cli_args(parser, raw)
    except ArgumentParserError as e:
        sys.exit(bootstrap_error(cli_error_format_from_raw(raw), str(e), hint="try: agent-cli --help"))

    # --help inherits the normal JSON output default; --recursive expands the
    # tree and --output picks the format. A bare --recursive is ignored.
    if args.help:
        print_help(parser, args, raw)
        return

    # Step 1: parse --output with shared helper
    try:
        fmt = cli_parse_output(args.output)
    except ValueError as e:
        sys.exit(bootstrap_error(OutputFormat.JSON, str(e)))

    # A finite one-shot emitter is this example's single output path from here on:
    # `result` → stdout, `error`/`progress`/`log` → stderr (see the spec's CLI
    # Event Framing). stdout therefore carries only successful payloads, so shell
    # capture and pipelines never treat a diagnostic or failure as data. (An
    # agentic, long-running producer would instead pick CliEmitter.from_output_to
    # with --output-to stdout to keep every event on one ordered stream.)
    emitter = CliEmitter.finite(fmt)

    # Step 2: parse --log with shared helper (trim + lowercase + dedup)
    log = cli_parse_log_filters(args.log.split(",") if args.log else [])
    if args.verbose:
        # --verbose is shorthand for --log all.
        log.append("all")

    # Each diagnostic line self-tags with its `category`, so `--log all` reveals
    # the full set from real output rather than a static help list. Logs are
    # diagnostics: the finite emitter routes them to stderr.
    if log_enabled(log, "request"):
        emitter.emit(build_request_log(args.command))
    if log_enabled(log, "startup"):
        emitter.emit(build_startup_log(raw, args, log))

    # Step 3: no subcommand → a cli_error with a hint. Build it through the error
    # builder and hand it to finish (routed to stderr in finite mode).
    if not args.command:
        sys.exit(emitter.finish(build_cli_error("no subcommand provided", hint="try: agent-cli --help"), 2))

    if args.command == "echo":
        # Step 4: --dry-run → preview without executing. The preview carries an
        # explicit trace, so build the event and hand it to finish (finish_result
        # would build a default-trace result instead).
        if args.dry_run:
            preview = json_result({"action": "echo", "log": list(log)}).trace({"duration_ms": 0}).build()
            sys.exit(emitter.finish(preview, 0))

        sys.exit(emitter.finish_result({"action": "echo", "log": list(log)}))

    elif args.command == "ping":
        # Step 5: a rich protocol v1 error (hint + trace) → build event + finish.
        host = args.host or os.environ.get(PING_HOST_ENV)
        if not host:
            err = json_error(
                "ping_target_not_configured",
                "ping target not configured",
            ).hint("set PING_HOST or pass --host").trace({"duration_ms": 0}).build()
            sys.exit(emitter.finish(err, 1))

    elif args.command == "cancel":
        err = json_error(
            "cancelled",
            "operation cancelled",
        ).hint("the operation was cancelled before completion").trace({"duration_ms": 0}).build()
        sys.exit(emitter.finish(err, 1))


# ── Tests (run via: pytest examples/agent_cli.py) ─────────────────────────────


def test_root_help_is_one_level():
    parser = build_parser()
    md = parser.format_help()
    assert "echo" in md, "root --help must include echo subcommand"
    assert "ping" in md, "root --help must include ping subcommand"
    assert "--output" in md, "root --help must include global flags"
    assert "--help-all" not in md, "root --help must not advertise removed recursive flag"
    assert "--dry-run" not in md, "root --help must not include echo's --dry-run"
    assert "--host" not in md, "root --help must not include ping's --host"
    assert "--stream" not in md, "root --help must not include a stream mode flag"
    assert "--result-only" not in md, "root --help must not include a result-only mode flag"


def test_recursive_markdown_export_contains_all_subcommand_details():
    parser = build_parser()
    md = format_markdown_help(parser, None, True)
    assert "# agent-cli" in md, "markdown export must include root heading"
    assert "AFDATA:" not in md, "help must leave version metadata to --version"
    assert "--dry-run" in md, "recursive markdown export must include echo's --dry-run"
    assert "--host" in md, "recursive markdown export must include ping's --host"


def test_one_level_markdown_omits_descendant_details():
    parser = build_parser()
    md = format_markdown_help(parser, None, False)
    assert "# agent-cli" in md, "one-level markdown must include root heading"
    assert "--dry-run" not in md, "one-level markdown must not expand echo's --dry-run"
    assert "--host" not in md, "one-level markdown must not expand ping's --host"


def test_markdown_about_appears_once():
    """The about lives in the heading only, never repeated in the fenced block."""
    parser = build_parser()
    root = format_markdown_help(parser, None, False)
    assert root.count("Minimal agent-first CLI example") == 1, "root about must appear once (heading only)"
    echo = format_markdown_help(parser, "echo", False)
    assert echo.startswith("# agent-cli echo - Echo back the input as structured output"), "subcommand heading must carry the about"
    assert echo.count("Echo back the input as structured output") == 1, "subcommand about must appear once (heading only)"


def test_one_level_help_schema_omits_child_flags():
    parser = build_parser()
    schema = help_schema(parser, None, "one_level")
    assert schema["scope"] == "one_level"
    assert schema["command_path"] == "agent-cli"
    assert not any("arguments" in command for command in schema["subcommands"]), (
        "one-level schema must not expand child arguments"
    )


def test_recursive_requested_is_help_modifier_only():
    # The detector is purely a flag presence check; main only consults it when
    # --help is present, so a bare --recursive never triggers help.
    assert recursive_requested(["--help", "--recursive"]) is True
    assert recursive_requested(["--recursive"]) is True
    assert help_requested(["--recursive"]) is False, (
        "a bare --recursive must not be treated as a help request"
    )


def test_recursive_help_contains_all_subcommand_details():
    parser = build_parser()
    md = format_complete_help(parser)
    assert "echo" in md, "recursive help must include echo subcommand"
    assert "ping" in md, "recursive help must include ping subcommand"
    assert "--output" in md, "recursive help must include global flags"
    assert "--dry-run" in md, "recursive help must include echo's --dry-run"
    assert "--host" in md, "recursive help must include ping's --host"


def test_help_schema_is_recursive_export():
    parser = build_parser()
    schema = help_schema(parser, None, "recursive")
    assert schema["scope"] == "recursive"
    assert schema["command_path"] == "agent-cli"
    assert "code" not in schema
    assert "versions" not in schema
    assert any("arguments" in command for command in schema["subcommands"])


def test_subcommand_help_scoped():
    parser = build_parser()
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            echo_help = action.choices["echo"].format_help()
            assert "--dry-run" in echo_help, "echo --help must include --dry-run"
            assert "--host" not in echo_help, "echo --help must NOT include ping's --host"


def test_leaf_help_target_documents_formats():
    # A leaf --help target (markdown here) must still advertise the --output
    # formats via the global-options note.
    parser = build_parser()
    leaf_md = format_markdown_help(parser, "echo", False)
    assert "--output" in leaf_md, "leaf --help target must document --output"
    assert "markdown" in leaf_md, "leaf --help target must mention the markdown format"
    assert "Global options" in leaf_md


def test_recursive_dumps_do_not_repeat_global_options():
    # Token economy: the modifiers are documented once on the target, never
    # repeated on every descendant block in a recursive dump.
    parser = build_parser()
    assert "Global options" not in format_complete_help(parser), (
        "recursive plain must not repeat the leaf global-options note"
    )
    assert "Global options" not in format_markdown_help(parser, None, True), (
        "recursive markdown must not repeat the leaf global-options note"
    )


def test_help_schema_documents_formats():
    import json

    parser = build_parser()
    root = json.dumps(help_schema(parser, None, "one_level"))
    for token in ("--output", "markdown", "--recursive"):
        assert token in root, f"root help schema must document {token!r}"
    leaf = json.dumps(help_schema(parser, "echo", "one_level"))
    assert "--output" in leaf and "markdown" in leaf, (
        "leaf help schema must document the --output formats"
    )


def security_help_default_case():
    fixture_path = os.path.join(
        os.path.dirname(__file__),
        "..",
        "..",
        "spec",
        "fixtures",
        "security.json",
    )
    with open(fixture_path, encoding="utf-8") as fixture_file:
        return json.load(fixture_file)["help_default_cases"][0]


def test_help_redacts_secret_defaults_in_every_format():
    help_case = security_help_default_case()
    secret_default = help_case["default"]
    redaction_marker = help_case["expected"]
    assert secret_default == HELP_DEFAULT_API_KEY_SECRET
    assert redaction_marker == "***"
    parser = build_parser()
    for rendered in (
        parser.format_help(),
        format_markdown_help(parser, None, False),
        render(help_schema(parser, None, "one_level"), OutputFormat.JSON),
        render(help_schema(parser, None, "one_level"), OutputFormat.YAML),
    ):
        assert redaction_marker in rendered
        assert secret_default not in rendered


def test_parse_output_all_variants():
    assert cli_parse_output("json") is OutputFormat.JSON
    assert cli_parse_output("yaml") is OutputFormat.YAML
    assert cli_parse_output("plain") is OutputFormat.PLAIN
    import pytest
    with pytest.raises(ValueError):
        cli_parse_output("xml")


def test_output_missing_detection():
    for raw in (["--output"], ["--output", "--recursive"], ["--output="]):
        assert output_missing(raw), f"{raw} must be treated as missing --output value"
    for raw in (["--output", "json"], ["--output=json"]):
        assert not output_missing(raw), f"{raw} must have a valid output value"


def test_parse_cli_args_is_strict():
    parser = build_parser()
    assert parse_cli_args(parser, ["echo"]).command == "echo"
    assert parse_cli_args(parser, ["ping", "--host", "example.com"]).command == "ping"
    import pytest

    for raw in (
        ["--bogus", "echo"],
        ["--log"],
        ["echo", "--host", "example.com"],
        ["echo", "extra"],
        ["missing"],
    ):
        with pytest.raises(ArgumentParserError):
            parse_cli_args(parser, list(raw))


def test_parse_log_normalizes():
    assert list(cli_parse_log_filters(["Startup", " REQUEST ", "startup"])) == ["startup", "request"]


def test_log_enabled_wildcards():
    assert not log_enabled(cli_parse_log_filters([]), "startup")
    assert log_enabled(cli_parse_log_filters(["startup"]), "startup")
    assert not log_enabled(cli_parse_log_filters(["startup"]), "request")
    # Prefix matching is the library's contract, shared by all four SDKs.
    assert log_enabled(cli_parse_log_filters(["start"]), "startup")
    # `all` is the single wildcard word; `*` is not special.
    assert log_enabled(cli_parse_log_filters(["all"]), "startup")
    assert log_enabled(cli_parse_log_filters(["all"]), "request")
    assert not log_enabled(cli_parse_log_filters(["*"]), "request")


def test_log_lines_are_category_tagged():
    req = build_request_log(None)
    assert req["kind"] == "log"
    assert req["log"]["category"] == "request"
    assert req["log"]["command"] == "none"
    parser = build_parser()
    args, _ = parser.parse_known_args(["--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"])
    start = build_startup_log(["--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"], args, cli_parse_log_filters(["startup"]))
    assert start["kind"] == "log"
    assert start["log"]["category"] == "startup"
    assert start["log"]["argv"] == ["--output", "yaml", "--log", "startup", "--api-key-secret", "***", "ping"]
    assert start["log"]["parsed"] == {
        "command": "ping",
        "output": "yaml",
        "log": ["startup"],
        "verbose": False,
    }
    assert start["log"]["effective_config"] == {"output": "yaml", "log": ["startup"]}
    env = start["log"]["env"]
    assert env == [{"key": PING_HOST_ENV, "present": PING_HOST_ENV in os.environ, **({"value": os.environ[PING_HOST_ENV]} if PING_HOST_ENV in os.environ else {})}]


def test_build_cli_error_structure():
    v = build_cli_error("--output: invalid value 'xml'")
    assert v["kind"] == "error"
    assert v["error"]["code"] == "cli_error"
    assert v["error"]["message"] == "--output: invalid value 'xml'"
    assert v["error"]["retryable"] is False  # 0.16: error.retryable defaults to false
    assert "error_code" not in v
    assert v["trace"] == {}  # 0.16: all events have trace by default


def test_build_cli_error_with_hint():
    v = build_cli_error("unknown action: foo", hint="valid actions: echo, ping")
    assert v["kind"] == "error"
    assert v["error"]["hint"] == "valid actions: echo, ping"


def test_json_error_builder_with_hint():
    v = json_error("not_configured", "not configured").hint("set PING_HOST").build()
    assert v.to_dict()["kind"] == "error"
    assert v.to_dict()["error"]["code"] == "not_configured"
    assert v.to_dict()["error"]["message"] == "not configured"
    assert v.to_dict()["error"]["hint"] == "set PING_HOST"


def test_json_error_builder_without_hint_has_no_hint_key():
    v = json_error("failed", "something failed").build()
    assert "hint" not in v.to_dict()["error"]


def test_render_all_formats():
    v = json_result({"ok": True}).build()
    v_dict = v.to_dict()
    json_out = render(v_dict, OutputFormat.JSON)
    yaml_out = render(v_dict, OutputFormat.YAML)
    plain_out = render(v_dict, OutputFormat.PLAIN)
    assert '"kind"' in json_out
    assert yaml_out.startswith("---")
    assert "kind=result" in plain_out


def test_error_round_trip_is_valid_jsonl():
    v = build_cli_error("unknown flag: --foo")
    line = render(v, OutputFormat.JSON)
    parsed = json.loads(line)
    assert parsed["kind"] == "error"
    assert parsed["error"]["code"] == "cli_error"
    assert "\n" not in line


if __name__ == "__main__":
    if hasattr(signal, "SIGPIPE"):
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    try:
        main()
    except BrokenPipeError:
        sys.stdout = open(os.devnull, "w")
        os._exit(0)
