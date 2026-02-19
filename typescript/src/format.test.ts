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
  outputPlain,
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
        case "error_trace": result = buildJsonError(args.message, args.trace); break;
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
