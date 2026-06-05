"""Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.

Demonstrates: human --help (one-level) plus orthogonal --recursive scope and
--output json|yaml|markdown format for full surface export, cli_parse_output,
cli_parse_log_filters, cli_output, build_cli_error, --dry-run, and error hints.

Run:  PYTHONPATH=. python3 examples/agent_cli.py --help
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive --output json
      PYTHONPATH=. python3 examples/agent_cli.py --help --recursive --output markdown
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
import sys

from agent_first_data import (
    OutputFormat,
    SkillAction,
    SkillAgentSelection,
    SkillError,
    SkillOptions,
    SkillScope,
    SkillSpec,
    build_cli_error,
    build_json,
    build_json_error,
    build_json_ok,
    cli_output,
    cli_parse_log_filters,
    cli_parse_output,
    output_json,
    run_skill_admin,
)

# A fictional spore's embedded Agent Skill, used by the `skill` subcommand to
# demonstrate run_skill_admin.
WIDGET_SKILL = (
    "---\nname: agent-first-widget\n"
    "description: Example skill bundled by the agent-cli demo.\n"
    "---\n\n# Agent-First Widget\n\nExample behavior rules go here.\n"
)
WIDGET_SPEC = SkillSpec(
    name="agent-first-widget",
    source=WIDGET_SKILL,
    title="Agent-First Widget",
    marker_slug="afwidget",
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="agent-cli",
        description="Minimal agent-first CLI example",
        add_help=False,  # we handle --help ourselves
    )
    parser.add_argument("--help", "-h", action="store_true", help="Show this help (one-level)")
    parser.add_argument("--recursive", action="store_true", help="With --help, expand the full command tree (a bare --recursive is ignored)")
    parser.add_argument("--output", default="json", help="Output format: json, yaml, plain; help also accepts markdown")
    parser.add_argument("--log", default="", help="Log categories (comma-separated); --log all (or --verbose) enables every category")
    parser.add_argument("--verbose", action="store_true", help="Enable all log categories (shorthand for --log all)")

    subs = parser.add_subparsers(dest="command")

    echo_p = subs.add_parser("echo", add_help=False, help="Echo back the input as structured output")
    echo_p.add_argument("--help", "-h", action="store_true", help="Show help for echo")
    echo_p.add_argument("--dry-run", action="store_true", help="Preview without executing")

    ping_p = subs.add_parser("ping", add_help=False, help="Ping a remote target")
    ping_p.add_argument("--help", "-h", action="store_true", help="Show help for ping")
    ping_p.add_argument("--host", help="Target host to ping")

    skill_p = subs.add_parser("skill", add_help=False, help="Manage this tool's embedded Agent Skill")
    skill_p.add_argument("--help", "-h", action="store_true", help="Show help for skill")
    skill_p.add_argument("verb", nargs="?", help="status, install, or uninstall")
    skill_p.add_argument("--agent", default="all", help="all, codex, claude-code, opencode")
    skill_p.add_argument("--scope", default="personal", help="personal, project")
    skill_p.add_argument("--skills-dir", dest="skills_dir", default=None, help="Skills directory (requires a single concrete --agent)")
    skill_p.add_argument("--force", action="store_true", help="Overwrite or remove a skill this tool did not manage")

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
    return "\n".join(lines)


def format_markdown_help(parser: argparse.ArgumentParser, command: str | None, recursive: bool) -> str:
    """Format Markdown docs for the selected command; expand the tree if recursive."""
    sub = find_subparser(parser, command)
    if sub is not None:
        return f"# {parser.prog} {command}\n\n```text\n{sub.format_help()}{leaf_global_options_note()}```\n"

    lines = [f"# {parser.prog} - Minimal agent-first CLI example", "", "```text", parser.format_help().rstrip(), "```"]
    if not recursive:
        return "\n".join(lines) + "\n"
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            for name, choice in action.choices.items():
                lines.extend(["", f"## {parser.prog} {name}", "", "```text", choice.format_help().rstrip(), "```"])
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


def output_value(raw: list[str], default: str | None = None) -> str | None:
    for arg in raw:
        if arg.startswith("--output="):
            return arg.split("=", 1)[1]
    if "--output" in raw:
        idx = raw.index("--output")
        if idx + 1 < len(raw) and not raw[idx + 1].startswith("-"):
            return raw[idx + 1]
    return default


def help_requested(raw: list[str]) -> bool:
    return "--help" in raw or "-h" in raw


def recursive_requested(raw: list[str]) -> bool:
    # A help *modifier*: only consulted when --help is present, so a bare
    # --recursive never affects normal command parsing.
    return "--recursive" in raw


def log_enabled(filters: list[str], category: str) -> bool:
    """`all` / `*` (what --verbose expands to) enable every category."""
    return any(f in (category, "all", "*") for f in filters)


def build_request_log(command: str | None) -> dict:
    return build_json("log", {"category": "request", "command": command or "none"})


def build_startup_log() -> dict:
    return build_json("log", {"category": "startup", "event": "startup"})


def global_help_options(include_recursive: bool) -> list[dict]:
    """Global flags documented in the structured (json/yaml) help schema so it
    advertises the help surface — the scope modifier and output formats — like
    the plain and markdown formats do. Only the target command carries it; a leaf
    target omits --recursive (nothing to expand)."""
    opts = [
        {"name": "--output", "help": "Output format: json, yaml, plain; help also accepts markdown"},
        {"name": "--log", "help": "Log categories (comma-separated); --log all (or --verbose) enables every category"},
        {"name": "--verbose", "help": "Enable all log categories (shorthand for --log all)"},
    ]
    if include_recursive:
        opts.append({"name": "--recursive", "help": "With --help, expand the full command tree (a bare --recursive is ignored)"})
    opts.append({"name": "--help", "help": "Show this help (one-level)"})
    return opts


def help_schema(parser: argparse.ArgumentParser, command: str | None, scope: str) -> dict:
    sub = find_subparser(parser, command)
    if sub is not None:
        return {
            "code": "help",
            "scope": scope,
            "name": command,
            "command_path": f"{parser.prog} {command}",
            "usage": sub.format_usage().strip(),
            "help": sub.format_help(),
            "options": global_help_options(False),
        }
    commands = []
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            for name, choice in action.choices.items():
                item = {"name": name, "usage": choice.format_usage().strip()}
                if scope == "recursive":
                    item["help"] = choice.format_help()
                commands.append(item)
    return {
        "code": "help",
        "scope": scope,
        "name": parser.prog,
        "command_path": parser.prog,
        "usage": parser.format_usage().strip(),
        "options": global_help_options(True),
        "commands": commands,
    }


def print_help(parser: argparse.ArgumentParser, args, raw: list[str]) -> None:
    explicit = output_explicit(raw)
    value = output_value(raw, args.output)
    sub = find_subparser(parser, args.command)
    # Scope (--recursive) and format (--output) are orthogonal. A specific
    # subcommand is leaf-level here, so its scope is the same either way.
    recursive = recursive_requested(raw)
    scope = "recursive" if recursive else "one_level"

    if explicit and value is None:
        print(output_json(build_cli_error("missing value for --output: expected plain, json, yaml, or markdown", hint="valid help output formats: plain, markdown, json, yaml")))
        sys.exit(2)

    if not explicit or value == "plain":
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
        print(output_json(build_cli_error(str(e))))
        sys.exit(2)
    print(cli_output(help_schema(parser, args.command, scope), fmt))


def main() -> None:
    parser = build_parser()
    raw = sys.argv[1:]
    if help_requested(raw) and output_explicit(raw) and output_value(raw) is None:
        print(output_json(build_cli_error("missing value for --output: expected plain, json, yaml, or markdown", hint="valid help output formats: plain, markdown, json, yaml")))
        sys.exit(2)
    args, _ = parser.parse_known_args()

    # --help is one-level plain; --recursive expands the tree and --output picks
    # the format. A bare --recursive (no --help) is ignored and parsing continues.
    if args.help:
        print_help(parser, args, raw)
        return

    # Step 1: parse --output with shared helper
    try:
        fmt = cli_parse_output(args.output)
    except ValueError as e:
        print(output_json(build_cli_error(str(e))))
        sys.exit(2)

    # Step 2: parse --log with shared helper (trim + lowercase + dedup)
    log = cli_parse_log_filters(args.log.split(",") if args.log else [])
    if args.verbose:
        # --verbose is shorthand for --log all.
        log.append("all")

    # Each diagnostic line self-tags with its `category`, so `--log all` reveals
    # the full set from real output rather than a static help list.
    if log_enabled(log, "request"):
        print(cli_output(build_request_log(args.command), fmt))
    if log_enabled(log, "startup"):
        print(cli_output(build_startup_log(), fmt))

    # Step 3: no subcommand → error with hint
    if not args.command:
        print(output_json(build_cli_error("no subcommand provided", hint="try: agent-cli --help")))
        sys.exit(2)

    if args.command == "echo":
        # Step 4: --dry-run → preview without executing
        if args.dry_run:
            preview = build_json("dry_run", {"action": "echo", "log": log}, trace={"duration_ms": 0})
            print(cli_output(preview, fmt))
            return

        result = build_json_ok({"action": "echo", "log": log})
        print(cli_output(result, fmt))

    elif args.command == "ping":
        # Step 5: demonstrate build_json_error with hint on failure
        if not args.host:
            err = build_json_error("ping target not configured", hint="set PING_HOST or pass --host", trace={"duration_ms": 0})
            print(cli_output(err, fmt))
            sys.exit(1)

    elif args.command == "skill":
        # Step 6: wire the embedded Agent Skill installer to the library.
        sys.exit(run_skill(args, fmt))


def build_skill_options(args):
    """Parse the --agent/--scope flags into library enums.

    Returns ``(options, None)`` or ``(None, (message, hint))`` on an unknown value.
    """
    agents = {
        "all": SkillAgentSelection.ALL,
        "codex": SkillAgentSelection.CODEX,
        "claude-code": SkillAgentSelection.CLAUDE_CODE,
        "opencode": SkillAgentSelection.OPENCODE,
    }
    agent = agents.get(args.agent)
    if agent is None:
        return None, (f"invalid --agent '{args.agent}'", "valid values: all, codex, claude-code, opencode")
    scopes = {"personal": SkillScope.PERSONAL, "project": SkillScope.PROJECT}
    scope = scopes.get(args.scope)
    if scope is None:
        return None, (f"invalid --scope '{args.scope}'", "valid values: personal, project")
    return SkillOptions(agent=agent, scope=scope, skills_dir=args.skills_dir, force=args.force), None


def run_skill(args, fmt) -> int:
    """Wire the parsed `skill` subcommand to the library and print the result.

    Returns the process exit code (0 ok, 1 action error, 2 bad flag value).
    """
    actions = {
        "status": SkillAction.STATUS,
        "install": SkillAction.INSTALL,
        "uninstall": SkillAction.UNINSTALL,
    }
    action = actions.get(args.verb)
    if action is None:
        err = build_cli_error(
            "skill requires a subcommand: status, install, uninstall",
            hint="example: agent-cli skill status --agent opencode",
        )
        print(cli_output(err, fmt))
        return 2

    options, parse_error = build_skill_options(args)
    if parse_error is not None:
        message, hint = parse_error
        print(cli_output(build_cli_error(message, hint=hint), fmt))
        return 2

    try:
        report = run_skill_admin(WIDGET_SPEC, action, options)
    except SkillError as e:
        print(cli_output(build_cli_error(e.message, hint=e.hint), fmt))
        return 1
    # The report is structured; serialize it for output.
    print(cli_output(report.to_dict(), fmt))
    return 0


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


def test_recursive_markdown_export_contains_all_subcommand_details():
    parser = build_parser()
    md = format_markdown_help(parser, None, True)
    assert "# agent-cli" in md, "markdown export must include root heading"
    assert "--dry-run" in md, "recursive markdown export must include echo's --dry-run"
    assert "--host" in md, "recursive markdown export must include ping's --host"


def test_one_level_markdown_omits_descendant_details():
    parser = build_parser()
    md = format_markdown_help(parser, None, False)
    assert "# agent-cli" in md, "one-level markdown must include root heading"
    assert "--dry-run" not in md, "one-level markdown must not expand echo's --dry-run"
    assert "--host" not in md, "one-level markdown must not expand ping's --host"


def test_one_level_help_schema_omits_child_flags():
    parser = build_parser()
    schema = help_schema(parser, None, "one_level")
    assert schema["scope"] == "one_level"
    assert not any("help" in command for command in schema["commands"]), (
        "one-level schema must not expand child help"
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
    assert schema["code"] == "help"
    assert schema["scope"] == "recursive"
    assert any("help" in command for command in schema["commands"])


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


def test_parse_output_all_variants():
    assert cli_parse_output("json") is OutputFormat.JSON
    assert cli_parse_output("yaml") is OutputFormat.YAML
    assert cli_parse_output("plain") is OutputFormat.PLAIN
    import pytest
    with pytest.raises(ValueError):
        cli_parse_output("xml")


def test_parse_log_normalizes():
    assert cli_parse_log_filters(["Startup", " REQUEST ", "startup"]) == ["startup", "request"]


def test_log_enabled_wildcards():
    assert not log_enabled([], "startup")
    assert log_enabled(["startup"], "startup")
    assert not log_enabled(["startup"], "request")
    # all / * enable every category, including unnamed ones
    for everything in ("all", "*"):
        assert log_enabled([everything], "startup")
        assert log_enabled([everything], "request")


def test_log_lines_are_category_tagged():
    req = build_request_log(None)
    assert req["code"] == "log" and req["category"] == "request" and req["command"] == "none"
    start = build_startup_log()
    assert start["code"] == "log" and start["category"] == "startup"


def test_build_cli_error_structure():
    v = build_cli_error("--output: invalid value 'xml'")
    assert v["code"] == "error"
    assert v["error_code"] == "invalid_request"
    assert v["retryable"] is False
    assert v["trace"]["duration_ms"] == 0


def test_build_cli_error_with_hint():
    v = build_cli_error("unknown action: foo", hint="valid actions: echo, ping")
    assert v["code"] == "error"
    assert v["hint"] == "valid actions: echo, ping"


def test_build_json_error_with_hint():
    v = build_json_error("not configured", hint="set PING_HOST")
    assert v["code"] == "error"
    assert v["error"] == "not configured"
    assert v["hint"] == "set PING_HOST"


def test_build_json_error_without_hint_has_no_hint_key():
    v = build_json_error("something failed")
    assert "hint" not in v


def test_cli_output_all_formats():
    v = {"code": "ok"}
    json_out = cli_output(v, OutputFormat.JSON)
    yaml_out = cli_output(v, OutputFormat.YAML)
    plain_out = cli_output(v, OutputFormat.PLAIN)
    assert '"code"' in json_out
    assert yaml_out.startswith("---")
    assert "code=ok" in plain_out


def test_error_round_trip_is_valid_jsonl():
    v = build_cli_error("unknown flag: --foo")
    line = output_json(v)
    parsed = json.loads(line)
    assert parsed["code"] == "error"
    assert "\n" not in line


if __name__ == "__main__":
    main()
