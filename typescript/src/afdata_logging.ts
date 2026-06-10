/**
 * AFDATA-compliant structured logging.
 *
 * Outputs log events using agent-first-data formatting functions:
 * - JSON: single-line JSONL via outputJson (secrets redacted, original keys)
 * - Plain: single-line logfmt via outputPlain (keys stripped, values formatted)
 * - YAML: multi-line via outputYaml (keys stripped, values formatted)
 *
 * Span fields are carried via AsyncLocalStorage (async-safe).
 *
 * Usage:
 *   import { log, span, initJson, initPlain, initYaml } from "agent-first-data/afdata_logging";
 *   initJson();  // or initPlain() or initYaml()
 *   log.info("Server started");
 *   await span({ request_id: "abc" }, async () => {
 *     log.info("Processing", { domain: "example.com" });
 *   });
 */

import { AsyncLocalStorage } from "node:async_hooks";
import {
  outputJsonWithOptions,
  outputYamlWithOptions,
  outputPlainWithOptions,
  type JsonValue,
  type RedactionOptions,
} from "./format.js";

type Level = "trace" | "debug" | "info" | "warn" | "error";
type LogFormat = "json" | "plain" | "yaml";
type LoggingRedactionOptions = {
  secretNames?: readonly string[];
};

const spanStore = new AsyncLocalStorage<Record<string, unknown>>();

let currentFormat: LogFormat = "json";
let currentRedaction: RedactionOptions = {};

function setFormat(format: LogFormat, options?: LoggingRedactionOptions): void {
  currentFormat = format;
  currentRedaction = options?.secretNames ? { secretNames: options.secretNames } : {};
}

/** Set log output to single-line JSONL (secrets redacted, original keys). */
export function initJson(options?: LoggingRedactionOptions): void { setFormat("json", options); }

/** Set log output to single-line logfmt (keys stripped, values formatted). */
export function initPlain(options?: LoggingRedactionOptions): void { setFormat("plain", options); }

/** Set log output to multi-line YAML (keys stripped, values formatted). */
export function initYaml(options?: LoggingRedactionOptions): void { setFormat("yaml", options); }

function emit(level: Level, message: string, fields?: Record<string, unknown>): void {
  const entry: Record<string, unknown> = {
    timestamp_epoch_ms: Date.now(),
    message,
    code: "log",
    level,
  };

  // Span fields (from AsyncLocalStorage)
  const spanFields = spanStore.getStore();
  if (spanFields) {
    for (const [k, v] of Object.entries(spanFields)) {
      entry[k] = v;
    }
  }

  // Event fields override span fields, except protocol code is always "log".
  if (fields) {
    for (const [k, v] of Object.entries(fields)) {
      if (k === "code") continue;
      entry[k] = v;
    }
  }

  // Format using the library's own output functions
  const value = entry as unknown as JsonValue;
  let line: string;
  switch (currentFormat) {
    case "plain":
      line = outputPlainWithOptions(value, { redaction: currentRedaction });
      break;
    case "yaml":
      line = outputYamlWithOptions(value, { redaction: currentRedaction });
      break;
    default:
      line = outputJsonWithOptions(value, { redaction: currentRedaction });
      break;
  }

  process.stdout.write(line + "\n");
}

/**
 * AFDATA logger. Each method outputs a single log line to stdout.
 * Format is controlled by initJson/initPlain/initYaml (default: JSON).
 */
export const log = {
  trace: (msg: string, fields?: Record<string, unknown>): void => emit("trace", msg, fields),
  debug: (msg: string, fields?: Record<string, unknown>): void => emit("debug", msg, fields),
  info:  (msg: string, fields?: Record<string, unknown>): void => emit("info", msg, fields),
  warn:  (msg: string, fields?: Record<string, unknown>): void => emit("warn", msg, fields),
  error: (msg: string, fields?: Record<string, unknown>): void => emit("error", msg, fields),
};

/**
 * Run fn within a span that adds fields to all log events.
 * Spans nest: inner spans inherit and can override outer span fields.
 */
export function span<T>(fields: Record<string, unknown>, fn: () => T): T {
  const parent = spanStore.getStore() ?? {};
  return spanStore.run({ ...parent, ...fields }, fn);
}
