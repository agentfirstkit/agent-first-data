/**
 * Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
 *
 * Demonstrates the correct use of: cliParseOutput, cliParseLogFilters,
 * cliOutput, and buildCliError.
 *
 * Run:  npx tsx examples/agent_cli.ts echo --output json
 *       npx tsx examples/agent_cli.ts echo --output yaml --log startup,request
 * Test: npx tsx --test examples/agent_cli.ts
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  type OutputFormat,
  buildCliError,
  cliOutput,
  cliParseLogFilters,
  cliParseOutput,
  outputJson,
} from "../src/index.js";

function main(): void {
  const args = process.argv.slice(2);
  const outputIdx = args.indexOf("--output");
  const logIdx = args.indexOf("--log");
  const action = args.find((a) => !a.startsWith("--")) ?? "echo";
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

  // Step 3: do work, emit JSONL
  const result = { code: "ok", action, log };
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
