import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  cliParseOutput,
  cliParseLogFilters,
  cliOutput,
  buildCliError,
  outputJson,
} from "./index.js";

// ── cliParseOutput ────────────────────────────────────────────────────────────

describe("cliParseOutput", () => {
  it("accepts all formats", () => {
    assert.equal(cliParseOutput("json"), "json");
    assert.equal(cliParseOutput("yaml"), "yaml");
    assert.equal(cliParseOutput("plain"), "plain");
  });

  it("rejects unknown values", () => {
    assert.throws(() => cliParseOutput("xml"));
    assert.throws(() => cliParseOutput("JSON"));
    assert.throws(() => cliParseOutput(""));
  });

  it("error message contains the invalid value", () => {
    try {
      cliParseOutput("toml");
      assert.fail("expected throw");
    } catch (e) {
      assert.ok(e instanceof Error);
      assert.ok(e.message.includes("toml"));
      assert.ok(e.message.includes("json"));
    }
  });
});

// ── cliParseLogFilters ────────────────────────────────────────────────────────

describe("cliParseLogFilters", () => {
  it("trims and lowercases", () => {
    assert.deepEqual(cliParseLogFilters(["  Query  ", "ERROR"]), ["query", "error"]);
  });

  it("deduplicates", () => {
    assert.deepEqual(
      cliParseLogFilters(["query", "error", "Query", "query"]),
      ["query", "error"]
    );
  });

  it("removes empty entries", () => {
    assert.deepEqual(cliParseLogFilters(["", "query", "  "]), ["query"]);
  });

  it("handles empty array", () => {
    assert.deepEqual(cliParseLogFilters([]), []);
  });

  it("preserves order", () => {
    assert.deepEqual(
      cliParseLogFilters(["startup", "request", "retry"]),
      ["startup", "request", "retry"]
    );
  });
});

// ── buildCliError ─────────────────────────────────────────────────────────────

describe("buildCliError", () => {
  it("has required fields", () => {
    const v = buildCliError("missing --sql") as Record<string, unknown>;
    assert.equal(v["code"], "error");
    assert.equal(v["error_code"], "invalid_request");
    assert.equal(v["error"], "missing --sql");
    assert.equal(v["retryable"], false);
    assert.equal((v["trace"] as Record<string, unknown>)["duration_ms"], 0);
  });

  it("produces valid JSONL", () => {
    const v = buildCliError("oops");
    const line = outputJson(v);
    const parsed = JSON.parse(line);
    assert.equal(parsed.code, "error");
    assert.ok(!line.includes("\n"));
  });
});

// ── cliOutput ─────────────────────────────────────────────────────────────────

describe("cliOutput", () => {
  it("dispatches json (raw keys, single line)", () => {
    const v = { code: "ok", size_bytes: 1024 };
    const out = cliOutput(v, "json");
    assert.ok(out.includes("size_bytes"));  // json: no suffix processing
    assert.ok(!out.includes("\n"));
  });

  it("dispatches yaml (suffix stripped)", () => {
    const v = { code: "ok", size_bytes: 1024 };
    const out = cliOutput(v, "yaml");
    assert.ok(out.startsWith("---"));
    assert.ok(out.includes("size:"));       // yaml: suffix stripped
  });

  it("dispatches plain (logfmt)", () => {
    const v = { code: "ok" };
    const out = cliOutput(v, "plain");
    assert.ok(!out.includes("\n"));
    assert.ok(out.includes("code=ok"));
  });
});
