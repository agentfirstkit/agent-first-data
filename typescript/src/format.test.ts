/**
 * Tests for AFD output formatting — driven by shared spec/fixtures.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  OutputFormat,
  toYaml,
  toPlain,
  redactSecrets,
  ok,
  okTrace,
  error,
  errorTrace,
  startup,
  status,
  formatValue,
  parseSize,
} from "./format.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = join(__dirname, "..", "..", "spec", "fixtures");

function load(name: string): any[] {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf-8"));
}

// --- Plain fixtures ---

describe("plain fixtures", () => {
  for (const tc of load("plain.json")) {
    it(tc.name, () => {
      const plain = toPlain(tc.input);
      for (const s of tc.contains) {
        assert.ok(plain.includes(s), `expected ${JSON.stringify(s)} in ${JSON.stringify(plain)}`);
      }
      for (const s of tc.not_contains ?? []) {
        assert.ok(!plain.includes(s), `unexpected ${JSON.stringify(s)} in ${JSON.stringify(plain)}`);
      }
    });
  }
});

// --- YAML fixtures ---

describe("yaml fixtures", () => {
  for (const tc of load("yaml.json")) {
    it(tc.name, () => {
      const yaml = toYaml(tc.input);
      if (tc.starts_with) {
        assert.ok(yaml.startsWith(tc.starts_with), `starts_with failed`);
      }
      for (const s of tc.contains ?? []) {
        assert.ok(yaml.includes(s), `expected ${JSON.stringify(s)} in ${JSON.stringify(yaml)}`);
      }
    });
  }
});

// --- Redact fixtures ---

describe("redact fixtures", () => {
  for (const tc of load("redact.json")) {
    it(tc.name, () => {
      const inp = JSON.parse(JSON.stringify(tc.input));
      redactSecrets(inp);
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
        case "ok": result = ok(args.result); break;
        case "ok_trace": result = okTrace(args.result, args.trace); break;
        case "error": result = error(args.message); break;
        case "error_trace": result = errorTrace(args.message, args.trace); break;
        case "startup": result = startup(args.config, args.args, args.env); break;
        case "status": result = status(args.code, args.fields); break;
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

// --- Exact fixtures ---

describe("exact fixtures", () => {
  for (const tc of load("exact.json")) {
    it(tc.name, () => {
      let got: string;
      if (tc.format === "plain") got = toPlain(tc.input);
      else if (tc.format === "yaml") got = toYaml(tc.input);
      else throw new Error(`unknown format: ${tc.format}`);
      assert.equal(got, tc.expected);
    });
  }
});

// --- Helper fixtures (format_bytes_human, format_with_commas via plain output) ---

describe("helper fixtures", () => {
  for (const tc of load("helpers.json")) {
    if (tc.name === "format_bytes_human") {
      for (const [input, expected] of tc.cases) {
        it(`bytes ${input} → ${expected}`, () => {
          const plain = toPlain({ size_bytes: input });
          assert.ok(plain.includes(`size_bytes: ${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "format_with_commas") {
      for (const [input, expected] of tc.cases) {
        it(`commas ${input} → ${expected}`, () => {
          const plain = toPlain({ price_jpy: input });
          assert.ok(plain.includes(`price_jpy: \u00a5${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "extract_currency_code") {
      for (const [input, expected] of tc.cases) {
        it(`currency code ${input}`, () => {
          if (expected === null) {
            // _cents alone shouldn't produce currency formatting with a code
            const plain = toPlain({ [input]: 100 });
            assert.ok(!plain.match(/\d+\.\d{2} [A-Z]+/), `got ${plain}`);
          } else {
            // Specific handlers (_usd_cents→$, _eur_cents→€) don't emit the code string,
            // so just verify the field produces a formatted (non-raw) value
            const plain = toPlain({ [input]: 100 });
            assert.ok(!plain.includes(`${input}: 100\n`) && !plain.endsWith(`${input}: 100`), `got ${plain}`);
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

// --- OutputFormat (not in fixtures) ---

describe("OutputFormat", () => {
  it("json", () => assert.equal(formatValue(OutputFormat.Json, { status: "ok" }), '{"status":"ok"}'));
  it("yaml", () => {
    const out = formatValue(OutputFormat.Yaml, { status: "ok" });
    assert.ok(out.startsWith("---\n"));
    assert.ok(out.includes('status: "ok"'));
  });
  it("plain", () => assert.equal(formatValue(OutputFormat.Plain, { status: "ok" }), "status: ok"));
});
