import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  cliParseOutput,
  parseOutputTo,
  cliParseLogFilters,
  render,
  PlainStyle,
  CliEmitter,
  jsonError,
  buildCliError,
  buildCliVersion,
  cliRenderVersion,
  cliHandleVersionOrContinue,
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

// ── parseOutputTo ─────────────────────────────────────────────────────────────

describe("parseOutputTo", () => {
  it("accepts all selectors", () => {
    assert.equal(parseOutputTo("split"), "split");
    assert.equal(parseOutputTo("stdout"), "stdout");
    assert.equal(parseOutputTo("stderr"), "stderr");
  });

  it("rejects unknown values", () => {
    assert.throws(() => parseOutputTo("both"));
    assert.throws(() => parseOutputTo("SPLIT"));
    assert.throws(() => parseOutputTo(""));
  });

  it("error message contains the invalid value and the valid choices", () => {
    try {
      parseOutputTo("file");
      assert.fail("expected throw");
    } catch (e) {
      assert.ok(e instanceof Error);
      assert.ok(e.message.includes("file"));
      assert.ok(e.message.includes("split"));
      assert.ok(e.message.includes("stdout"));
      assert.ok(e.message.includes("stderr"));
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

  it("enabled() returns true for the 'all' wildcard, but '*' is not special", () => {
    const lf1 = cliParseLogFilters(["all"]);
    assert.equal(lf1.enabled("anything"), true);
    // "*" is a literal prefix now, not a wildcard.
    const lf2 = cliParseLogFilters(["*"]);
    assert.equal(lf2.enabled("anything"), false);
    assert.equal(lf2.enabled("*special"), true);
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
    const line = render(event.toJSON() as Record<string, unknown>, "json");
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

  it("never throws on an empty message", () => {
    // L1: buildCliError must never throw. An empty message is substituted
    // with a placeholder so the internal jsonError(...).build() cannot fail.
    const event = buildCliError("");
    const v = event.toJSON() as Record<string, unknown>;
    const err = v["error"] as Record<string, unknown>;
    assert.equal(v["kind"], "error");
    assert.equal(err["code"], "cli_error");
    assert.equal(err["message"], "unspecified error");
  });
});

// ── render ────────────────────────────────────────────────────────────────────

describe("render", () => {
  it("dispatches json (raw keys, single line)", () => {
    const v = jsonResult({ size_bytes: 1024 }).build();
    const out = render(v, "json");
    assert.ok(out.includes("size_bytes"));  // json: no suffix processing
    assert.ok(!out.includes("\n"));
  });

  it("dispatches yaml (structure-preserving, raw keys)", () => {
    const v = jsonResult({ size_bytes: 1024 }).build();
    const out = render(v, "yaml");
    assert.ok(out.startsWith("---"));
    assert.ok(out.includes("size_bytes: 1024"));  // yaml: no suffix processing
    assert.ok(!out.includes("size:"));
  });

  it("dispatches plain (logfmt)", () => {
    const v = jsonResult({ ok: true }).build();
    const out = render(v, "plain");
    assert.ok(!out.includes("\n"));
    assert.ok(out.includes("kind=result"));
  });

  it("dispatches raw yaml with output options", () => {
    const v = { size_bytes: 1024 };
    const out = render(v, "yaml", { style: PlainStyle.Raw });
    assert.ok(out.includes("size_bytes: 1024"));
    assert.ok(!out.includes("size:"));
  });
});

// ── CliEmitter ───────────────────────────────────────────────────────────────

describe("CliEmitter", () => {
  it("writes events and tracks terminal state", () => {
    const lines: string[] = [];
    const emitter = new CliEmitter((line) => lines.push(line), "json");
    emitter.emit(jsonLog({ level: "info", message: "startup", event: "startup" }).build());
    emitter.emit(jsonResult({ rows: 2 }).build());
    assert.equal(lines.length, 2);
    assert.ok(lines[0]!.includes('"kind":"log"'));
    assert.ok(lines[1]!.includes('"kind":"result"'));
  });

  it("frames every supported output format explicitly", () => {
    const events = [
      jsonLog({ level: "info", message: "startup", event: "startup" }).build(),
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
      () => emitter.emit(jsonProgress({ message: "working", percent: 100 }).build()),
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

  it("finite mode splits result to the primary sink and diagnostics to the diagnostic sink", () => {
    const out: string[] = [];
    const err: string[] = [];
    const emitter = CliEmitter.finiteWith(
      (line) => out.push(line),
      (line) => err.push(line),
      "json",
    );
    emitter.emit(jsonProgress({ message: "working", percent: 50 }).build());
    emitter.emit(jsonLog({ level: "info", message: "startup", event: "startup" }).build());
    emitter.emit(jsonResult({ rows: 2 }).build());
    // result → primary (stdout) sink only
    assert.equal(out.length, 1);
    assert.ok(out[0]!.includes('"kind":"result"'));
    // progress + log → diagnostic (stderr) sink
    assert.equal(err.length, 2);
    assert.deepEqual(err.map((line) => JSON.parse(line).kind), ["progress", "log"]);
  });

  it("finite mode routes error (not result) to the diagnostic sink", () => {
    const out: string[] = [];
    const err: string[] = [];
    const emitter = CliEmitter.finiteWith(
      (line) => out.push(line),
      (line) => err.push(line),
      "json",
    );
    emitter.emit(jsonError("boom", "it broke").build());
    assert.equal(out.length, 0);
    assert.equal(err.length, 1);
    assert.ok(err[0]!.includes('"kind":"error"'));
  });

  it("stream mode collapses every event onto the single writer", () => {
    const lines: string[] = [];
    const emitter = CliEmitter.stream((line) => lines.push(line), "json");
    emitter.emit(jsonProgress({ message: "working", percent: 50 }).build());
    emitter.emit(jsonLog({ level: "info", message: "startup", event: "startup" }).build());
    emitter.emit(jsonError("boom", "it broke").build());
    assert.equal(lines.length, 3);
    assert.deepEqual(lines.map((line) => JSON.parse(line).kind), ["progress", "log", "error"]);
  });

  it("the unified constructor is stream mode (no split)", () => {
    const lines: string[] = [];
    const emitter = new CliEmitter((line) => lines.push(line), "json");
    emitter.emit(jsonError("boom", "it broke").build());
    // error stays on the single writer — no diagnostic sink to divert it.
    assert.equal(lines.length, 1);
    assert.ok(lines[0]!.includes('"kind":"error"'));
  });
});

// ── CliEmitter finish helpers ─────────────────────────────────────────────────

describe("CliEmitter finish helpers", () => {
  it("finish returns the success code on a successful write", () => {
    const lines: string[] = [];
    const emitter = new CliEmitter((line) => lines.push(line), "json");
    assert.equal(emitter.finish(jsonResult({ ok: true }).build(), 0), 0);
    assert.equal(lines.length, 1);
    assert.ok(lines[0]!.includes('"kind":"result"'));
  });

  it("finish returns the caller's success code even for an error event (exit code is the caller's)", () => {
    const lines: string[] = [];
    const emitter = new CliEmitter((line) => lines.push(line), "json");
    assert.equal(emitter.finish(jsonError("boom", "it broke").build(), 1), 1);
    assert.ok(lines[0]!.includes('"kind":"error"'));
  });

  it("finishResult writes a result to the primary sink and returns 0", () => {
    const out: string[] = [];
    const err: string[] = [];
    const emitter = CliEmitter.finiteWith((l) => out.push(l), (l) => err.push(l), "json");
    assert.equal(emitter.finishResult({ rows: 3 }), 0);
    assert.equal(out.length, 1);
    assert.equal(err.length, 0);
    assert.equal(JSON.parse(out[0]!).result.rows, 3);
  });

  it("finish routes a builder-built error (with hint) to the diagnostic sink and returns the exit code", () => {
    // Errors go through the builder — the builder is the error "type" — then
    // finish(event, exitCode). No finishError shortcut.
    const out: string[] = [];
    const err: string[] = [];
    const emitter = CliEmitter.finiteWith((l) => out.push(l), (l) => err.push(l), "json");
    const event = jsonError("ping_failed", "no route").hint("check --host").build();
    assert.equal(emitter.finish(event, 1), 1);
    assert.equal(out.length, 0);
    assert.equal(err.length, 1);
    const parsed = JSON.parse(err[0]!);
    assert.equal(parsed.kind, "error");
    assert.equal(parsed.error.code, "ping_failed");
    assert.equal(parsed.error.message, "no route");
    assert.equal(parsed.error.hint, "check --host");
  });

  it("finish maps a broken pipe (EPIPE) to exit code 0", () => {
    const emitter = new CliEmitter(() => {
      const e = new Error("write EPIPE") as NodeJS.ErrnoException;
      e.code = "EPIPE";
      throw e;
    }, "json");
    assert.equal(emitter.finish(jsonResult({ ok: true }).build(), 0), 0);
  });

  it("finish maps any other write failure to exit code 4", () => {
    const emitter = new CliEmitter(() => {
      throw new Error("disk full");
    }, "json");
    assert.equal(emitter.finish(jsonResult({ ok: true }).build(), 0), 4);
  });
});

// ── version helpers ──────────────────────────────────────────────────────────

describe("version helpers", () => {
  it("builds the standard version shape", () => {
    const event = buildCliVersion("agent-cli", "Agent CLI Example", "1.2.3", "abc1234");
    const v = event.toJSON() as Record<string, unknown>;
    const result = v["result"] as Record<string, unknown>;
    assert.equal(v["kind"], "result");
    assert.equal(result["code"], "version");
    assert.equal(result["name"], "agent-cli");
    assert.equal(result["display_name"], "Agent CLI Example");
    assert.equal(result["version"], "1.2.3");
    assert.equal(result["build"], "abc1234");
    assert.deepEqual(v["trace"], {});
  });

  it("omits absent display_name and build", () => {
    const event = buildCliVersion("agent-cli", undefined, "1.2.3", undefined);
    const result = (event.toJSON() as Record<string, unknown>)["result"] as Record<string, unknown>;
    assert.equal(result["name"], "agent-cli");
    assert.equal(result["version"], "1.2.3");
    assert.ok(!("display_name" in result));
    assert.ok(!("build" in result));
  });

  it("renders a protocol-v1 event as JSON", () => {
    const out = cliRenderVersion("agent-cli", "Agent CLI Example", "1.2.3", undefined, "json");
    assert.ok(out.endsWith("\n"));
    const parsed = JSON.parse(out.trim());
    assert.equal(parsed.kind, "result");
    assert.equal(parsed.result.code, "version");
    assert.equal(parsed.result.name, "agent-cli");
    assert.equal(parsed.result.display_name, "Agent CLI Example");
    assert.equal(parsed.result.version, "1.2.3");
    assert.ok(!("build" in parsed.result));
  });

  it("bare --version defaults to a JSON version event", () => {
    // The one blessed behavior: `--version` always answers with a protocol-v1
    // event, JSON by default — no conventional bare-text special case.
    const out = cliHandleVersionOrContinue(["--version"], [], "agent-cli", "Agent CLI Example", "1.2.3", undefined);
    assert.ok(out !== undefined);
    const parsed = JSON.parse(out!.trim());
    assert.equal(parsed.kind, "result");
    assert.equal(parsed.result.code, "version");
    assert.equal(parsed.result.name, "agent-cli");
    assert.equal(parsed.result.display_name, "Agent CLI Example");
    assert.equal(parsed.result.version, "1.2.3");
  });

  it("honors explicit output flags", () => {
    const out = cliHandleVersionOrContinue(["--version", "--output", "plain"], [], "agent-cli", undefined, "1.2.3", undefined);
    assert.ok(out?.includes("kind=result"));
    assert.ok(out?.includes("result.version=1.2.3"));
  });

  it("supports --json as --output json", () => {
    const out = cliHandleVersionOrContinue(["--version", "--json"], [], "agent-cli", undefined, "1.2.3", undefined);
    assert.ok(out?.includes('"kind":"result"'));
    assert.ok(out?.includes('"version":"1.2.3"'));
  });

  it("rejects --json with another explicit output format", () => {
    assert.throws(
      () => cliHandleVersionOrContinue(["--version", "--json", "--output", "yaml"], [], "agent-cli", undefined, "1.2.3", undefined),
      /conflicting output formats/
    );
  });

  it("returns undefined without a version flag", () => {
    assert.equal(cliHandleVersionOrContinue(["ping"], [], "agent-cli", undefined, "1.2.3", undefined), undefined);
  });

  it("rejects invalid output values", () => {
    assert.throws(
      () => cliHandleVersionOrContinue(["--version", "--output", "xml"], [], "agent-cli", undefined, "1.2.3", undefined),
      /xml/
    );
  });

  it("ignores a --version flag after a subcommand", () => {
    // A subcommand that takes its own --version <value> must not be hijacked
    // by the top-level pre-parser.
    assert.equal(cliHandleVersionOrContinue(["hatch", "--version", "1.3.0"], [], "agent-cli", undefined, "1.2.3", undefined), undefined);
    assert.equal(cliHandleVersionOrContinue(["hatch", "-V", "1.3.0"], [], "agent-cli", undefined, "1.2.3", undefined), undefined);
  });

  it("still honors a top-level --version after a value-consuming output flag", () => {
    const out = cliHandleVersionOrContinue(["--output", "json", "--version"], [], "agent-cli", undefined, "1.2.3", undefined);
    assert.ok(out?.includes('"version":"1.2.3"'));
  });

  it("recognizes --version after a caller-defined value flag", () => {
    // A consumer's own value-taking global flag (here a comma-list `--log`)
    // must have its space-separated value recognized via `valueFlags`, not a
    // hardcoded flag list; otherwise `a,b` would be mistaken for the subcommand
    // boundary and `--version` would be dropped.
    const out = cliHandleVersionOrContinue(["--log", "a,b", "--version"], ["--log"], "hypha", undefined, "1.2.3", undefined);
    assert.ok(out !== undefined);
    const parsed = JSON.parse(out!.trim());
    assert.equal(parsed.result.name, "hypha");
    assert.equal(parsed.result.version, "1.2.3");
  });

  it("does not over-consume an unlisted flag's following positional", () => {
    // A caller flag NOT in `valueFlags` takes no value, so the following
    // positional is the subcommand boundary; a later `--version` belongs to it.
    assert.equal(
      cliHandleVersionOrContinue(["--verbose", "sense", "--version"], ["--log"], "hypha", undefined, "1.2.3", undefined),
      undefined,
    );
  });

  it("skips an --output-to space value before --version", () => {
    // `--output-to <value>` takes a value that must not be mistaken for the
    // subcommand boundary; the later `--version` must still be detected.
    const out = cliHandleVersionOrContinue(["--output-to", "stdout", "--version"], [], "agent-cli", undefined, "1.2.3", undefined);
    assert.ok(out !== undefined);
    const parsed = JSON.parse(out!.trim());
    assert.equal(parsed.result.name, "agent-cli");
    assert.equal(parsed.result.version, "1.2.3");
  });
});
