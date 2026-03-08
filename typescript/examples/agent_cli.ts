/**
 * Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
 *
 * Demonstrates the correct use of: cliParseOutput, cliParseLogFilters,
 * cliOutput, buildCliError, --dry-run, and error hints.
 *
 * Run:  npx tsx examples/agent_cli.ts echo --output json
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

const VALID_ACTIONS = ["echo", "ping"];

function main(): void {
  const args = process.argv.slice(2);
  const outputIdx = args.indexOf("--output");
  const logIdx = args.indexOf("--log");
  const dryRun = args.includes("--dry-run");
  const action = args.find((a) => !a.startsWith("--") && args[args.indexOf(a) - 1] !== "--output" && args[args.indexOf(a) - 1] !== "--log") ?? "echo";
  const outputArg = outputIdx !== -1 ? args[outputIdx + 1] : "json";
  const logArg = logIdx !== -1 ? args[logIdx + 1] : "";

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

  // Step 3: validate action — demonstrate buildCliError with hint
  if (!VALID_ACTIONS.includes(action)) {
    console.log(outputJson(buildCliError(`unknown action: ${action}`, `valid actions: ${VALID_ACTIONS.join(", ")}`)));
    process.exit(2);
  }

  // Step 4: --dry-run → preview without executing
  if (dryRun) {
    const preview = buildJson("dry_run", { action, log }, { duration_ms: 0 });
    console.log(cliOutput(preview, fmt));
    return;
  }

  // Step 5: do work — demonstrate buildJsonError with hint on failure
  if (action === "ping") {
    const err = buildJsonError("ping target not configured", "set PING_HOST or pass --host", { duration_ms: 0 });
    console.log(cliOutput(err, fmt));
    process.exit(1);
  }

  const result = buildJsonOk({ action, log });
  console.log(cliOutput(result, fmt));
}

// ── Tests (run via: npx tsx --test examples/agent_cli.ts) ────────────────────

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
