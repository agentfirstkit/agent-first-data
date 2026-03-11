/**
 * Tests for AFDATA output formatting — driven by shared spec/fixtures.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildJsonOk,
  buildJsonError,
  buildJson,
  internalRedactSecrets,
  RedactionPolicy,
  outputJson,
  outputJsonWith,
  outputYaml,
  outputPlain,
  type JsonValue,
  parseSize,
} from "./format.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = join(__dirname, "..", "..", "spec", "fixtures");

function load(name: string): any[] {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf-8"));
}

// --- Redact fixtures ---

describe("redact fixtures", () => {
  for (const tc of load("redact.json")) {
    it(tc.name, () => {
      const inp = JSON.parse(JSON.stringify(tc.input));
      internalRedactSecrets(inp);
      assert.deepEqual(inp, tc.expected);
    });
  }
});

// --- Protocol fixtures ---

describe("protocol fixtures", () => {
  for (const tc of load("protocol.json")) {
    it(tc.name, () => {
      let result: any;
      const args = tc.args;
      switch (tc.type) {
        case "ok": result = buildJsonOk(args.result); break;
        case "ok_trace": result = buildJsonOk(args.result, args.trace); break;
        case "error": result = buildJsonError(args.message); break;
        case "error_trace": result = buildJsonError(args.message, undefined, args.trace); break;
        case "error_hint": result = buildJsonError(args.message, args.hint); break;
        case "error_hint_trace": result = buildJsonError(args.message, args.hint, args.trace); break;
        case "status": result = buildJson(args.code, args.fields); break;
        default: throw new Error(`unknown type: ${tc.type}`);
      }
      if (tc.expected) {
        assert.deepEqual(result, tc.expected);
      }
      if (tc.expected_contains) {
        for (const [k, v] of Object.entries(tc.expected_contains)) {
          assert.deepEqual(result[k], v, `key ${k}`);
        }
      }
    });
  }
});

// --- Helper fixtures ---

describe("helper fixtures", () => {
  for (const tc of load("helpers.json")) {
    if (tc.name === "format_bytes_human") {
      for (const [input, expected] of tc.cases) {
        it(`bytes ${input} → ${expected}`, () => {
          const plain = outputPlain({ size_bytes: input });
          assert.ok(plain.includes(`size=${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "format_with_commas") {
      for (const [input, expected] of tc.cases) {
        it(`commas ${input} → ${expected}`, () => {
          const plain = outputPlain({ price_jpy: input });
          assert.ok(plain.includes(`price=\u00a5${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "extract_currency_code") {
      for (const [input, expected] of tc.cases) {
        it(`currency code ${input}`, () => {
          // Test via outputPlain: valid codes strip key, null keeps it
          const plain = outputPlain({ [input]: 100 });
          if (expected === null) {
            assert.ok(plain.includes(`${input}=100`), `got ${plain}`);
          } else {
            assert.ok(!plain.includes(`${input}=`), `expected key stripped, got ${plain}`);
          }
        });
      }
    }
    if (tc.name === "parse_size") {
      for (const [input, expected] of tc.cases) {
        it(`parseSize ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(parseSize(input), expected);
        });
      }
    }
  }
});

describe("output format fixtures", () => {
  for (const tc of load("output_formats.json")) {
    it(tc.name, () => {
      const input = tc.input as JsonValue;

      const gotJson = JSON.parse(outputJson(input));
      assert.deepEqual(gotJson, tc.expected_json, "json mismatch");

      const gotYaml = outputYaml(input);
      assert.equal(gotYaml, tc.expected_yaml, "yaml mismatch");

      const gotPlain = outputPlain(input);
      assert.equal(gotPlain, tc.expected_plain, "plain mismatch");
    });
  }
});

describe("parseSize safety", () => {
  it("returns null for unsafe integers", () => {
    assert.equal(parseSize("9007199254740993"), null);
    assert.equal(parseSize("9007199254740992"), null);
    assert.equal(parseSize("8796093022208K"), null);
  });
});

describe("json safety", () => {
  it("serializes Error fields as readable strings", () => {
    const out = outputJson({ error: new Error("timeout") } as unknown as JsonValue);
    const parsed = JSON.parse(out) as Record<string, unknown>;
    assert.equal(parsed["error"], "timeout");
  });

  it("handles bigint and circular references without leaking secrets", () => {
    const meta: Record<string, unknown> = { api_key_secret: "sk-live-123", amount: 1n };
    meta["self"] = meta;

    const out = outputJson({ meta } as unknown as JsonValue);
    assert.ok(!out.includes("sk-live-123"), `secret leaked in output: ${out}`);

    const parsed = JSON.parse(out) as Record<string, unknown>;
    const parsedMeta = parsed["meta"] as Record<string, unknown>;
    assert.equal(parsedMeta["api_key_secret"], "***");
    assert.equal(parsedMeta["amount"], "<unsupported:bigint>");
    assert.equal(parsedMeta["self"], "<unsupported:circular>");
  });

  it("outputJsonWith RedactionTraceOnly keeps result secrets", () => {
    const out = outputJsonWith({
      code: "ok",
      result: { api_key_secret: "sk-live-123" },
      trace: { request_secret: "top-secret" },
    } as unknown as JsonValue, RedactionPolicy.RedactionTraceOnly);
    const parsed = JSON.parse(out) as Record<string, unknown>;
    const parsedResult = parsed["result"] as Record<string, unknown>;
    const parsedTrace = parsed["trace"] as Record<string, unknown>;
    assert.equal(parsedTrace["request_secret"], "***");
    assert.equal(parsedResult["api_key_secret"], "sk-live-123");
  });

  it("outputJsonWith RedactionNone keeps all secrets", () => {
    const out = outputJsonWith(
      { api_key_secret: "sk-live-123" } as unknown as JsonValue,
      RedactionPolicy.RedactionNone,
    );
    const parsed = JSON.parse(out) as Record<string, unknown>;
    assert.equal(parsed["api_key_secret"], "sk-live-123");
  });
});
