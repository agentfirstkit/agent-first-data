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
  EventBuildError,
  validateProtocolEvent,
  validateProtocolStream,
  redactedValue,
  RedactionPolicy,
  PlainStyle,
  type OutputOptions,
  outputOptionsForPolicy,
  redactUrlSecrets,
  type JsonValue,
  normalizeUtcOffset,
  isValidRfc3339Date,
  isValidRfc3339Time,
  isValidRfc3339,
  isValidBcp47,
  decodeProtocolEvent,
  EventDecodeError,
} from "./format.ts";
import { render } from "./cli.ts";

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
        style: PlainStyle.Readable,
      };
      const got = redactedValue(tc.input, options);
      assert.deepEqual(got, tc.expected, "value mismatch");

      const gotJson = JSON.parse(render(tc.input as JsonValue, "json", outputOptions));
      assert.deepEqual(gotJson, tc.expected, "json mismatch");

      if (tc.expected_yaml !== undefined) {
        assert.equal(render(tc.input as JsonValue, "yaml", outputOptions), tc.expected_yaml, "yaml mismatch");
      }
      if (tc.expected_plain !== undefined) {
        assert.equal(render(tc.input as JsonValue, "plain", outputOptions), tc.expected_plain, "plain mismatch");
      }
    });
  }
});

describe("security fixtures", () => {
  const fixture = loadObject("security.json");
  for (const tc of fixture.redaction_cases) {
    it(`redaction/${tc.name}`, () => {
      const options = redactionOptions(tc);
      const outputOptions: OutputOptions = { redaction: options, style: PlainStyle.Readable };
      assert.deepEqual(redactedValue(tc.input, options), tc.expected);
      for (const output of [
        render(tc.input as JsonValue, "json", outputOptions),
        render(tc.input as JsonValue, "yaml", outputOptions),
        render(tc.input as JsonValue, "plain", outputOptions),
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
// Fixture protocol.json uses trace: {} by default for all builders.
// Result, progress, and log builders receive their complete payload; only the
// error builder owns protocol fields.
// Test cases use "args" vocabulary: "result" (payload), "code"+"message" (error),
// "hint" (→ .hint()), "retryable" (bool → .retryableIf()), "fields" (→ .fields()),
// "trace" (→ .trace()), and complete progress/log payloads.
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
          const builder = jsonProgress({ message: args.message, ...(args.fields ?? {}) });
          if (args.trace) {
            builder.trace(args.trace);
          }
          result = builder.build().toJSON();
          break;
        }
        case "log": {
          const builder = jsonLog({ level: args.level, message: args.message, ...(args.fields ?? {}) });
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

// --- decodeProtocolEvent ---

describe("decodeProtocolEvent", () => {
  // Payloads here are deliberately string-only, not numeric: a decoded number
  // is a LosslessNumber (see the module-level fidelity note in format.ts),
  // not a plain JS `number`, so it does not deepEqual a plain-number fixture
  // literal. Numeric decode fidelity has its own dedicated coverage below
  // ("number fidelity fixtures").
  it("decodes a result event", () => {
    const event = jsonResult({ hash: "abc" }).build();
    const decoded = decodeProtocolEvent(render(event, "json"));
    assert.equal(decoded.kind, "result");
    assert.deepEqual((decoded as { result: unknown }).result, { hash: "abc" });
    assert.deepEqual(decoded.trace, {});
  });

  it("decodes an error event with extension fields", () => {
    const event = jsonError("not_found", "missing").hint("check the id").field("id", "abc").retryable().build();
    const decoded = decodeProtocolEvent(render(event, "json"));
    assert.equal(decoded.kind, "error");
    if (decoded.kind !== "error") throw new Error("expected error");
    assert.equal(decoded.code, "not_found");
    assert.equal(decoded.message, "missing");
    assert.equal(decoded.retryable, true);
    assert.equal(decoded.hint, "check the id");
    assert.deepEqual(decoded.fields, { id: "abc" });
  });

  it("decodes a progress event", () => {
    const event = jsonProgress({ message: "halfway", status: "uploading" }).build();
    const decoded = decodeProtocolEvent(render(event, "json"));
    assert.equal(decoded.kind, "progress");
    assert.deepEqual((decoded as { progress: unknown }).progress, { message: "halfway", status: "uploading" });
  });

  it("decodes a log event", () => {
    const event = jsonLog({ level: "warn", message: "slow query" }).build();
    const decoded = decodeProtocolEvent(render(event, "json"));
    assert.equal(decoded.kind, "log");
    assert.deepEqual((decoded as { log: unknown }).log, { level: "warn", message: "slow query" });
  });

  it("throws EventDecodeError on invalid JSON", () => {
    assert.throws(() => decodeProtocolEvent("not json"), EventDecodeError);
  });

  it("throws EventDecodeError on an invalid envelope", () => {
    assert.throws(() => decodeProtocolEvent(JSON.stringify({ kind: "result" })), EventDecodeError);
  });

  it("throws EventDecodeError when strict validation fails", () => {
    // Missing trace fails the strict profile even though the shape is otherwise valid.
    assert.throws(() => decodeProtocolEvent(JSON.stringify({ kind: "result", result: {} })), EventDecodeError);
  });
});

// --- Number literal fidelity (shared spec/fixtures/number_fidelity.json) ---
//
// Regression guard for decodeProtocolEvent's lossless-json parse path and
// canonicalJsonString/yamlScalar/plainScalar's LosslessNumber handling in
// format.ts (the main cross-language gap this phase closes: plain
// JSON.parse forces every number through float64, corrupting digits).

describe("number fidelity fixtures", () => {
  for (const tc of load("number_fidelity.json")) {
    it(tc.name, () => {
      const decoded = decodeProtocolEvent(tc.input_line);
      assert.equal(decoded.kind, "result", `[number_fidelity/${tc.name}] expected a result event`);
      const result = (decoded as { result: JsonValue }).result;

      const gotJson = render(result, "json");
      assert.equal(gotJson, tc.expected_json, `[number_fidelity/${tc.name}] json mismatch`);

      if (tc.expected_yaml !== undefined) {
        const gotYaml = render(result, "yaml");
        assert.equal(gotYaml, tc.expected_yaml, `[number_fidelity/${tc.name}] yaml mismatch`);
      }
    });
  }

  it("does not regress ordinary (non-exotic) decoded numbers in plain output", () => {
    // decodeProtocolEvent wraps every number, including small ordinary ones;
    // tryProcessField must normalize back to a native number for Plain's
    // suffix arithmetic or this would silently stop formatting duration_ms
    // etc. for any decoded event (see format.ts's tryProcessField comment).
    const decoded = decodeProtocolEvent(
      JSON.stringify({ kind: "result", result: { duration_ms: 42, size_bytes: 5242880, cpu_percent: 85.5 }, trace: {} }),
    );
    const plain = render((decoded as { result: JsonValue }).result, "plain");
    assert.equal(plain, "cpu=85.5% duration=42ms size=5.0MiB");
  });
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

// --- JsonErrorBuilder is the only builder whose build() can throw ---

it("jsonError with empty code throws EventBuildError only at build()", () => {
  const builder = jsonError("", "message");
  assert.throws(() => builder.build(), (err: unknown) => {
    assert.ok(err instanceof EventBuildError);
    assert.match((err as EventBuildError).message, /code must be a non-empty string/);
    return true;
  });
});

it("jsonError with empty message throws EventBuildError only at build()", () => {
  const builder = jsonError("code", "");
  assert.throws(() => builder.build(), (err: unknown) => {
    assert.ok(err instanceof EventBuildError);
    assert.match((err as EventBuildError).message, /message must be a non-empty string/);
    return true;
  });
});

it("jsonResult/jsonProgress/jsonLog build() never throws, even with a non-object trace", () => {
  const badTrace = "not-an-object" as unknown as Record<string, JsonValue>;
  assert.doesNotThrow(() => jsonResult({ ok: true }).trace(badTrace).build());
  assert.doesNotThrow(() => jsonProgress({ current: 1 }).trace(badTrace).build());
  assert.doesNotThrow(() => jsonLog({ level: "info", message: "hi" }).trace(badTrace).build());
});

// --- Helper fixtures ---

describe("helper fixtures", () => {
  for (const tc of load("helpers.json")) {
    if (tc.name === "format_bytes_human") {
      for (const [input, expected] of tc.cases) {
        it(`bytes ${input} → ${expected}`, () => {
          const plain = render({ size_bytes: input }, "plain");
          assert.ok(plain.includes(`size=${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "format_with_commas") {
      for (const [input, expected] of tc.cases) {
        it(`commas ${input} → ${expected}`, () => {
          const plain = render({ price_jpy: input }, "plain");
          assert.ok(plain.includes(`price=\u00a5${expected}`), `got ${plain}`);
        });
      }
    }
    if (tc.name === "extract_currency_code") {
      for (const [input, expected] of tc.cases) {
        it(`currency code ${input}`, () => {
          // Test via render(..., "plain"): valid codes strip key, null keeps it
          const plain = render({ [input]: 100 }, "plain");
          if (expected === null) {
            assert.ok(plain.includes(`${input}=100`), `got ${plain}`);
          } else {
            assert.ok(!plain.includes(`${input}=`), `expected key stripped, got ${plain}`);
          }
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
    if (tc.name === "is_valid_bcp47") {
      for (const [input, expected] of tc.cases) {
        it(`isValidBcp47 ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(isValidBcp47(input), expected);
        });
      }
    }
    if (tc.name === "is_valid_rfc3339") {
      for (const [input, expected] of tc.cases) {
        it(`isValidRfc3339 ${JSON.stringify(input)} → ${expected}`, () => {
          assert.equal(isValidRfc3339(input), expected);
        });
      }
    }
  }
});

describe("output format fixtures", () => {
  for (const tc of load("output_formats.json")) {
    it(tc.name, () => {
      const input = tc.input as JsonValue;

      const gotJson = JSON.parse(render(input, "json"));
      assert.deepEqual(gotJson, tc.expected_json, "json mismatch");

      const gotYaml = render(input, "yaml");
      assert.equal(gotYaml, tc.expected_yaml, "yaml mismatch");

      const gotPlain = render(input, "plain");
      assert.equal(gotPlain, tc.expected_plain, "plain mismatch");
    });
  }
});

describe("output options", () => {
  it("raw yaml keeps suffix keys and structure", () => {
    const options: OutputOptions = {
      redaction: { policy: RedactionPolicy.TraceOnly },
      style: PlainStyle.Raw,
    };
    const out = render({
      code: "result",
      rows: [{ api_key_secret: "sk-live-1", duration_ms: 42 }],
      trace: { request_secret: "top-secret" },
    } as JsonValue, "yaml", options);

    assert.ok(out.includes("rows:\n  -"));
    assert.ok(out.includes('api_key_secret: "sk-live-1"'));
    assert.ok(out.includes("duration_ms: 42"));
    assert.ok(out.includes('request_secret: "***"'));
    assert.ok(!out.includes('duration: "42ms"'));
  });

  it("raw plain keeps suffix keys and redacts trace", () => {
    const options: OutputOptions = {
      redaction: { policy: RedactionPolicy.TraceOnly },
      style: PlainStyle.Raw,
    };
    const out = render({
      duration_ms: 42,
      trace: { request_secret: "top-secret" },
    } as JsonValue, "plain", options);

    assert.ok(out.includes("duration_ms=42"));
    assert.ok(out.includes("trace.request_secret=***"));
    assert.ok(!out.includes("duration=42ms"));
  });

  it("yaml ignores output style: Readable and Raw render identically", () => {
    // Unlike `plain`, YAML renders identically regardless of `PlainStyle`: it
    // is always structure-preserving.
    const value = { duration_ms: 42, name: "alice" } as JsonValue;
    const baseRedaction = { policy: RedactionPolicy.Off };
    const readable = render(value, "yaml", { redaction: baseRedaction, style: PlainStyle.Readable });
    const raw = render(value, "yaml", { redaction: baseRedaction, style: PlainStyle.Raw });

    assert.equal(readable, raw);
    assert.ok(readable.includes("duration_ms: 42"));
    assert.ok(!readable.includes("42ms"));
  });

  it("yaml stream of records has stable separator framing", () => {
    // Simulates how a CLI streams multiple AFDATA records: each record is
    // rendered independently and concatenated. `---` framing must stay
    // stable and each record's raw keys must stay intact and in order.
    const first = render({ kind: "log", duration_ms: 1 } as JsonValue, "yaml");
    const second = render({ kind: "result", duration_ms: 2 } as JsonValue, "yaml");
    const stream = `${first}\n${second}\n`;

    assert.equal((stream.match(/---/g) ?? []).length, 2);
    const firstIdx = stream.indexOf("duration_ms: 1");
    const secondIdx = stream.indexOf("duration_ms: 2");
    assert.ok(firstIdx >= 0, "first record present");
    assert.ok(secondIdx >= 0, "second record present");
    assert.ok(firstIdx < secondIdx, `records out of order: ${stream}`);
  });
});

describe("json safety", () => {
  it("serializes Error fields as readable strings", () => {
    const out = render({ error: new Error("timeout") } as unknown as JsonValue, "json");
    const parsed = JSON.parse(out) as Record<string, unknown>;
    assert.equal(parsed["error"], "timeout");
  });

  it("handles bigint and circular references without leaking secrets", () => {
    const meta: Record<string, unknown> = { api_key_secret: "sk-live-123", amount: 1n };
    meta["self"] = meta;

    const out = render({ meta } as unknown as JsonValue, "json");
    assert.ok(!out.includes("sk-live-123"), `secret leaked in output: ${out}`);

    const parsed = JSON.parse(out) as Record<string, unknown>;
    const parsedMeta = parsed["meta"] as Record<string, unknown>;
    assert.equal(parsedMeta["api_key_secret"], "***");
    assert.equal(parsedMeta["amount"], "<unsupported:bigint>");
    assert.equal(parsedMeta["self"], "<unsupported:circular>");
  });

  it("render json with TraceOnly keeps result secrets", () => {
    const out = render({
      kind: "result",
      result: { api_key_secret: "sk-live-123" },
      trace: { request_secret: "top-secret" },
    } as unknown as JsonValue, "json", outputOptionsForPolicy(RedactionPolicy.TraceOnly));
    const parsed = JSON.parse(out) as Record<string, unknown>;
    const parsedResult = parsed["result"] as Record<string, unknown>;
    const parsedTrace = parsed["trace"] as Record<string, unknown>;
    assert.equal(parsedTrace["request_secret"], "***");
    assert.equal(parsedResult["api_key_secret"], "sk-live-123");
  });

  it("render json with Off keeps all secrets", () => {
    const out = render({ api_key_secret: "sk-live-123" } as unknown as JsonValue, "json", outputOptionsForPolicy(RedactionPolicy.Off));
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
    const out = render(input as JsonValue, "json");
    assert.ok(out.includes("<afdata:max-depth>"));
    assert.ok(!out.includes("***"));
  });
});
