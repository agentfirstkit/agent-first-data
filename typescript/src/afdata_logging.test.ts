/**
 * Tests for AFDATA logging module.
 */

import { describe, it, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { log, span, initJson, initPlain, initYaml } from "./afdata_logging.ts";

// Capture stdout writes
let captured: string[] = [];
const originalWrite = process.stdout.write;

function startCapture() {
  captured = [];
  process.stdout.write = ((chunk: string | Uint8Array) => {
    captured.push(typeof chunk === "string" ? chunk : new TextDecoder().decode(chunk));
    return true;
  }) as typeof process.stdout.write;
}

function stopCapture() {
  process.stdout.write = originalWrite;
}

function lastLine(): Record<string, unknown> {
  const lines = captured.filter(l => l.trim());
  assert.ok(lines.length > 0, "No output captured");
  return JSON.parse(lines[lines.length - 1]);
}

describe("afdata_logging", () => {
  beforeEach(() => startCapture());
  afterEach(() => stopCapture());

  describe("basic fields", () => {
    it("outputs timestamp_epoch_ms, message, code", () => {
      log.info("hello world");
      const m = lastLine();
      assert.equal(m["message"], "hello world");
      assert.equal(m["code"], "info");
      assert.equal(typeof m["timestamp_epoch_ms"], "number");
    });

    it("maps warn level", () => {
      log.warn("caution");
      assert.equal(lastLine()["code"], "warn");
    });

    it("maps error level", () => {
      log.error("failure");
      assert.equal(lastLine()["code"], "error");
    });

    it("maps debug level", () => {
      log.debug("verbose");
      assert.equal(lastLine()["code"], "debug");
    });

    it("maps trace level", () => {
      log.trace("finest");
      assert.equal(lastLine()["code"], "trace");
    });
  });

  describe("event fields", () => {
    it("includes structured fields", () => {
      log.info("request", { domain: "example.com", status: 200 });
      const m = lastLine();
      assert.equal(m["domain"], "example.com");
      assert.equal(m["status"], 200);
    });

    it("allows code override", () => {
      log.info("ready", { code: "log", event: "startup" });
      const m = lastLine();
      assert.equal(m["code"], "log");
      assert.equal(m["event"], "startup");
    });
  });

  describe("span", () => {
    it("adds fields to log events", () => {
      span({ request_id: "abc-123" }, () => {
        log.info("processing");
      });
      const m = lastLine();
      assert.equal(m["request_id"], "abc-123");
      assert.equal(m["message"], "processing");
    });

    it("nests spans", () => {
      span({ request_id: "outer" }, () => {
        span({ step: "inner" }, () => {
          log.info("nested");
        });
      });
      const m = lastLine();
      assert.equal(m["request_id"], "outer");
      assert.equal(m["step"], "inner");
    });

    it("inner span overrides parent", () => {
      span({ source: "parent" }, () => {
        span({ source: "child" }, () => {
          log.info("test");
        });
      });
      assert.equal(lastLine()["source"], "child");
    });

    it("removes span fields after exit", () => {
      span({ request_id: "temp" }, () => {
        log.info("inside");
      });
      captured = [];
      log.info("outside");
      const m = lastLine();
      assert.equal(m["request_id"], undefined);
    });

    it("works with async functions", async () => {
      await span({ request_id: "async-1" }, async () => {
        await new Promise(r => setTimeout(r, 1));
        log.info("async log");
      });
      assert.equal(lastLine()["request_id"], "async-1");
    });
  });

  describe("plain format", () => {
    it("outputs logfmt", () => {
      initPlain();
      log.info("hello");
      const raw = captured.join("");
      assert.ok(raw.includes("message="), `expected logfmt, got: ${raw}`);
      assert.ok(raw.includes("code=info"), `expected code=info, got: ${raw}`);
      initJson(); // restore
    });
  });

  describe("yaml format", () => {
    it("outputs yaml with separator", () => {
      initYaml();
      log.info("hello");
      const raw = captured.join("");
      assert.ok(raw.startsWith("---"), `expected yaml separator, got: ${raw}`);
      initJson(); // restore
    });
  });
});
