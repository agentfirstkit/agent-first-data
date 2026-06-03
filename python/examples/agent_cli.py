"""Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.

Demonstrates: complete --help (all subcommands in one output), cli_parse_output,
cli_parse_log_filters, cli_output, build_cli_error, --dry-run, and error hints.

Run:  PYTHONPATH=. python3 examples/agent_cli.py --help
      PYTHONPATH=. python3 examples/agent_cli.py echo --help
      PYTHONPATH=. python3 examples/agent_cli.py echo --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --dry-run --output yaml
      PYTHONPATH=. python3 examples/agent_cli.py ping --output json
      PYTHONPATH=. python3 examples/agent_cli.py echo --output yaml --log startup,request
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
    parser.add_argument("--help", "-h", action="store_true", help="Show complete help")
    parser.add_argument("--output", default="json", help="Output format: json, yaml, plain")
    parser.add_argument("--log", default="", help="Log categories (comma-separated)")

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


def main() -> None:
    parser = build_parser()
    args, _ = parser.parse_known_args()

    # Complete help: --help expands all subcommands in one output.
    # Subcommand --help expands only that subcommand.
    if args.help:
        if args.command:
            # Scoped to subcommand
            for action in parser._subparsers._actions:
                if isinstance(action, argparse._SubParsersAction):
                    sub = action.choices.get(args.command)
                    if sub:
                        print(sub.format_help())
                        return
        print(format_complete_help(parser))
        return

    # Step 1: parse --output with shared helper
    try:
        fmt = cli_parse_output(args.output)
    except ValueError as e:
        print(output_json(build_cli_error(str(e))))
        sys.exit(2)

    # Step 2: parse --log with shared helper (trim + lowercase + dedup)
    log = cli_parse_log_filters(args.log.split(",") if args.log else [])

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


def test_complete_help_contains_all_subcommands():
    parser = build_parser()
    md = format_complete_help(parser)
    assert "echo" in md, "root --help must include echo subcommand"
    assert "ping" in md, "root --help must include ping subcommand"
    assert "--output" in md, "root --help must include global flags"
    assert "--dry-run" in md, "root --help must include echo's --dry-run"
    assert "--host" in md, "root --help must include ping's --host"


def test_subcommand_help_scoped():
    parser = build_parser()
    for action in parser._subparsers._actions:
        if isinstance(action, argparse._SubParsersAction):
            echo_help = action.choices["echo"].format_help()
            assert "--dry-run" in echo_help, "echo --help must include --dry-run"
            assert "--host" not in echo_help, "echo --help must NOT include ping's --host"


def test_parse_output_all_variants():
    assert cli_parse_output("json") is OutputFormat.JSON
    assert cli_parse_output("yaml") is OutputFormat.YAML
    assert cli_parse_output("plain") is OutputFormat.PLAIN
    import pytest
    with pytest.raises(ValueError):
        cli_parse_output("xml")


def test_parse_log_normalizes():
    assert cli_parse_log_filters(["Startup", " REQUEST ", "startup"]) == ["startup", "request"]


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
