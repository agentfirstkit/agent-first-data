import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  cliParseOutput,
  cliParseLogFilters,
  cliOutput,
  OutputStyle,
  CliEmitter,
  jsonError,
  buildCliError,
  buildCliVersion,
  cliRenderVersion,
  cliHandleVersionOrContinue,
  outputJson,
  jsonLog,
  jsonProgress,
  jsonResult,
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
    const lf = cliParseLogFilters(["  Query  ", "ERROR"]);
    assert.deepEqual(Array.from(lf.filters), ["query", "error"]);
  });

  it("deduplicates", () => {
    const lf = cliParseLogFilters(["query", "error", "Query", "query"]);
    assert.deepEqual(Array.from(lf.filters), ["query", "error"]);
  });

  it("removes empty entries", () => {
    const lf = cliParseLogFilters(["", "query", "  "]);
    assert.deepEqual(Array.from(lf.filters), ["query"]);
  });

  it("handles empty array", () => {
    const lf = cliParseLogFilters([]);
    assert.deepEqual(Array.from(lf.filters), []);
  });

  it("preserves order", () => {
    const lf = cliParseLogFilters(["startup", "request", "retry"]);
    assert.deepEqual(Array.from(lf.filters), ["startup", "request", "retry"]);
  });

  it("enabled() returns false for empty filters", () => {
    const lf = cliParseLogFilters([]);
    assert.equal(lf.enabled("query"), false);
  });

  it("enabled() returns true for 'all' or '*'", () => {
    const lf1 = cliParseLogFilters(["all"]);
    assert.equal(lf1.enabled("anything"), true);
    const lf2 = cliParseLogFilters(["*"]);
    assert.equal(lf2.enabled("anything"), true);
  });

  it("enabled() checks prefix match case-insensitively", () => {
    const lf = cliParseLogFilters(["query", "error"]);
    assert.equal(lf.enabled("QueryStarted"), true);
    assert.equal(lf.enabled("ERROR_BAD"), true);
    assert.equal(lf.enabled("debug"), false);
    assert.equal(lf.enabled("retry"), false);
  });
});

// ── buildCliError ─────────────────────────────────────────────────────────────

describe("buildCliError", () => {
  it("has required fields", () => {
    const event = buildCliError("missing --sql");
    const v = event.toJSON() as Record<string, unknown>;
    assert.equal(v["kind"], "error");
    assert.equal((v["error"] as Record<string, unknown>)["code"], "cli_error");
    assert.equal((v["error"] as Record<string, unknown>)["message"], "missing --sql");
    assert.equal((v["error"] as Record<string, unknown>)["retryable"], false);
    assert.equal(v["error_code"], undefined);
    assert.equal(v["retryable"], undefined);
    assert.deepEqual(v["trace"], {});
  });

  it("produces valid JSONL", () => {
    const event = buildCliError("oops");
    const line = outputJson(event.toJSON() as Record<string, unknown>);
    const parsed = JSON.parse(line);
    assert.equal(parsed.kind, "error");
    assert.equal(parsed.error.code, "cli_error");
    assert.ok(!line.includes("\n"));
  });

  it("includes hint when provided", () => {
    const event = buildCliError("bad flag", "try --help");
    const v = event.toJSON() as Record<string, unknown>;
    assert.equal((v["error"] as Record<string, unknown>)["hint"], "try --help");
  });

  it("omits hint key when not provided", () => {
    const event = buildCliError("oops");
    const v = event.toJSON() as Record<string, unknown>;
    const err = v["error"] as Record<string, unknown>;
    assert.equal(err["hint"], undefined);
    assert.ok(!("hint" in err));
  });
});

// ── cliOutput ─────────────────────────────────────────────────────────────────

describe("cliOutput", () => {
  it("dispatches json (raw keys, single line)", () => {
    const v = jsonResult({ size_bytes: 1024 }).build();
    const out = cliOutput(v, "json");
    assert.ok(out.includes("size_bytes"));  // json: no suffix processing
    assert.ok(!out.includes("\n"));
  });

  it("dispatches yaml (suffix stripped)", () => {
    const v = jsonResult({ size_bytes: 1024 }).build();
    const out = cliOutput(v, "yaml");
    assert.ok(out.startsWith("---"));
    assert.ok(out.includes("size:"));       // yaml: suffix stripped
  });

  it("dispatches plain (logfmt)", () => {
    const v = jsonResult({ ok: true }).build();
    const out = cliOutput(v, "plain");
    assert.ok(!out.includes("\n"));
    assert.ok(out.includes("kind=result"));
  });

  it("dispatches raw yaml with output options", () => {
    const v = { size_bytes: 1024 };
    const out = cliOutput(v, "yaml", { style: OutputStyle.Raw });
    assert.ok(out.includes("size_bytes: 1024"));
    assert.ok(!out.includes("size:"));
  });
});

// ── CliEmitter ───────────────────────────────────────────────────────────────

describe("CliEmitter", () => {
  it("writes events and tracks terminal state", () => {
    const lines: string[] = [];
    const emitter = new CliEmitter((line) => lines.push(line), "json");
    emitter.emit(jsonLog("info", "startup").field("event", "startup").build());
    emitter.emit(jsonResult({ rows: 2 }).build());
    assert.equal(lines.length, 2);
    assert.ok(lines[0]!.includes('"kind":"log"'));
    assert.ok(lines[1]!.includes('"kind":"result"'));
  });

  it("frames every supported output format explicitly", () => {
    const events = [
      jsonLog("info", "startup").field("event", "startup").build(),
      jsonResult({ rows: 2 }).build(),
    ];
    for (const format of ["json", "plain", "yaml"] as const) {
      const lines: string[] = [];
      const emitter = new CliEmitter((line) => lines.push(line), format);
      for (const event of events) emitter.emit(event);
      assert.equal(lines.length, 2);
      if (format === "json") {
        assert.deepEqual(lines.map((line) => JSON.parse(line).kind), ["log", "result"]);
      } else if (format === "plain") {
        assert.ok(lines[0]!.startsWith("kind=log"));
        assert.ok(lines[1]!.startsWith("kind=result"));
      } else {
        assert.ok(lines.every((line) => line.startsWith("---\n")));
      }
    }
  });

  it("rejects duplicate terminal events", () => {
    const emitter = new CliEmitter(() => undefined, "json");
    emitter.emit(jsonResult({ rows: 2 }).build());
    assert.throws(
      () => emitter.emit(jsonError("late_error", "too late").build()),
      /duplicate terminal/
    );
  });

  it("rejects non-terminal events after terminal", () => {
    const emitter = new CliEmitter(() => undefined, "json");
    emitter.emit(jsonResult({ rows: 2 }).build());
    assert.throws(
      () => emitter.emit(jsonProgress("working").field("percent", 100).build()),
      /after terminal/
    );
  });

  it("returns writer errors", () => {
    const emitter = new CliEmitter(() => {
      throw new Error("closed");
    }, "json");
    assert.throws(() => emitter.emit(jsonResult({ rows: 2 }).build()), /closed/);
  });

  it("does not commit terminal state when the writer fails", () => {
    const lines: string[] = [];
    let failed = false;
    const emitter = new CliEmitter((line) => {
      if (!failed) {
        failed = true;
        throw new Error("retry");
      }
      lines.push(line);
    }, "json");
    const event = jsonResult({ rows: 2 }).build();
    assert.throws(() => emitter.emit(event), /retry/);
    emitter.emit(event);
    assert.equal(lines.length, 1);
  });
});

// ── version helpers ──────────────────────────────────────────────────────────

describe("version helpers", () => {
  it("builds the standard version shape", () => {
    const event = buildCliVersion("1.2.3");
    const v = event.toJSON() as Record<string, unknown>;
    assert.equal(v["kind"], "result");
    assert.equal((v["result"] as Record<string, unknown>)["version"], "1.2.3");
    assert.deepEqual(v["trace"], {});
  });

  it("renders conventional bare text by default", () => {
    assert.equal(cliRenderVersion("agent-cli", "1.2.3"), "agent-cli 1.2.3\n");
  });

  it("can render JSON", () => {
    const out = cliRenderVersion("agent-cli", "1.2.3", "json");
    assert.ok(out.endsWith("\n"));
    assert.ok(out.includes('"kind":"result"'));
    assert.ok(out.includes('"version":"1.2.3"'));
  });

  it("uses conventional bare text by default", () => {
    assert.equal(cliHandleVersionOrContinue(["--version"], "agent-cli", "1.2.3"), "agent-cli 1.2.3\n");
  });

  it("honors explicit output flags", () => {
    const out = cliHandleVersionOrContinue(["--version", "--output", "plain"], "agent-cli", "1.2.3");
    assert.ok(out?.includes("kind=result"));
    assert.ok(out?.includes("result.version=1.2.3"));
  });

  it("supports --json as --output json", () => {
    const out = cliHandleVersionOrContinue(["--version", "--json"], "agent-cli", "1.2.3");
    assert.ok(out?.includes('"kind":"result"'));
    assert.ok(out?.includes('"version":"1.2.3"'));
  });

  it("rejects --json with another explicit output format", () => {
    assert.throws(
      () => cliHandleVersionOrContinue(["--version", "--json", "--output", "yaml"], "agent-cli", "1.2.3"),
      /conflicting output formats/
    );
  });

  it("returns undefined without a version flag", () => {
    assert.equal(cliHandleVersionOrContinue(["ping"], "agent-cli", "1.2.3"), undefined);
  });

  it("rejects invalid output values", () => {
    assert.throws(
      () => cliHandleVersionOrContinue(["--version", "--output", "xml"], "agent-cli", "1.2.3"),
      /xml/
    );
  });
});
