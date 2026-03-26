/**
 * Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
 *
 * Demonstrates: complete --help (all subcommands in one output),
 * cliParseOutput, cliParseLogFilters, cliOutput, buildCliError,
 * --dry-run, and error hints.
 *
 * Run:  npx tsx examples/agent_cli.ts --help
 *       npx tsx examples/agent_cli.ts echo --help
 *       npx tsx examples/agent_cli.ts echo --output json
 *       npx tsx examples/agent_cli.ts echo --dry-run --output yaml
 *       npx tsx examples/agent_cli.ts ping --output json
 *       npx tsx examples/agent_cli.ts echo --output yaml --log startup,request
 * Test: npx tsx --test examples/agent_cli.ts
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  type OutputFormat,
  buildCliError,
  buildJson,
  buildJsonError,
  buildJsonOk,
  cliOutput,
  cliParseLogFilters,
  cliParseOutput,
  outputJson,
} from "../src/index.js";

interface Subcommand {
  name: string;
  about: string;
  flags: string;
}

const SUBCOMMANDS: Subcommand[] = [
  { name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing" },
  { name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping" },
];

/** Format help for root command and all subcommands. */
function formatCompleteHelp(): string {
  const lines = [
    "agent-cli — Minimal agent-first CLI example",
    "",
    "Usage: agent-cli [OPTIONS] <COMMAND>",
    "",
    "Options:",
    "  --output <FORMAT>  Output format: json, yaml, plain (default: json)",
    "  --log <FILTERS>    Log categories (comma-separated)",
    "  --help             Show this help",
    "",
    "Commands:",
  ];
  for (const sc of SUBCOMMANDS) {
    lines.push(`  ${sc.name.padEnd(8)} ${sc.about}`);
  }
  for (const sc of SUBCOMMANDS) {
    lines.push("", "=".repeat(60), `agent-cli ${sc.name}`, "=".repeat(60));
    lines.push(sc.about, "", "Flags:", sc.flags);
  }
  return lines.join("\n") + "\n";
}

/** Format help scoped to a single subcommand. */
function formatSubcommandHelp(name: string): string {
  const sc = SUBCOMMANDS.find((s) => s.name === name);
  if (!sc) return "";
  return `agent-cli ${sc.name} — ${sc.about}\n\nFlags:\n${sc.flags}\n`;
}

function main(): void {
  const args = process.argv.slice(2);
  const showHelp = args.includes("--help") || args.includes("-h");
  const argsWithoutHelp = args.filter((a) => a !== "--help" && a !== "-h");
  const command = argsWithoutHelp.find((a) => !a.startsWith("--") && argsWithoutHelp[argsWithoutHelp.indexOf(a) - 1] !== "--output" && argsWithoutHelp[argsWithoutHelp.indexOf(a) - 1] !== "--log");

  // Complete help: --help expands all subcommands in one output.
  // Subcommand --help expands only that subcommand.
  if (showHelp) {
    if (command) {
      process.stdout.write(formatSubcommandHelp(command));
    } else {
      process.stdout.write(formatCompleteHelp());
    }
    return;
  }

  const outputIdx = args.indexOf("--output");
  const logIdx = args.indexOf("--log");
  const dryRun = args.includes("--dry-run");
  const hostIdx = args.indexOf("--host");
  const outputArg = outputIdx !== -1 ? args[outputIdx + 1] : "json";
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
    default: {
      console.log(outputJson(buildCliError(`unknown command: ${command}`, "valid commands: echo, ping")));
      process.exit(2);
    }
  }
}

// ── Tests (run via: npx tsx --test examples/agent_cli.ts) ────────────────────

describe("complete help", () => {
  it("root help contains all subcommands", () => {
    const help = formatCompleteHelp();
    assert.ok(help.includes("echo"), "root --help must include echo");
    assert.ok(help.includes("ping"), "root --help must include ping");
    assert.ok(help.includes("--output"), "root --help must include --output");
    assert.ok(help.includes("--dry-run"), "root --help must include echo's --dry-run");
    assert.ok(help.includes("--host"), "root --help must include ping's --host");
  });

  it("subcommand help scoped to subtree", () => {
    const echoHelp = formatSubcommandHelp("echo");
    assert.ok(echoHelp.includes("--dry-run"), "echo --help must include --dry-run");
    assert.ok(!echoHelp.includes("--host"), "echo --help must NOT include ping's --host");
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

  it("build cli error structure", () => {
    const v = buildCliError("--output: invalid value 'xml'") as Record<string, unknown>;
    assert.equal(v["code"], "error");
    assert.equal(v["error_code"], "invalid_request");
    assert.equal(v["retryable"], false);
    assert.equal((v["trace"] as Record<string, unknown>)["duration_ms"], 0);
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

// Only run main() when executed directly, not during `--test`
if (!process.env["NODE_TEST_CONTEXT"]) {
  main();
}
