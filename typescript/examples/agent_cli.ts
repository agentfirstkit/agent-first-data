/**
 * Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
 *
 * Demonstrates: human --help (one-level) plus orthogonal --recursive scope and
 * --output json|yaml|markdown format for full surface export, cliParseOutput,
 * cliParseLogFilters, cliOutput, buildCliError, --dry-run, and error hints.
 *
 * Run:  npx tsx examples/agent_cli.ts --help
 *       npx tsx examples/agent_cli.ts --help --recursive
 *       npx tsx examples/agent_cli.ts --help --recursive --output json
 *       npx tsx examples/agent_cli.ts --help --recursive --output markdown
 *       npx tsx examples/agent_cli.ts --version --output json
 *       npx tsx examples/agent_cli.ts echo --help
 *       npx tsx examples/agent_cli.ts echo --output json
 *       npx tsx examples/agent_cli.ts echo --dry-run --output yaml
 *       npx tsx examples/agent_cli.ts ping --output json
 *       npx tsx examples/agent_cli.ts echo --output yaml --log startup,request
 *       npx tsx examples/agent_cli.ts --log all ping   # or --verbose
 * Test: npx tsx --test examples/agent_cli.ts
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  type JsonValue,
  type OutputFormat,
  type SkillAction,
  type SkillAgentSelection,
  type SkillOptions,
  type SkillScope,
  type SkillSpec,
  SkillError,
  buildCliError,
  buildJson,
  buildJsonError,
  buildJsonOk,
  cliOutput,
  cliHandleVersionOrContinue,
  cliParseLogFilters,
  cliParseOutput,
  outputJson,
  runSkillAdmin,
} from "../src/index.js";

// A fictional spore's embedded Agent Skill, used by the `skill` subcommand to
// demonstrate runSkillAdmin.
const WIDGET_SKILL =
  "---\nname: agent-first-widget\ndescription: Example skill bundled by the agent-cli demo.\n---\n\n# Agent-First Widget\n\nExample behavior rules go here.\n";
const WIDGET_SPEC: SkillSpec = {
  name: "agent-first-widget",
  source: WIDGET_SKILL,
  title: "Agent-First Widget",
  markerSlug: "afwidget",
};
const AGENT_CLI_VERSION = "0.13.0";

interface Subcommand {
  name: string;
  about: string;
  flags: string;
}

const SUBCOMMANDS: Subcommand[] = [
  { name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing" },
  { name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping" },
  {
    name: "skill",
    about: "Manage this tool's embedded Agent Skill",
    flags:
      "  status|install|uninstall  Skill action\n  --agent      all, codex, claude-code, opencode (default: all)\n  --scope      personal, project (default: personal)\n  --skills-dir Skills directory (requires a single concrete --agent)\n  --force      Overwrite or remove a skill this tool did not manage",
  },
];

/** Format one-level help for the root command. */
function formatRootHelp(): string {
  const lines = [
    "agent-cli — Minimal agent-first CLI example",
    "",
    "Usage: agent-cli [OPTIONS] <COMMAND>",
    "",
    "Options:",
    "  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown",
    "  --log <FILTERS>    Log categories (comma-separated); --log all (or --verbose) enables every category",
    "  --verbose          Enable all log categories (shorthand for --log all)",
    "  --help             Show this help (one-level); add --recursive to expand all subcommands",
    "  --recursive        With --help, expand the full command tree; --output picks the format",
    "",
    "Commands:",
  ];
  for (const sc of SUBCOMMANDS) {
    lines.push(`  ${sc.name.padEnd(8)} ${sc.about}`);
  }
  return lines.join("\n") + "\n";
}

/** Format recursive help for root command and all subcommands. */
function formatCompleteHelp(): string {
  const lines = [formatRootHelp().trimEnd()];
  for (const sc of SUBCOMMANDS) {
    lines.push("", "=".repeat(60), `agent-cli ${sc.name}`, "=".repeat(60));
    lines.push(sc.about, "", "Flags:", sc.flags);
  }
  return lines.join("\n") + "\n";
}

/**
 * Format help scoped to a single subcommand. When the subcommand is the help
 * target (withGlobals), it also documents the global --output formats so even a
 * leaf `--help` advertises them. Descendants in a recursive dump pass
 * withGlobals=false: the root already documented the modifiers once, so
 * repeating them per command would be pure noise.
 */
function formatSubcommandHelp(name: string, withGlobals = false): string {
  const sc = SUBCOMMANDS.find((s) => s.name === name);
  if (!sc) return "";
  let help = `agent-cli ${sc.name} — ${sc.about}\n\nFlags:\n${sc.flags}\n`;
  if (withGlobals) {
    help += "\nGlobal options:\n  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n";
  }
  return help;
}

function formatMarkdownHelp(command: string | undefined, recursive: boolean): string {
  if (command) {
    const sc = SUBCOMMANDS.find((s) => s.name === command);
    if (sc) {
      return `# agent-cli ${sc.name} - ${sc.about}\n\n\`\`\`text\n${formatSubcommandHelp(sc.name, true)}\`\`\`\n`;
    }
  }

  const lines = [
    "# agent-cli - Minimal agent-first CLI example",
    "",
    "```text",
    formatRootHelp().trimEnd(),
    "```",
  ];
  if (!recursive) return `${lines.join("\n")}\n`;
  for (const sc of SUBCOMMANDS) {
    lines.push("", `## agent-cli ${sc.name} - ${sc.about}`, "", "```text", formatSubcommandHelp(sc.name, false).trimEnd(), "```");
  }
  return `${lines.join("\n")}\n`;
}

/**
 * Global flags documented in the structured (json/yaml) help schema so it
 * advertises the help surface — the scope modifier and output formats — like the
 * plain and markdown formats do. Only the target command carries it; a leaf
 * target omits --recursive (nothing to expand).
 */
function globalHelpOptions(includeRecursive: boolean): JsonValue[] {
  const opts: JsonValue[] = [
    { name: "--output", help: "Output format: json, yaml, plain (default: json); help also accepts markdown" },
    { name: "--log", help: "Log categories (comma-separated); --log all (or --verbose) enables every category" },
    { name: "--verbose", help: "Enable all log categories (shorthand for --log all)" },
  ];
  if (includeRecursive) {
    opts.push({ name: "--recursive", help: "With --help, expand the full command tree (a bare --recursive is ignored)" });
  }
  opts.push({ name: "--help", help: "Show this help (one-level)" });
  return opts;
}

function helpSchema(command: string | undefined, scope: "one_level" | "recursive"): JsonValue {
  const commandPath = command ? `agent-cli ${command}` : "agent-cli";
  if (command) {
    const sc = SUBCOMMANDS.find((s) => s.name === command);
    if (sc) {
      return {
        code: "help",
        scope,
        command_path: commandPath,
        name: sc.name,
        about: sc.about,
        flags: sc.flags,
        options: globalHelpOptions(false),
      };
    }
  }
  return {
    code: "help",
    scope,
    command_path: commandPath,
    name: "agent-cli",
    about: "Minimal agent-first CLI example",
    options: globalHelpOptions(true),
    commands: SUBCOMMANDS.map((sc) => {
      const entry: Record<string, JsonValue> = { name: sc.name, about: sc.about };
      if (scope === "recursive") entry.flags = sc.flags;
      return entry;
    }),
  };
}

function hasExplicitOutput(args: string[]): boolean {
  return args.includes("--output") || args.some((a) => a.startsWith("--output="));
}

function argValue(args: string[], flag: string): string | undefined {
  const inline = args.find((a) => a.startsWith(`${flag}=`));
  if (inline !== undefined) return inline.slice(flag.length + 1);
  const idx = args.indexOf(flag);
  const value = idx !== -1 ? args[idx + 1] : undefined;
  return value !== undefined && !value.startsWith("-") ? value : undefined;
}

function renderHelpOutput(
  command: string | undefined,
  outputArg: string | undefined,
  outputExplicit: boolean,
  recursive: boolean,
): string {
  if (outputExplicit && outputArg === undefined) {
    throw new Error("missing value for --output: expected plain, json, yaml, or markdown");
  }
  // Scope (--recursive) and format (--output) are orthogonal. A specific
  // subcommand is leaf-level here, so its scope is the same either way.
  const scope = recursive ? "recursive" : "one_level";
  if (!outputExplicit || outputArg === "plain") {
    if (command) return formatSubcommandHelp(command, true);
    return recursive ? formatCompleteHelp() : formatRootHelp();
  }
  if (outputArg === "markdown") return formatMarkdownHelp(command, recursive);
  const fmt = cliParseOutput(outputArg);
  return `${cliOutput(helpSchema(command, scope), fmt)}\n`;
}

// Flags that consume the following token as their value.
const VALUE_FLAGS = new Set(["--output", "--log", "--host", "--agent", "--scope", "--skills-dir"]);

/** `all` / `*` (what --verbose expands to) enable every diagnostic category. */
function logEnabled(filters: string[], category: string): boolean {
  return filters.some((f) => f === category || f === "all" || f === "*");
}

function buildRequestLog(command: string | undefined): JsonValue {
  return buildJson("log", { category: "request", command: command ?? "none" });
}

function buildStartupLog(): JsonValue {
  return buildJson("log", { category: "startup", event: "startup" });
}

/** Collect positional arguments, skipping flags and the values they consume. */
function positionalArgs(args: string[]): string[] {
  const out: string[] = [];
  for (let i = 0; i < args.length; i++) {
    const a = args[i]!;
    if (a.startsWith("--")) continue;
    if (i > 0 && VALUE_FLAGS.has(args[i - 1]!)) continue;
    out.push(a);
  }
  return out;
}

function main(): void {
  const args = process.argv.slice(2);
  try {
    const version = cliHandleVersionOrContinue(args, "agent-cli", AGENT_CLI_VERSION);
    if (version !== undefined) {
      process.stdout.write(version);
      return;
    }
  } catch (e) {
    console.log(outputJson(buildCliError((e as Error).message, "valid version output formats: json, yaml, plain")));
    process.exit(2);
  }

  const showHelp = args.includes("--help") || args.includes("-h");
  // A help modifier only: consulted just below when showHelp is true, so a bare
  // --recursive never affects normal command parsing.
  const recursive = args.includes("--recursive");
  const argsWithoutHelp = args.filter((a) => a !== "--help" && a !== "-h");
  const positionals = positionalArgs(argsWithoutHelp);
  const command = positionals[0];

  // --help is one-level plain; --recursive expands the tree and --output picks
  // the format. A bare --recursive (no --help) falls through to normal parsing.
  if (showHelp) {
    try {
      process.stdout.write(renderHelpOutput(command, argValue(args, "--output"), hasExplicitOutput(args), recursive));
    } catch (e) {
      console.log(outputJson(buildCliError((e as Error).message)));
      process.exit(2);
    }
    return;
  }

  const logIdx = args.indexOf("--log");
  const dryRun = args.includes("--dry-run");
  const hostIdx = args.indexOf("--host");
  const outputArg = argValue(args, "--output") ?? (hasExplicitOutput(args) ? "" : "json");
  const logArg = logIdx !== -1 ? args[logIdx + 1] : "";
  const host = hostIdx !== -1 ? args[hostIdx + 1] : undefined;

  // Step 1: parse --output with shared helper
  let fmt: OutputFormat;
  try {
    fmt = cliParseOutput(outputArg ?? "json");
  } catch (e) {
    console.log(outputJson(buildCliError((e as Error).message)));
    process.exit(2);
  }

  // Step 2: parse --log with shared helper (trim + lowercase + dedup)
  const log = cliParseLogFilters(logArg ? logArg.split(",") : []);
  if (args.includes("--verbose")) {
    // --verbose is shorthand for --log all.
    log.push("all");
  }

  // Each diagnostic line self-tags with its `category`, so `--log all` reveals
  // the full set from real output rather than a static help list.
  if (logEnabled(log, "request")) {
    console.log(cliOutput(buildRequestLog(command), fmt));
  }
  if (logEnabled(log, "startup")) {
    console.log(cliOutput(buildStartupLog(), fmt));
  }

  // Step 3: no subcommand → error with hint
  if (!command) {
    console.log(outputJson(buildCliError("no subcommand provided", "try: agent-cli --help")));
    process.exit(2);
  }

  switch (command) {
    case "echo": {
      // Step 4: --dry-run → preview without executing
      if (dryRun) {
        const preview = buildJson("dry_run", { action: "echo", log }, { duration_ms: 0 });
        console.log(cliOutput(preview, fmt));
        return;
      }
      const result = buildJsonOk({ action: "echo", log });
      console.log(cliOutput(result, fmt));
      break;
    }
    case "ping": {
      // Step 5: demonstrate buildJsonError with hint on failure
      if (!host) {
        const err = buildJsonError("ping target not configured", "set PING_HOST or pass --host", { duration_ms: 0 });
        console.log(cliOutput(err, fmt));
        process.exit(1);
      }
      break;
    }
    case "skill": {
      // Step 6: wire the embedded Agent Skill installer to the library.
      const agentIdx = args.indexOf("--agent");
      const scopeIdx = args.indexOf("--scope");
      const skillsDirIdx = args.indexOf("--skills-dir");
      const agentArg = agentIdx !== -1 ? args[agentIdx + 1] : "all";
      const scopeArg = scopeIdx !== -1 ? args[scopeIdx + 1] : "personal";
      const skillsDir = skillsDirIdx !== -1 ? args[skillsDirIdx + 1] : undefined;
      const force = args.includes("--force");
      process.exit(runSkill(positionals[1], agentArg ?? "all", scopeArg ?? "personal", skillsDir, force, fmt));
      break;
    }
    default: {
      console.log(outputJson(buildCliError(`unknown command: ${command}`, "valid commands: echo, ping, skill")));
      process.exit(2);
    }
  }
}

/**
 * Wire the parsed `skill` subcommand to the library and print the result.
 * Returns the process exit code (0 ok, 1 action error, 2 bad flag value).
 */
function runSkill(
  verb: string | undefined,
  agentArg: string,
  scopeArg: string,
  skillsDir: string | undefined,
  force: boolean,
  fmt: OutputFormat,
): number {
  const actions: Record<string, SkillAction> = { status: "status", install: "install", uninstall: "uninstall" };
  const action = verb !== undefined ? actions[verb] : undefined;
  if (action === undefined) {
    const err = buildCliError("skill requires a subcommand: status, install, uninstall", "example: agent-cli skill status --agent opencode");
    console.log(cliOutput(err, fmt));
    return 2;
  }

  const built = buildSkillOptions(agentArg, scopeArg, skillsDir, force);
  if ("error" in built) {
    console.log(cliOutput(buildCliError(built.error, built.hint), fmt));
    return 2;
  }

  try {
    const report = runSkillAdmin(WIDGET_SPEC, action, built.options);
    // The report is structured; serialize it for output (it is already JSON-shaped).
    console.log(cliOutput(report as unknown as JsonValue, fmt));
    return 0;
  } catch (e) {
    if (e instanceof SkillError) {
      console.log(cliOutput(buildCliError(e.message, e.hint), fmt));
      return 1;
    }
    throw e;
  }
}

/** Parse the --agent/--scope string flags into the library types. */
function buildSkillOptions(
  agentArg: string,
  scopeArg: string,
  skillsDir: string | undefined,
  force: boolean,
): { options: SkillOptions } | { error: string; hint: string } {
  const agents: Record<string, SkillAgentSelection> = {
    all: "all",
    codex: "codex",
    "claude-code": "claude-code",
    opencode: "opencode",
  };
  const agent = agents[agentArg];
  if (agent === undefined) {
    return { error: `invalid --agent '${agentArg}'`, hint: "valid values: all, codex, claude-code, opencode" };
  }
  const scopes: Record<string, SkillScope> = { personal: "personal", project: "project" };
  const scope = scopes[scopeArg];
  if (scope === undefined) {
    return { error: `invalid --scope '${scopeArg}'`, hint: "valid values: personal, project" };
  }
  return { options: { agent, scope, skillsDir, force } };
}

// ── Tests (run via: npx tsx --test examples/agent_cli.ts) ────────────────────

if (process.env["NODE_TEST_CONTEXT"]) {
  describe("help", () => {
    it("root help is one-level", () => {
      const help = formatRootHelp();
      assert.ok(help.includes("echo"), "root --help must include echo");
      assert.ok(help.includes("ping"), "root --help must include ping");
      assert.ok(help.includes("--output"), "root --help must include --output");
      assert.ok(!help.includes("--help-all"), "root --help must not include removed --help-all");
      assert.ok(!help.includes("--dry-run"), "root --help must NOT include echo's --dry-run");
      assert.ok(!help.includes("--host"), "root --help must NOT include ping's --host");
    });

    it("recursive markdown export contains subcommand details", () => {
      const help = formatMarkdownHelp(undefined, true);
      assert.ok(help.includes("# agent-cli"), "markdown export must include root heading");
      assert.ok(help.includes("--dry-run"), "recursive markdown export must include echo's --dry-run");
      assert.ok(help.includes("--host"), "recursive markdown export must include ping's --host");
    });

    it("one-level markdown omits descendant details", () => {
      const help = formatMarkdownHelp(undefined, false);
      assert.ok(help.includes("# agent-cli"), "one-level markdown must include root heading");
      assert.ok(!help.includes("--dry-run"), "one-level markdown must not expand echo's --dry-run");
      assert.ok(!help.includes("--host"), "one-level markdown must not expand ping's --host");
    });

    it("recursive help contains subcommand details", () => {
      const help = formatCompleteHelp();
      assert.ok(help.includes("echo"), "recursive help must include echo");
      assert.ok(help.includes("ping"), "recursive help must include ping");
      assert.ok(help.includes("--output"), "recursive help must include --output");
      assert.ok(help.includes("--dry-run"), "recursive help must include echo's --dry-run");
      assert.ok(help.includes("--host"), "recursive help must include ping's --host");
    });

    it("help schema is recursive export", () => {
      const schema = helpSchema(undefined, "recursive") as Record<string, unknown>;
      assert.equal(schema.code, "help");
      assert.equal(schema.scope, "recursive");
      const commands = schema.commands as Array<Record<string, unknown>>;
      assert.ok(commands.some((command) => command.flags !== undefined), "recursive schema must include child flags");
    });

    it("one-level help schema omits child flags", () => {
      const schema = helpSchema(undefined, "one_level") as Record<string, unknown>;
      assert.equal(schema.scope, "one_level");
      const commands = schema.commands as Array<Record<string, unknown>>;
      assert.ok(commands.every((command) => command.flags === undefined), "one-level schema must not expand child flags");
    });

    it("--recursive without --help does not render help", () => {
      // renderHelpOutput is only reached when --help is present; with --output
      // omitted and recursive=true it stays one-level-or-recursive plain. The
      // guarantee that a bare --recursive falls through lives in main(), which
      // only consults `recursive` inside the showHelp branch. Here we assert the
      // orthogonal contract: recursive flips scope without forcing a format.
      const oneLevel = renderHelpOutput(undefined, undefined, false, false);
      const recursivePlain = renderHelpOutput(undefined, undefined, false, true);
      assert.ok(!oneLevel.includes("--dry-run"), "one-level plain must not expand subcommands");
      assert.ok(recursivePlain.includes("--dry-run"), "recursive plain must expand subcommands");
    });

    it("subcommand help scoped to subtree", () => {
      const echoHelp = formatSubcommandHelp("echo", true);
      assert.ok(echoHelp.includes("--dry-run"), "echo --help must include --dry-run");
      assert.ok(!echoHelp.includes("--host"), "echo --help must NOT include ping's --host");
    });

    it("leaf help target documents formats; descendants do not", () => {
      const target = formatSubcommandHelp("echo", true);
      assert.ok(target.includes("--output"), "leaf --help target must document --output");
      assert.ok(target.includes("markdown"), "leaf --help target must mention markdown");
      const descendant = formatSubcommandHelp("echo", false);
      assert.ok(!descendant.includes("Global options"), "descendant rendering must not repeat global options");
    });

    it("recursive dumps document modifiers once, not per descendant", () => {
      const plain = formatCompleteHelp();
      const md = formatMarkdownHelp(undefined, true);
      assert.equal((plain.match(/Global options/g) ?? []).length, 0, "recursive plain must not repeat global-options note");
      assert.equal((md.match(/Global options/g) ?? []).length, 0, "recursive markdown must not repeat global-options note");
    });

    it("every help schema documents the formats", () => {
      const root = JSON.stringify(helpSchema(undefined, "one_level"));
      for (const token of ["--output", "markdown", "--recursive"]) {
        assert.ok(root.includes(token), `root help schema must document ${token}`);
      }
      const leaf = JSON.stringify(helpSchema("echo", "one_level"));
      assert.ok(leaf.includes("--output") && leaf.includes("markdown"), "leaf help schema must document --output formats");
    });
  });

  describe("agent_cli example", () => {
    it("parse output all variants", () => {
      assert.equal(cliParseOutput("json"), "json");
      assert.equal(cliParseOutput("yaml"), "yaml");
      assert.equal(cliParseOutput("plain"), "plain");
      assert.throws(() => cliParseOutput("xml"));
    });

    it("parse log normalizes", () => {
      assert.deepEqual(
        cliParseLogFilters(["Startup", " REQUEST ", "startup"]),
        ["startup", "request"]
      );
    });

    it("log enabled honors all/* wildcards", () => {
      assert.ok(!logEnabled([], "startup"));
      assert.ok(logEnabled(["startup"], "startup"));
      assert.ok(!logEnabled(["startup"], "request"));
      for (const everything of ["all", "*"]) {
        assert.ok(logEnabled([everything], "startup"), `${everything} enables startup`);
        assert.ok(logEnabled([everything], "request"), `${everything} enables request`);
      }
    });

    it("log lines are category-tagged", () => {
      const req = buildRequestLog(undefined) as Record<string, unknown>;
      assert.equal(req["code"], "log");
      assert.equal(req["category"], "request");
      assert.equal(req["command"], "none");
      const start = buildStartupLog() as Record<string, unknown>;
      assert.equal(start["category"], "startup");
    });

    it("build cli error structure", () => {
      const v = buildCliError("--output: invalid value 'xml'") as Record<string, unknown>;
      assert.equal(v["code"], "error");
      assert.equal(v["error"], "--output: invalid value 'xml'");
      assert.equal(v["error_code"], undefined);
      assert.equal(v["retryable"], undefined);
      assert.equal(v["trace"], undefined);
    });

    it("build cli error with hint", () => {
      const v = buildCliError("unknown action: foo", "valid actions: echo, ping") as Record<string, unknown>;
      assert.equal(v["code"], "error");
      assert.equal(v["hint"], "valid actions: echo, ping");
    });

    it("build json error with hint", () => {
      const v = buildJsonError("not configured", "set PING_HOST") as Record<string, unknown>;
      assert.equal(v["code"], "error");
      assert.equal(v["error"], "not configured");
      assert.equal(v["hint"], "set PING_HOST");
    });

    it("build json error without hint has no hint key", () => {
      const v = buildJsonError("something failed") as Record<string, unknown>;
      assert.equal(v["hint"], undefined);
    });

    it("cli output all formats", () => {
      const v = { code: "ok" };
      const jsonOut = cliOutput(v, "json");
      const yamlOut = cliOutput(v, "yaml");
      const plainOut = cliOutput(v, "plain");
      assert.ok(jsonOut.includes('"code"'));
      assert.ok(yamlOut.startsWith("---"));
      assert.ok(plainOut.includes("code=ok"));
    });

    it("error round trip is valid jsonl", () => {
      const v = buildCliError("unknown flag: --foo");
      const line = outputJson(v);
      const parsed = JSON.parse(line);
      assert.equal(parsed.code, "error");
      assert.ok(!line.includes("\n"));
    });
  });
}

// Only run main() when executed directly, not during `--test`
if (!process.env["NODE_TEST_CONTEXT"]) {
  main();
}
