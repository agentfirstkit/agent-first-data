/**
 * Tests for AFDATA output formatting — driven by shared spec/fixtures.
 */

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  jsonError,
  jsonProgress,
  jsonLog,
  jsonResult,
  validateProtocolEvent,
  validateProtocolStream,
  redactedValue,
  RedactionPolicy,
  OutputStyle,
  type OutputOptions,
  outputOptionsForPolicy,
  outputJson,
  outputYaml,
  outputPlain,
  redactUrlSecrets,
  type JsonValue,
  parseSize,
  normalizeUtcOffset,
  isValidRfc3339Date,
  isValidRfc3339Time,
  decodeProtocolEvent,
  EventDecodeError,
} from "./format.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = join(__dirname, "..", "..", "spec", "fixtures");

function load(name: string): any[] {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf-8"));
}

function loadObject(name: string): any {
  return JSON.parse(readFileSync(join(FIXTURES_DIR, name), "utf-8"));
}

function redactionOptions(tc: any): { policy?: RedactionPolicy; secretNames?: readonly string[] } {
  const opts = tc.options ?? {};
  return {
    policy: opts.policy as RedactionPolicy | undefined,
    secretNames: opts.secret_names ?? [],
  };
}

// --- URL redaction fixtures ---

describe("redact_url fixtures", () => {
  for (const tc of load("redact_url.json")) {
    it(tc.name, () => {
      const options = redactionOptions(tc);
      assert.equal(redactUrlSecrets(tc.input, options), tc.expected);
    });
  }
});

// --- Redact fixtures (default full redaction via redactedValue) ---

describe("redact fixtures", () => {
  for (const tc of load("redact.json")) {
    it(tc.name, () => {
      const input = JSON.parse(JSON.stringify(tc.input));
      assert.deepEqual(redactedValue(input), tc.expected);
      // redactedValue returns a copy: the input must be untouched.
      assert.deepEqual(input, tc.input);
    });
  }
});

// --- Redaction options fixtures ---

describe("redaction options fixtures", () => {
  for (const tc of load("redaction_options.json")) {
    it(tc.name, () => {
      const options = redactionOptions(tc);
      const outputOptions: OutputOptions = {
        redaction: options,
        style: OutputStyle.Readable,
      };
      const got = redactedValue(tc.input, options);
      assert.deepEqual(got, tc.expected, "value mismatch");

      const gotJson = JSON.parse(outputJson(tc.input as JsonValue, outputOptions));
      assert.deepEqual(gotJson, tc.expected, "json mismatch");

      if (tc.expected_yaml !== undefined) {
        assert.equal(outputYaml(tc.input as JsonValue, outputOptions), tc.expected_yaml, "yaml mismatch");
      }
      if (tc.expected_plain !== undefined) {
        assert.equal(outputPlain(tc.input as JsonValue, outputOptions), tc.expected_plain, "plain mismatch");
      }
    });
  }
});

describe("security fixtures", () => {
  const fixture = loadObject("security.json");
  for (const tc of fixture.redaction_cases) {
    it(`redaction/${tc.name}`, () => {
      const options = redactionOptions(tc);
      const outputOptions: OutputOptions = { redaction: options, style: OutputStyle.Readable };
      assert.deepEqual(redactedValue(tc.input, options), tc.expected);
      for (const output of [
        outputJson(tc.input as JsonValue, outputOptions),
        outputYaml(tc.input as JsonValue, outputOptions),
        outputPlain(tc.input as JsonValue, outputOptions),
      ]) {
        for (const needle of tc.must_contain) {
          assert.ok(output.includes(needle), `output missing ${JSON.stringify(needle)}: ${output}`);
        }
        for (const needle of tc.must_not_contain) {
          assert.ok(!output.includes(needle), `output leaked ${JSON.stringify(needle)}: ${output}`);
        }
      }
    });
  }
});

// --- Protocol fixtures ---
// Fixture protocol.json has been updated to canonical 0.16 format with:
// 1. trace: {} added by default to all builders
// 2. log builder requires (level, message) parameters
// 3. progress builder requires message parameter
// Test cases use "args" vocabulary: "result" (payload), "code"+"message" (error),
// "hint" (→ .hint()), "retryable" (bool → .retryableIf()), "fields" (→ .fields()),
// "trace" (→ .trace()), "message" (progress/log), "level" (log).
// All builder results are deep-compared against fixture "expected".

describe("protocol fixtures", () => {
  for (const tc of load("protocol.json")) {
    it(tc.name, () => {
      if (tc.invalid !== undefined) {
        assert.throws(() => validateProtocolEvent(tc.invalid));
        return;
      }
      let result: any;
      const args = tc.args;
      switch (tc.type) {
        case "result": result = jsonResult(args.result).build().toJSON(); break;
        case "result_trace": result = jsonResult(args.result).trace(args.trace).build().toJSON(); break;
        case "error": result = jsonError(args.code, args.message).build().toJSON(); break;
        case "error_trace": result = jsonError(args.code, args.message).trace(args.trace).build().toJSON(); break;
        case "error_hint": result = jsonError(args.code, args.message).hint(args.hint).build().toJSON(); break;
        case "error_retryable": result = jsonError(args.code, args.message).retryableIf(args.retryable).build().toJSON(); break;
        case "error_extension_fields": {
          const builder = jsonError(args.code, args.message);
          if (args.fields) {
            builder.fields(args.fields);
          }
          result = builder.build().toJSON();
          break;
        }
        case "progress": {
          const builder = jsonProgress(args.message);
          if (args.fields) {
            builder.fields(args.fields);
          }
          if (args.trace) {
            builder.trace(args.trace);
          }
          result = builder.build().toJSON();
          break;
        }
        case "log": {
          const builder = jsonLog(args.level, args.message);
          if (args.fields) {
            builder.fields(args.fields);
          }
          if (args.trace) {
            builder.trace(args.trace);
          }
          result = builder.build().toJSON();
          break;
        }
        default: throw new Error(`unknown type: ${tc.type}`);
      }
      validateProtocolEvent(result);
      // Deep equality check against fixture expected value
      assert.deepEqual(result, tc.expected, `fixture ${tc.name} event mismatch`);
    });
  }
});

describe("protocol stream fixtures", () => {
  for (const tc of load("protocol_streams.json")) {
    it(tc.name, () => {
      if (tc.valid) {
        assert.doesNotThrow(() => validateProtocolStream(tc.events, false));
      } else {
        assert.throws(() => validateProtocolStream(tc.events, false));
      }
    });
  }
});

describe("protocol strict fixtures", () => {
  for (const tc of load("protocol_strict.json")) {
    it(tc.name, () => {
      if (tc.valid) assert.doesNotThrow(() => validateProtocolStream(tc.events));
      else assert.throws(() => validateProtocolStream(tc.events));
    });
  }
});

it("error builder ignores reserved extension fields", () => {
  const event = jsonError("explicit", "message")
    .field("code", "wrong")
    .field("message", "wrong")
    .field("hint", "wrong")
    .field("detail", 1);

  // Attempting to write reserved fields should accumulate errors
  // but not throw until build() is called
  assert.throws(() => event.build(), /cannot write reserved error field/);
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
    if (tc.name === "normalize_utc_offset") {
      for (const [input, expected] of tc.cases) {
        it(`normalizeUtcOffset ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(normalizeUtcOffset(input), expected);
        });
      }
    }
    if (tc.name === "is_valid_rfc3339_date") {
      for (const [input, expected] of tc.cases) {
        it(`isValidRfc3339Date ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(isValidRfc3339Date(input), expected);
        });
      }
    }
    if (tc.name === "is_valid_rfc3339_time") {
      for (const [input, expected] of tc.cases) {
        it(`isValidRfc3339Time ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(isValidRfc3339Time(input), expected);
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

describe("output options", () => {
  it("raw yaml keeps suffix keys and structure", () => {
    const options: OutputOptions = {
      redaction: { policy: RedactionPolicy.RedactionTraceOnly },
      style: OutputStyle.Raw,
    };
    const out = outputYaml({
      code: "result",
      rows: [{ api_key_secret: "sk-live-1", duration_ms: 42 }],
      trace: { request_secret: "top-secret" },
    } as JsonValue, options);

    assert.ok(out.includes("rows:\n  -"));
    assert.ok(out.includes('api_key_secret: "sk-live-1"'));
    assert.ok(out.includes("duration_ms: 42"));
    assert.ok(out.includes('request_secret: "***"'));
    assert.ok(!out.includes('duration: "42ms"'));
  });

  it("raw plain keeps suffix keys and redacts trace", () => {
    const options: OutputOptions = {
      redaction: { policy: RedactionPolicy.RedactionTraceOnly },
      style: OutputStyle.Raw,
    };
    const out = outputPlain({
      duration_ms: 42,
      trace: { request_secret: "top-secret" },
    } as JsonValue, options);

    assert.ok(out.includes("duration_ms=42"));
    assert.ok(out.includes("trace.request_secret=***"));
    assert.ok(!out.includes("duration=42ms"));
  });

  it("with-options functions default to readable style", () => {
    const out = outputYaml(
      { duration_ms: 42 } as JsonValue,
      {
        redaction: { policy: RedactionPolicy.RedactionNone },
        style: OutputStyle.Readable,
      },
    );
    assert.ok(out.includes('duration: "42ms"'));
    assert.ok(!out.includes("duration_ms:"));
  });
});

describe("parseSize safety", () => {
  it("returns null for unsafe integers", () => {
    assert.equal(parseSize("9007199254740993"), null);
    assert.equal(parseSize("9007199254740992"), null);
    assert.equal(parseSize("9007199255MB"), null);
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

  it("outputJson with RedactionTraceOnly keeps result secrets", () => {
    const out = outputJson({
      kind: "result",
      result: { api_key_secret: "sk-live-123" },
      trace: { request_secret: "top-secret" },
    } as unknown as JsonValue, outputOptionsForPolicy(RedactionPolicy.RedactionTraceOnly));
    const parsed = JSON.parse(out) as Record<string, unknown>;
    const parsedResult = parsed["result"] as Record<string, unknown>;
    const parsedTrace = parsed["trace"] as Record<string, unknown>;
    assert.equal(parsedTrace["request_secret"], "***");
    assert.equal(parsedResult["api_key_secret"], "sk-live-123");
  });

  it("outputJson with RedactionNone keeps all secrets", () => {
    const out = outputJson(
      { api_key_secret: "sk-live-123" } as unknown as JsonValue,
      outputOptionsForPolicy(RedactionPolicy.RedactionNone),
    );
    const parsed = JSON.parse(out) as Record<string, unknown>;
    assert.equal(parsed["api_key_secret"], "sk-live-123");
  });

  it("redactedValue returns a safe copy", () => {
    const input = { api_key_secret: "sk-live-123", nested: { token_secret: "tok" } };
    const got = redactedValue(input) as Record<string, unknown>;
    const nested = got["nested"] as Record<string, unknown>;
    assert.equal(got["api_key_secret"], "***");
    assert.equal(nested["token_secret"], "***");
    assert.equal(input.api_key_secret, "sk-live-123");
  });

  it("redactedValue redacts secret subtrees by default", () => {
    const input = { db_secret: { password_secret: "real", host: "localhost" } };
    const defaultValue = redactedValue(input) as Record<string, unknown>;
    assert.equal(defaultValue["db_secret"], "***");
  });

  it("max-depth marker is not the secret redaction marker", () => {
    let input: unknown = "leaf";
    for (let i = 0; i < 300; i++) input = { next: input };
    const out = outputJson(input as JsonValue);
    assert.ok(out.includes("<afdata:max-depth>"));
    assert.ok(!out.includes("***"));
  });
});
