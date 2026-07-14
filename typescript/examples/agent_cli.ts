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
 *       npx tsx examples/agent_cli.ts --stdout-file /tmp/agent-cli.out --stderr-file /tmp/agent-cli.err ping
 * Test: npx tsx --test examples/agent_cli.ts
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

process.stdout.on("error", (err: NodeJS.ErrnoException) => {
  if (err.code === "EPIPE") {
    process.exit(0);
  }
  throw err;
});

import {
  type JsonValue,
  type OutputFormat,
  buildCliError,
  jsonLog,
  jsonError,
  jsonResult,
  cliOutput,
  cliHandleVersionOrContinue,
  cliParseLogFilters,
  cliParseOutput,
  outputJson,
  LogFilters,
} from "../src/index.js";
import {
  type SkillAction,
  type SkillAgentSelection,
  type SkillOptions,
  type SkillScope,
  type SkillSpec,
  SkillError,
  runSkillAdmin,
} from "../src/skill.js";
import { installStreamRedirectFromRawArgs } from "../src/stream_redirect.js";

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
const AFDATA_VERSION = "0.15.0";
const HELP_DEFAULT_API_KEY_SECRET = "sk-help-default";
const PING_HOST_ENV = "PING_HOST";

interface Subcommand {
  name: string;
  about: string;
  flags: string;
}

const SUBCOMMANDS: Subcommand[] = [
  { name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing" },
  { name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping" },
  { name: "cancel", about: "Return a tool-defined cancellation error", flags: "  (no flags)" },
  {
    name: "skill",
    about: "Manage this tool's embedded Agent Skill",
    flags:
      "  status|install|uninstall  Skill action\n  --agent      all, codex, claude-code, opencode, hermes (default: all)\n  --scope      personal, workspace (default: personal)\n  --skills-dir Skills directory (requires a single concrete --agent)\n  --force      Overwrite or remove a skill this tool did not manage",
  },
];

/**
 * Format one-level help for the root command. Markdown rendering passes
 * withTitle=false: the `# agent-cli - <about>` heading already carries the
 * summary, so repeating it as the first line of the fenced block is duplication.
 */
function formatRootHelp(withTitle = true): string {
  const lines: string[] = [];
  if (withTitle) {
    lines.push("agent-cli — Minimal agent-first CLI example", "");
  }
  lines.push(
    "Usage: agent-cli [OPTIONS] <COMMAND>",
    "",
    "Options:",
    "  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown",
    "  --json             Equivalent to --output json",
    "  --log <FILTERS>    Log categories (comma-separated); --log all (or --verbose) enables every category",
    "  --verbose          Enable all log categories (shorthand for --log all)",
    `  --api-key-secret <VALUE> API key used by examples (default: ${redactHelpDefault("--api-key-secret", HELP_DEFAULT_API_KEY_SECRET)})`,
    "  --stdout-file <PATH> Redirect stdout to a file",
    "  --stderr-file <PATH> Redirect stderr to a file",
    "  --help             Show this help (one-level); add --recursive to expand all subcommands",
    "  --recursive        With --help, expand the full command tree; --output picks the format",
    "",
    "Commands:",
  );
  for (const sc of SUBCOMMANDS) {
    lines.push(`  ${sc.name.padEnd(8)} ${sc.about}`);
  }
  return `${lines.join("\n")}\n\nAFDATA: ${AFDATA_VERSION}\n`;
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
function formatSubcommandHelp(name: string, withGlobals = false, withTitle = true): string {
  const sc = SUBCOMMANDS.find((s) => s.name === name);
  if (!sc) return "";
  // Markdown rendering passes withTitle=false: the heading already shows the
  // `agent-cli <name> - <about>` summary, so the fenced block skips it.
  let help = withTitle ? `agent-cli ${sc.name} — ${sc.about}\n\nFlags:\n${sc.flags}\n` : `Flags:\n${sc.flags}\n`;
  if (withGlobals) {
    help += "\nGlobal options:\n  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n";
    help += "  --json             Equivalent to --output json\n";
  }
  if (withGlobals || withTitle) {
    help += `\nAFDATA: ${AFDATA_VERSION}\n`;
  }
  return help;
}

function formatMarkdownHelp(command: string | undefined, recursive: boolean): string {
  if (command) {
    const sc = SUBCOMMANDS.find((s) => s.name === command);
    if (sc) {
      return `# agent-cli ${sc.name} - ${sc.about}\n\n\`\`\`text\n${formatSubcommandHelp(sc.name, true, false)}\`\`\`\n`;
    }
  }

  const lines = [
    "# agent-cli - Minimal agent-first CLI example",
    "",
    "```text",
    formatRootHelp(false).trimEnd(),
    "```",
  ];
  if (!recursive) return `${lines.join("\n")}\n`;
  for (const sc of SUBCOMMANDS) {
    lines.push("", `## agent-cli ${sc.name} - ${sc.about}`, "", "```text", formatSubcommandHelp(sc.name, false, false).trimEnd(), "```");
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
    { name: "--json", help: "Equivalent to --output json" },
    { name: "--log", help: "Log categories (comma-separated); --log all (or --verbose) enables every category" },
    { name: "--verbose", help: "Enable all log categories (shorthand for --log all)" },
    {
      name: "--api-key-secret",
      help: "API key used by examples",
      default_values: [redactHelpDefault("--api-key-secret", HELP_DEFAULT_API_KEY_SECRET)],
    },
    { name: "--stdout-file", help: "Redirect stdout to a file" },
    { name: "--stderr-file", help: "Redirect stderr to a file" },
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
        versions: { afdata: AFDATA_VERSION },
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
    versions: { afdata: AFDATA_VERSION },
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
  return args.includes("--json") || args.includes("--output") || args.some((a) => a.startsWith("--output="));
}

function outputFlagMissing(args: string[]): boolean {
  return args.some((arg, idx) => {
    if (arg === "--output") {
      const value = args[idx + 1];
      return value === undefined || value.startsWith("-");
    }
    return arg.startsWith("--output=") && arg.slice("--output=".length) === "";
  });
}

function argValue(args: string[], flag: string): string | undefined {
  const inline = args.find((a) => a.startsWith(`${flag}=`));
  if (inline !== undefined) return inline.slice(flag.length + 1);
  const idx = args.indexOf(flag);
  const value = idx !== -1 ? args[idx + 1] : undefined;
  return value !== undefined && !value.startsWith("-") ? value : undefined;
}

function outputConflict(args: string[]): string | undefined {
  if (!args.includes("--json")) return undefined;
  const explicit = argValue(args.filter((arg) => arg !== "--json"), "--output");
  if (explicit !== undefined && explicit !== "json") {
    return `conflicting output formats: --json conflicts with --output ${explicit}`;
  }
  return undefined;
}

function resolveOutputArg(args: string[]): string | undefined {
  if (outputFlagMissing(args)) {
    throw new Error("missing value for --output: expected json, yaml, or plain");
  }
  const conflict = outputConflict(args);
  if (conflict !== undefined) throw new Error(conflict);
  if (args.includes("--json")) return "json";
  return argValue(args, "--output") ?? (hasExplicitOutput(args) ? "" : "json");
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

function redactHelpDefault(name: string, value: string): string {
  const normalized = name.replace(/^-+/, "").replaceAll("-", "_");
  return normalized.endsWith("_secret") || normalized.endsWith("_SECRET") ? "***" : value;
}

/** `all` / `*` (what --verbose expands to) enable every diagnostic category. */
function logEnabled(filters: LogFilters, category: string): boolean {
  return filters.enabled(category);
}

function buildRequestLog(command: string | undefined): JsonValue {
  return jsonLog({
    level: "info",
    message: "request",
    category: "request",
    command: command ?? "none",
  })
    .build()
    .toJSON();
}

function buildStartupLog(args: string[], command: string | undefined, output: string, log: LogFilters, verbose: boolean): JsonValue {
  return jsonLog({
    level: "info",
    message: "startup",
    category: "startup",
    event: "startup",
    argv: args,
    parsed: {
      command: command ?? "none",
      output,
      log: Array.from(log.filters),
      verbose,
    },
    effective_config: {
      output,
      log: Array.from(log.filters),
    },
    env: startupEnvSnapshot(),
  })
    .build()
    .toJSON();
}

function startupEnvSnapshot(): JsonValue {
  const value = process.env[PING_HOST_ENV];
  const item: Record<string, JsonValue> = { key: PING_HOST_ENV, present: value !== undefined };
  if (value !== undefined) item.value = value;
  return [item];
}

function strictParseArgs(args: string[]) {
  const parsed = parseArgs({
    args,
    allowPositionals: true,
    strict: true,
    options: {
      help: { type: "boolean", short: "h" },
      recursive: { type: "boolean" },
      output: { type: "string" },
      json: { type: "boolean" },
      log: { type: "string" },
      verbose: { type: "boolean" },
      "api-key-secret": { type: "string" },
      "stdout-file": { type: "string" },
      "stderr-file": { type: "string" },
      "dry-run": { type: "boolean" },
      host: { type: "string" },
      agent: { type: "string" },
      scope: { type: "string" },
      "skills-dir": { type: "string" },
      force: { type: "boolean" },
    },
  });
  const command = parsed.positionals[0];
  const values = parsed.values;
  const has = (name: string) => Object.prototype.hasOwnProperty.call(values, name);
  const helpRequested = values.help === true;
  if (command === undefined) return parsed;
  switch (command) {
    case "echo":
      if (parsed.positionals.length > 1) throw new Error(`unexpected positional argument: ${parsed.positionals[1]}`);
      if (has("host") || has("agent") || has("scope") || has("skills-dir") || has("force")) {
        throw new Error("option is not valid for echo");
      }
      break;
    case "ping":
      if (parsed.positionals.length > 1) throw new Error(`unexpected positional argument: ${parsed.positionals[1]}`);
      if (has("dry-run") || has("agent") || has("scope") || has("skills-dir") || has("force")) {
        throw new Error("option is not valid for ping");
      }
      break;
    case "cancel":
      if (parsed.positionals.length > 1) throw new Error(`unexpected positional argument: ${parsed.positionals[1]}`);
      if (has("dry-run") || has("host") || has("agent") || has("scope") || has("skills-dir") || has("force")) {
        throw new Error("option is not valid for cancel");
      }
      break;
    case "skill":
      if (parsed.positionals.length < 2 && !helpRequested) throw new Error("skill requires a subcommand: status, install, uninstall");
      if (parsed.positionals.length > 2) throw new Error(`unexpected positional argument: ${parsed.positionals[2]}`);
      if (has("dry-run") || has("host")) {
        throw new Error("option is not valid for skill");
      }
      break;
    default:
      throw new Error(`unknown command: ${command}`);
  }
  return parsed;
}

function renderCliParseError(args: string[], message: string, hint = "try: agent-cli --help"): string {
  const err = buildCliError(message, hint);
  try {
    const outputArg = resolveOutputArg(args);
    const fmt = cliParseOutput(outputArg ?? "json");
    return cliOutput(err, fmt);
  } catch {
    return outputJson(err);
  }
}

function main(): void {
  const args = process.argv.slice(2);
  try {
    installStreamRedirectFromRawArgs(args);
  } catch (e) {
    console.log(outputJson(buildCliError((e as Error).message)));
    process.exit(2);
  }

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
  let parsedArgs: ReturnType<typeof strictParseArgs>;
  try {
    parsedArgs = strictParseArgs(args);
  } catch (e) {
    console.log(renderCliParseError(args, (e as Error).message));
    process.exit(2);
  }
  const positionals = parsedArgs.positionals;
  const command = positionals[0];

  // --help is one-level plain; --recursive expands the tree and --output picks
  // the format. A bare --recursive (no --help) falls through to normal parsing.
  if (showHelp) {
    try {
      process.stdout.write(renderHelpOutput(command, resolveOutputArg(args), hasExplicitOutput(args), recursive));
    } catch (e) {
      console.log(outputJson(buildCliError((e as Error).message)));
      process.exit(2);
    }
    return;
  }

  const values = parsedArgs.values;
  const dryRun = values["dry-run"] === true;
  let outputArg: string | undefined;
  try {
    outputArg = resolveOutputArg(args);
  } catch (e) {
    console.log(outputJson(buildCliError((e as Error).message, "valid output formats: json, yaml, plain")));
    process.exit(2);
  }
  const logArg = typeof values.log === "string" ? values.log : "";
  const host = typeof values.host === "string" ? values.host : undefined;

  // Step 1: parse --output with shared helper
  let fmt: OutputFormat;
  try {
    fmt = cliParseOutput(outputArg ?? "json");
  } catch (e) {
    console.log(outputJson(buildCliError((e as Error).message)));
    process.exit(2);
  }

  // Step 2: parse --log with shared helper (trim + lowercase + dedup)
  let logArgsForVerbose = logArg ? logArg.split(",") : [];
  if (values.verbose === true) {
    // --verbose is shorthand for --log all.
    logArgsForVerbose = [...logArgsForVerbose, "all"];
  }
  const log = cliParseLogFilters(logArgsForVerbose);

  // Each diagnostic line self-tags with its `category`, so `--log all` reveals
  // the full set from real output rather than a static help list.
  if (logEnabled(log, "request")) {
    console.log(cliOutput(buildRequestLog(command), fmt));
  }
  if (logEnabled(log, "startup")) {
    console.log(cliOutput(buildStartupLog(args, command, outputArg ?? "json", log, values.verbose === true), fmt));
  }

  // Step 3: no subcommand → error with hint
  if (!command) {
    console.log(cliOutput(buildCliError("no subcommand provided", "try: agent-cli --help"), fmt));
    process.exit(2);
  }

  switch (command) {
    case "echo": {
      // Step 4: --dry-run → preview without executing
      if (dryRun) {
        const preview = jsonResult({ action: "echo", log }).trace({ duration_ms: 0 }).build();
        console.log(cliOutput(preview, fmt));
        return;
      }
      const result = jsonResult({ action: "echo", log }).build();
      console.log(cliOutput(result, fmt));
      break;
    }
    case "ping": {
      // Step 5: demonstrate a protocol v1 error with hint on failure
      const effectiveHost = host ?? process.env[PING_HOST_ENV];
      if (!effectiveHost) {
        const err = jsonError(
          "ping_target_not_configured",
          "ping target not configured"
        )
          .hint("set PING_HOST or pass --host")
          .trace({ duration_ms: 0 })
          .build();
        console.log(cliOutput(err, fmt));
        process.exit(1);
      }
      break;
    }
    case "cancel": {
      const err = jsonError(
        "cancelled",
        "operation cancelled"
      )
        .hint("the operation was cancelled before completion")
        .trace({ duration_ms: 0 })
        .build();
      console.log(cliOutput(err, fmt));
      process.exit(1);
    }
    case "skill": {
      // Step 6: wire the embedded Agent Skill installer to the library.
      const agentArg = typeof values.agent === "string" ? values.agent : "all";
      const scopeArg = typeof values.scope === "string" ? values.scope : "personal";
      const skillsDir = typeof values["skills-dir"] === "string" ? values["skills-dir"] : undefined;
      const force = values.force === true;
      process.exit(runSkill(positionals[1], agentArg ?? "all", scopeArg ?? "personal", skillsDir, force, fmt));
      break;
    }
    default: {
      console.log(cliOutput(buildCliError(`unknown command: ${command}`, "valid commands: echo, ping, skill"), fmt));
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
    hermes: "hermes",
  };
  const agent = agents[agentArg];
  if (agent === undefined) {
    return { error: `invalid --agent '${agentArg}'`, hint: "valid values: all, codex, claude-code, opencode, hermes" };
  }
  const scopes: Record<string, SkillScope> = { personal: "personal", workspace: "workspace" };
  const scope = scopes[scopeArg];
  if (scope === undefined) {
    return { error: `invalid --scope '${scopeArg}'`, hint: "valid values: personal, workspace" };
  }
  return { options: { agent, scope, skillsDir, force } };
}

// ── Tests (run via: npx tsx --test examples/agent_cli.ts) ────────────────────

if (process.env["NODE_TEST_CONTEXT"]) {
  interface SecurityHelpDefaultCase {
    default: string;
    expected: string;
  }

  function securityHelpDefaultCase(): SecurityHelpDefaultCase {
    const fixturePath = join(
      dirname(fileURLToPath(import.meta.url)),
      "..",
      "..",
      "spec",
      "fixtures",
      "security.json"
    );
    const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as {
      help_default_cases: SecurityHelpDefaultCase[];
    };
    return fixture.help_default_cases[0];
  }

  describe("help", () => {
    it("root help is one-level", () => {
      const help = formatRootHelp();
      assert.ok(help.includes("echo"), "root --help must include echo");
      assert.ok(help.includes("ping"), "root --help must include ping");
      assert.ok(help.includes("--output"), "root --help must include --output");
      assert.ok(help.includes(`AFDATA: ${AFDATA_VERSION}`), "root --help must include AFDATA version");
      assert.ok(!help.includes("--help-all"), "root --help must not include removed --help-all");
      assert.ok(!help.includes("--dry-run"), "root --help must NOT include echo's --dry-run");
      assert.ok(!help.includes("--host"), "root --help must NOT include ping's --host");
      assert.ok(!help.includes("--stream"), "root --help must not include stream mode");
      assert.ok(!help.includes("--result-only"), "root --help must not include result-only mode");
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

    it("markdown about appears once — heading only, never repeated in the fenced block", () => {
      const root = formatMarkdownHelp(undefined, false);
      assert.equal(
        (root.match(/Minimal agent-first CLI example/g) ?? []).length,
        1,
        "root about must live in the heading, not also in the fenced help block",
      );
      const echo = formatMarkdownHelp("echo", false);
      assert.equal(
        (echo.match(/Echo back the input as structured output/g) ?? []).length,
        1,
        "subcommand about must live in the heading, not also in the fenced help block",
      );
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
      assert.deepEqual(schema.versions, { afdata: AFDATA_VERSION });
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

    it("redacts secret defaults in every help format", () => {
      const helpCase = securityHelpDefaultCase();
      assert.equal(helpCase.default, HELP_DEFAULT_API_KEY_SECRET);
      assert.equal(helpCase.expected, "***");
      for (const rendered of [
        formatRootHelp(),
        formatMarkdownHelp(undefined, false),
        renderHelpOutput(undefined, "json", true, false),
        renderHelpOutput(undefined, "yaml", true, false),
      ]) {
        assert.ok(rendered.includes(helpCase.expected));
        assert.ok(!rendered.includes(helpCase.default));
      }
    });
  });

  describe("agent_cli example", () => {
    it("parse output all variants", () => {
      assert.equal(cliParseOutput("json"), "json");
      assert.equal(cliParseOutput("yaml"), "yaml");
      assert.equal(cliParseOutput("plain"), "plain");
      assert.throws(() => cliParseOutput("xml"));
    });

    it("detects missing output values before falling back to json", () => {
      for (const args of [["--output"], ["--output", "--json"], ["--output="]]) {
        assert.equal(outputFlagMissing(args), true, `${args.join(" ")} must be missing`);
        assert.throws(() => resolveOutputArg(args), /missing value for --output/);
      }
      for (const args of [["--output", "json"], ["--output=json"], ["--json"]]) {
        assert.equal(outputFlagMissing(args), false, `${args.join(" ")} must not be missing`);
      }
    });

    it("strictly validates known flags, command flags, and positionals", () => {
      for (const args of [
        ["echo"],
        ["echo", "--dry-run"],
        ["ping", "--host", "example.com"],
        ["skill", "status", "--agent", "opencode"],
      ]) {
        assert.doesNotThrow(() => strictParseArgs(args));
      }
      for (const args of [
        ["--bogus", "echo"],
        ["--log"],
        ["echo", "--host", "example.com"],
        ["echo", "extra"],
        ["ping", "extra"],
        ["skill"],
        ["skill", "status", "extra"],
      ]) {
        assert.throws(() => strictParseArgs(args), Error, `${args.join(" ")} should fail strict parsing`);
      }
    });

    it("parse log normalizes", () => {
      const lf = cliParseLogFilters(["Startup", " REQUEST ", "startup"]);
      assert.deepEqual(
        Array.from(lf.filters),
        ["startup", "request"]
      );
    });

    it("log enabled honors all/* wildcards", () => {
      assert.ok(!logEnabled(cliParseLogFilters([]), "startup"));
      assert.ok(logEnabled(cliParseLogFilters(["startup"]), "startup"));
      assert.ok(!logEnabled(cliParseLogFilters(["startup"]), "request"));
      for (const everything of ["all", "*"]) {
        assert.ok(logEnabled(cliParseLogFilters([everything]), "startup"), `${everything} enables startup`);
        assert.ok(logEnabled(cliParseLogFilters([everything]), "request"), `${everything} enables request`);
      }
    });

    it("log lines are category-tagged", () => {
      const req = buildRequestLog(undefined) as Record<string, any>;
      assert.equal(req["kind"], "log");
      assert.equal(req["log"].category, "request");
      assert.equal(req["log"].command, "none");
      const raw = ["--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"];
      const start = buildStartupLog(raw, "ping", "yaml", cliParseLogFilters(["startup"]), false) as Record<string, any>;
      assert.equal(start["kind"], "log");
      assert.equal(start["log"].category, "startup");
      assert.deepEqual(start["log"].argv, ["--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"]);
      assert.deepEqual(start["log"].parsed, { command: "ping", output: "yaml", log: ["startup"], verbose: false });
      assert.deepEqual(start["log"].effective_config, { output: "yaml", log: ["startup"] });
      const env = start["log"].env as Array<Record<string, unknown>>;
      assert.equal(env.length, 1);
      assert.equal(env[0]?.key, PING_HOST_ENV);
      assert.equal(env[0]?.present, process.env[PING_HOST_ENV] !== undefined);
      if (process.env[PING_HOST_ENV] !== undefined) assert.equal(env[0]?.value, process.env[PING_HOST_ENV]);
    });

    it("build cli error structure", () => {
      const event = buildCliError("--output: invalid value 'xml'");
      const v = event.toJSON() as Record<string, unknown>;
      assert.equal(v["kind"], "error");
      assert.equal((v["error"] as Record<string, unknown>).code, "cli_error");
      assert.equal((v["error"] as Record<string, unknown>).message, "--output: invalid value 'xml'");
      assert.equal(v["error_code"], undefined);
      assert.equal(v["retryable"], undefined);
      assert.deepEqual(v["trace"], {});
    });

    it("build cli error with hint", () => {
      const event = buildCliError("unknown action: foo", "valid actions: echo, ping");
      const v = event.toJSON() as Record<string, unknown>;
      assert.equal(v["kind"], "error");
      assert.equal((v["error"] as Record<string, unknown>).hint, "valid actions: echo, ping");
    });

    it("build json error with hint", () => {
      const event = jsonError("not_configured", "not configured").hint("set PING_HOST").build();
      const v = event.toJSON() as Record<string, any>;
      assert.equal(v["kind"], "error");
      assert.equal(v["error"].code, "not_configured");
      assert.equal(v["error"].message, "not configured");
      assert.equal(v["error"].hint, "set PING_HOST");
    });

    it("build json error without hint has no hint key", () => {
      const event = jsonError("failed", "something failed").build();
      const v = event.toJSON() as Record<string, any>;
      assert.equal(v["error"].hint, undefined);
    });

    it("cli output all formats", () => {
      const event = jsonResult({ ok: true }).build();
      const jsonOut = cliOutput(event, "json");
      const yamlOut = cliOutput(event, "yaml");
      const plainOut = cliOutput(event, "plain");
      assert.ok(jsonOut.includes('"kind"'));
      assert.ok(yamlOut.startsWith("---"));
      assert.ok(plainOut.includes("kind=result"));
    });

    it("error round trip is valid jsonl", () => {
      const event = buildCliError("unknown flag: --foo");
      const line = outputJson(event.toJSON() as JsonValue);
      const parsed = JSON.parse(line);
      assert.equal(parsed.kind, "error");
      assert.equal(parsed.error.code, "cli_error");
      assert.ok(!line.includes("\n"));
    });
  });
}

// Only run main() when executed directly, not during `--test`
if (!process.env["NODE_TEST_CONTEXT"]) {
  main();
}
