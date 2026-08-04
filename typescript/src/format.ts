/**
 * AFDATA output formatting and protocol templates.
 *
 * Public APIs include protocol event builders, redactedValue (covers _secret
 * and _url fields), redactUrlSecrets, normalizeUtcOffset, isValidRfc3339Date,
 * isValidRfc3339Time, isValidRfc3339, isValidBcp47, RedactionPolicy,
 * PlainStyle, and OutputOptions. The single render entry point (`render`)
 * lives in cli.ts and calls this module's internal JSON/YAML/plain formatters.
 *
 * Number literal fidelity: decodeProtocolEvent parses with `lossless-json`
 * instead of JSON.parse, so a number's exact source literal (arbitrary
 * precision integers, high-precision floats, exponent forms, "-0") survives
 * decode. Parsed numbers become LosslessNumber internally; every JSON/YAML
 * rendering path here treats it as an opaque scalar leaf (never destructured
 * as a plain object) and re-emits its exact literal text. Native bigint values
 * are also supported so callers can render integers beyond Number's safe
 * range without first corrupting them.
 */

import { parse as losslessParse, stringify as losslessStringify, isLosslessNumber } from "lossless-json";

export type JsonValue =
  | string
  | number
  | bigint
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

export type LogLevel = "debug" | "info" | "warn" | "error";

// ═══════════════════════════════════════════
// Event type: opaque branded wrapper
// ═══════════════════════════════════════════

/**
 * Event is an opaque type wrapping a protocol event envelope.
 * Guaranteed to be strict-valid by construction.
 */
export class Event {
  private readonly __brand = "Event";

  constructor(private readonly value: JsonValue) {}

  toJSON(): JsonValue {
    return this.value;
  }

  valueOf(): JsonValue {
    return this.value;
  }
}

// ═══════════════════════════════════════════
// EventBuildError: typed error from .build()
// ═══════════════════════════════════════════

export class EventBuildError extends Error {
  constructor(
    message: string,
    public readonly issues: string[],
  ) {
    super(message);
    this.name = "EventBuildError";
  }
}

// ═══════════════════════════════════════════
// Public API: Protocol v1 Builders and Validation
// ═══════════════════════════════════════════

// ═══════════════════════════════════════════
// Fluent builders
// ═══════════════════════════════════════════

export class JsonResultBuilder {
  private traceValue: Record<string, JsonValue> = {};

  constructor(private readonly payload: JsonValue) {}

  trace(value: Record<string, JsonValue> | undefined): this {
    if (value !== undefined) {
      this.traceValue = value;
    }
    return this;
  }

  /** Build the event. This builder cannot fail. */
  build(): Event {
    const m: Record<string, JsonValue> = { kind: "result", result: this.payload, trace: this.traceValue };
    return new Event(m);
  }
}

export class JsonErrorBuilder {
  private retryableValue = false;
  private hintValue: string | undefined;
  private extensionFields: Record<string, JsonValue> = {};
  private traceValue: Record<string, JsonValue> = {};
  private issues: string[] = [];

  constructor(
    private readonly code: string,
    private readonly message: string,
  ) {
    if (!code || code === "") this.issues.push("code must be a non-empty string");
    if (!message || message === "") this.issues.push("message must be a non-empty string");
  }

  retryable(): this {
    this.retryableValue = true;
    return this;
  }

  retryableIf(flag: boolean): this {
    this.retryableValue = flag;
    return this;
  }

  hint(value: string): this {
    if (value && value !== "") {
      this.hintValue = value;
    }
    return this;
  }

  hintIfSome(value: string | undefined | null): this {
    if (value) {
      return this.hint(value);
    }
    return this;
  }

  field(name: string, value: JsonValue): this {
    if (this.isReservedErrorField(name)) {
      this.issues.push(`cannot write reserved error field ${JSON.stringify(name)}`);
    } else {
      this.extensionFields[name] = value;
    }
    return this;
  }

  fields(obj: unknown): this {
    if (!isObject(obj)) {
      this.issues.push("fields must be a JSON object");
      return this;
    }
    for (const [key, val] of Object.entries(obj)) {
      if (this.isReservedErrorField(key)) {
        this.issues.push(`cannot write reserved error field ${JSON.stringify(key)}`);
      } else {
        this.extensionFields[key] = val as JsonValue;
      }
    }
    return this;
  }

  extend(obj: unknown): this {
    if (!isObject(obj)) {
      this.issues.push("extend must be a JSON object");
      return this;
    }
    for (const [key, val] of Object.entries(obj)) {
      if (this.isReservedErrorField(key)) {
        this.issues.push(`cannot write reserved error field ${JSON.stringify(key)}`);
      } else {
        this.extensionFields[key] = val as JsonValue;
      }
    }
    return this;
  }

  trace(value: Record<string, JsonValue> | undefined): this {
    if (value !== undefined) {
      if (!isObject(value)) {
        this.issues.push("trace must be a JSON object");
      } else {
        this.traceValue = value as Record<string, JsonValue>;
      }
    }
    return this;
  }

  private isReservedErrorField(name: string): boolean {
    return name === "code" || name === "message" || name === "hint" || name === "retryable";
  }

  build(): Event {
    if (this.issues.length > 0) {
      throw new EventBuildError(`Failed to build error event: ${this.issues.join("; ")}`, this.issues);
    }
    const error: Record<string, JsonValue> = { ...this.extensionFields };
    error.code = this.code;
    error.message = this.message;
    error.retryable = this.retryableValue;
    if (this.hintValue !== undefined) error.hint = this.hintValue;
    const m: Record<string, JsonValue> = { kind: "error", error, trace: this.traceValue };
    try {
      losslessStringify(m);
    } catch (error) {
      const issue = `event is not JSON serializable: ${error instanceof Error ? error.message : String(error)}`;
      throw new EventBuildError(`Failed to build error event: ${issue}`, [issue]);
    }
    return new Event(m);
  }
}

export class JsonProgressBuilder {
  private traceValue: Record<string, JsonValue> = {};

  constructor(private readonly payload: JsonValue) {}

  trace(value: Record<string, JsonValue> | undefined): this {
    if (value !== undefined) {
      this.traceValue = value;
    }
    return this;
  }

  /** Build the event. This builder cannot fail. */
  build(): Event {
    const m: Record<string, JsonValue> = { kind: "progress", progress: this.payload, trace: this.traceValue };
    return new Event(m);
  }
}

export class JsonLogBuilder {
  private traceValue: Record<string, JsonValue> = {};

  constructor(private readonly payload: JsonValue) {}

  trace(value: Record<string, JsonValue> | undefined): this {
    if (value !== undefined) {
      this.traceValue = value;
    }
    return this;
  }

  /** Build the event. This builder cannot fail. */
  build(): Event {
    const m: Record<string, JsonValue> = { kind: "log", log: this.payload, trace: this.traceValue };
    return new Event(m);
  }
}

/** Fluent builder for result events. */
export function jsonResult(payload: JsonValue): JsonResultBuilder {
  return new JsonResultBuilder(payload);
}

/** Fluent builder for error events. */
export function jsonError(code: string, message: string): JsonErrorBuilder {
  return new JsonErrorBuilder(code, message);
}

/** Fluent builder for progress events. */
export function jsonProgress(payload: JsonValue): JsonProgressBuilder {
  return new JsonProgressBuilder(payload);
}

/** Fluent builder for log events. */
export function jsonLog(payload: JsonValue): JsonLogBuilder {
  return new JsonLogBuilder(payload);
}

/** Validate one protocol event envelope. `strict` also enforces the recommended strict protocol profile (default true). */
export function validateProtocolEvent(event: unknown, strict = true): void {
  if (!isObject(event)) {
    throw new Error("event must be a JSON object");
  }
  const kind = event.kind;
  if (kind !== "result" && kind !== "error" && kind !== "progress" && kind !== "log") {
    throw new Error("event.kind must be one of result, error, progress, log");
  }
  if (!(kind in event)) {
    throw new Error(`event payload field ${JSON.stringify(kind)} is required`);
  }
  for (const key of Object.keys(event)) {
    if (key !== "kind" && key !== kind && key !== "trace") {
      throw new Error(`unexpected top-level field ${JSON.stringify(key)}`);
    }
  }
  if ("trace" in event && !isObject(event.trace)) {
    throw new Error("event.trace must be a JSON object when present");
  }
  if (kind === "error") {
    validateErrorPayload(event.error);
  }
  if (!strict) return;
  if (!isObject(event.trace)) throw new Error("event.trace is required in strict mode");
  if (kind === "error") validateStrictError(event.error);
}

function validateErrorPayload(error: unknown): void {
  if (!isObject(error)) {
    throw new Error("event.error must be a JSON object");
  }
  if (typeof error.code !== "string" || error.code === "") {
    throw new Error("event.error.code must be a non-empty string");
  }
  if (typeof error.message !== "string" || error.message === "") {
    throw new Error("event.error.message must be a non-empty string");
  }
  if ("hint" in error && typeof error.hint !== "string") {
    throw new Error("event.error.hint must be a string when present");
  }
}

/** Validate finite CLI lifecycle: (log | progress)* -> exactly one terminal. `strict` also enforces the recommended strict protocol profile (default true). */
export function validateProtocolStream(events: readonly unknown[], strict = true): void {
  let terminalSeen = false;
  events.forEach((event, idx) => {
    try {
      validateProtocolEvent(event, strict);
    } catch (e) {
      throw new Error(`event ${idx}: ${(e as Error).message}`);
    }
    const kind = (event as Record<string, unknown>).kind;
    if (kind === "log" || kind === "progress") {
      if (terminalSeen) throw new Error(`event ${idx}: non-terminal event after terminal`);
    } else if (kind === "result" || kind === "error") {
      if (terminalSeen) throw new Error(`event ${idx}: duplicate terminal event`);
      terminalSeen = true;
    }
  });
  if (!terminalSeen) {
    throw new Error("event stream must contain exactly one terminal result or error");
  }
}

function validateStrictError(error: unknown): void {
  if (!isObject(error)) throw new Error("event.error must be a JSON object in strict mode");
  if (typeof error.code !== "string" || error.code === "") {
    throw new Error("event.error.code must be a non-empty string in strict mode");
  }
  if (typeof error.message !== "string" || error.message === "") {
    throw new Error("event.error.message must be a non-empty string in strict mode");
  }
  if (typeof error.retryable !== "boolean") {
    throw new Error("event.error.retryable must be a boolean in strict mode");
  }
  if ("hint" in error && typeof error.hint !== "string") {
    throw new Error("event.error.hint must be a string when present in strict mode");
  }
}

// ═══════════════════════════════════════════
// Public API: decodeProtocolEvent
// ═══════════════════════════════════════════

export type DecodedResult = {
  kind: "result";
  result: JsonValue;
  trace?: Record<string, JsonValue>;
};

export type DecodedError = {
  kind: "error";
  code: string;
  message: string;
  retryable: boolean;
  hint?: string;
  fields: Record<string, JsonValue>;
  trace?: Record<string, JsonValue>;
};

export type DecodedProgress = {
  kind: "progress";
  progress: JsonValue;
  trace?: Record<string, JsonValue>;
};

export type DecodedLog = {
  kind: "log";
  log: JsonValue;
  trace?: Record<string, JsonValue>;
};

/** Discriminated union of decoded protocol events, narrow on `kind`. */
export type DecodedEvent = DecodedResult | DecodedError | DecodedProgress | DecodedLog;

/** Thrown by decodeProtocolEvent for malformed JSON or a strict-invalid event. */
export class EventDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EventDecodeError";
  }
}

const ERROR_RESERVED_FIELDS = new Set(["code", "message", "hint", "retryable"]);

/**
 * Parse one protocol line (a single JSON text value), strict-validate it, and
 * return a typed decoded event.
 */
export function decodeProtocolEvent(text: string): DecodedEvent {
  let parsed: unknown;
  try {
    // lossless-json instead of JSON.parse: preserves each number's exact
    // source literal (see module-level fidelity note) instead of collapsing
    // it through float64. isSyntaxError-compatible: throws the same
    // SyntaxError shape JSON.parse would.
    parsed = losslessParse(text);
  } catch (e) {
    throw new EventDecodeError(`invalid JSON: ${(e as Error).message}`);
  }
  try {
    validateProtocolEvent(parsed, true);
  } catch (e) {
    throw new EventDecodeError((e as Error).message);
  }
  const event = parsed as Record<string, JsonValue>;
  const trace = isObject(event.trace) ? (event.trace as Record<string, JsonValue>) : undefined;
  switch (event.kind) {
    case "result":
      return { kind: "result", result: event.result, trace };
    case "error": {
      const error = event.error as Record<string, JsonValue>;
      const fields: Record<string, JsonValue> = {};
      for (const [k, v] of Object.entries(error)) {
        if (!ERROR_RESERVED_FIELDS.has(k)) fields[k] = v;
      }
      const decoded: DecodedError = {
        kind: "error",
        code: error.code as string,
        message: error.message as string,
        retryable: error.retryable as boolean,
        fields,
        trace,
      };
      if (typeof error.hint === "string") decoded.hint = error.hint;
      return decoded;
    }
    case "progress":
      return { kind: "progress", progress: event.progress, trace };
    case "log":
      return { kind: "log", log: event.log, trace };
    default:
      // Unreachable: validateProtocolEvent already constrains event.kind.
      throw new EventDecodeError(`unsupported event kind ${JSON.stringify(event.kind)}`);
  }
}

// ═══════════════════════════════════════════
// Internal: Output Formatters (called by render() in cli.ts)
// ═══════════════════════════════════════════

/**
 * Which fields a redaction pass scrubs. The default is `All`.
 *
 * The policy selects a *scope* inside a structured value. A command line
 * (`redactArgv`) and a bare URL string (`redactUrlSecrets`) have no
 * result/trace split to scope to, so on those two paths `TraceOnly` redacts in
 * full like `All`, and only `Off` turns redaction off.
 */
export enum RedactionPolicy {
  All = "All",
  TraceOnly = "TraceOnly",
  Off = "Off",
}

/**
 * Whether a policy redacts an input that carries no scope of its own — a
 * command line or a single URL string, as opposed to a JSON value with a
 * `trace` object to narrow to.
 *
 * `TraceOnly` narrows *where* redaction applies within a value; it does not
 * weaken redaction. A standalone argv or URL has no non-`trace` half to leave
 * alone — and both are diagnostic material by construction, the very thing
 * `TraceOnly` scrubs — so `TraceOnly` redacts them in full, exactly like `All`.
 * Only `Off`, the caller explicitly asking for raw output, disables redaction,
 * and it does so on all three paths alike.
 */
function redactsUnscopedInput(policy: RedactionPolicy | undefined): boolean {
  return policy !== RedactionPolicy.Off;
}

export enum PlainStyle {
  Readable = "Readable",
  Raw = "Raw",
}

export type OutputOptions = {
  /** Redaction policy. Omitted means default full redaction. */
  redaction?: {
    /** Optional scoped policy. Omitted means default full redaction. */
    policy?: RedactionPolicy;
    /**
     * Field names to treat as secrets in addition to _secret suffixes.
     * Matching is exact field-name equality at any nesting level.
     */
    secretNames?: readonly string[];
    /**
     * Exact legacy field names whose string leaves receive URL-aware
     * redaction in addition to _url/_URL fields.
     */
    urlNames?: readonly string[];
  };
  /** Rendering style for plain (logfmt) output only. Omitted means readable.
   * JSON and YAML are structure-preserving and ignore this setting. */
  style?: PlainStyle;
};

/** Convenience: build OutputOptions scoped to a RedactionPolicy. */
export function outputOptionsForPolicy(policy: RedactionPolicy): OutputOptions {
  return { redaction: { policy } };
}

/**
 * Format as single-line JSON. Secrets redacted, original keys, raw values.
 * Internal: called by `render` in cli.ts, which is the single public render
 * entry point. Not re-exported from index.ts.
 */
export function formatJsonValue(value: JsonValue | Event, options: OutputOptions = {}): string {
  const unwrapped = value instanceof Event ? (value.toJSON() as JsonValue) : value;
  return canonicalJsonString(redactedValue(unwrapped, options.redaction ?? {}));
}

/**
 * Compact JSON serialization, byte-identical to `JSON.stringify(value)` for
 * every value that contains no LosslessNumber, and additionally correct for
 * values that do: `JSON.stringify` cannot serialize a LosslessNumber (it has
 * no `toJSON` and isn't a boxed Number, so it would fall back to serializing
 * its internal `{value, isLosslessNumber}` fields as if they were user data).
 * lossless-json's `stringify` recognizes LosslessNumber and re-emits its
 * exact literal text unquoted, and matches native JSON.stringify's escaping
 * and formatting for everything else.
 */
function canonicalJsonString(value: JsonValue): string {
  return losslessStringify(value) ?? "null";
}

/**
 * Format as multi-line YAML. Structure-preserving: like JSON, original keys,
 * scalar types, and numeric semantics are kept after redaction; secrets are
 * still redacted. `options.style` is ignored — YAML output does not vary by
 * PlainStyle.
 * Internal: called by `render` in cli.ts, which is the single public render
 * entry point. Not re-exported from index.ts.
 */
export function formatYamlValue(value: JsonValue | Event, options: OutputOptions = {}): string {
  const unwrapped = value instanceof Event ? (value.toJSON() as JsonValue) : value;
  const redacted = redactedValue(unwrapped, options.redaction ?? {});
  const lines = ["---"];
  renderYamlRaw(redacted, 0, lines);
  return lines.join("\n");
}

/**
 * Format as single-line logfmt. Keys stripped, values formatted, secrets redacted.
 * Internal: called by `render` in cli.ts, which is the single public render
 * entry point. Not re-exported from index.ts.
 */
export function formatPlainValue(value: JsonValue | Event, options: OutputOptions = {}): string {
  const unwrapped = value instanceof Event ? (value.toJSON() as JsonValue) : value;
  const redacted = redactedValue(unwrapped, options.redaction ?? {});
  if (!isObject(redacted)) {
    return plainScalar(redacted);
  }
  const pairs: [string, string][] = [];
  if (options.style === PlainStyle.Raw) {
    collectPlainPairsRaw(redacted, "", pairs);
  } else {
    collectPlainPairs(redacted, "", pairs);
  }
  if (pairs.length === 0) {
    return "{}";
  }
  pairs.sort(([a], [b]) => jcsCompare(a, b));
  return pairs
    .map(([k, v]) => `${quoteLogfmtKey(k)}=${quoteLogfmtValue(v)}`)
    .join(" ");
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/** Return a JSON-safe copy with redaction options applied (default: full _secret redaction). */
export function redactedValue(
  value: unknown,
  options?: {
    policy?: RedactionPolicy;
    secretNames?: readonly string[];
    urlNames?: readonly string[];
  },
): JsonValue {
  const v = sanitizeForJson(value);
  applyRedactionOptions(v, options ?? {});
  return v;
}

/**
 * Redact secret components of a single URL string.
 *
 * A query parameter is redacted iff its (form-decoded) name ends in
 * _secret/_SECRET or matches an exact entry in secretNames. A fragment written
 * in the same `k=v&k=v` shape — how an OAuth implicit-flow response carries its
 * token — is redacted by that same rule; any other fragment passes through
 * byte-for-byte. The userinfo password (scheme://user:pass@host) is always
 * redacted as a structural rule. Only the secret spans are replaced with "***";
 * every other byte is preserved. A string that is not a single,
 * whitespace-free, scheme-prefixed URL (including a URL embedded in surrounding
 * prose) is returned unchanged.
 *
 * A URL is unscoped input: `RedactionPolicy.Off` returns it unchanged, every
 * other policy redacts it in full.
 */
export function redactUrlSecrets(
  url: string,
  options?: { secretNames?: readonly string[]; policy?: RedactionPolicy },
): string {
  if (!redactsUnscopedInput(options?.policy)) return url;
  const redacted = redactUrlInStr(url, secretNameSet(options ?? {}));
  return redacted ?? url;
}

/**
 * Redact complete scheme-prefixed URL spans embedded in prose.
 *
 * This helper is explicit and URL-only: surrounding text is never scanned for
 * secret-looking names or values.
 */
export function redactUrlsInText(
  text: string,
  options?: { secretNames?: readonly string[]; policy?: RedactionPolicy },
): string {
  if (!redactsUnscopedInput(options?.policy)) return text;
  return redactUrlsInTextWithContext(text, contextFromOptions(options ?? {}));
}

/**
 * Redact secret values out of a command line.
 *
 * A long flag whose name is secret by AFDATA naming (`--api-key-secret`, or an
 * exact `secretNames` entry) has its value replaced by `***`, in both
 * `--flag=value` and `--flag value` spellings. Everything else is preserved
 * byte-for-byte.
 *
 * Free text is deliberately never scanned: a bare `api_key_secret=sk-live`
 * positional, or a secret-looking token after a non-secret flag, is left alone.
 * AFDATA decides sensitivity from the *field name*, and argv is no exception —
 * rename the flag rather than pattern-matching values. A flag with no value
 * (end of argv, or followed by another flag) is likewise left inspectable.
 *
 * Only long (`--`) flags are recognized, matching the convention's
 * long-flags-only rule.
 *
 * Intended for CLIs that record their own invocation — startup diagnostics,
 * audit trails, crash reports — where writing argv verbatim would put a
 * credential in the log.
 *
 * A command line is unscoped input: `RedactionPolicy.Off` returns `args`
 * unchanged, every other policy redacts in full.
 */
export function redactArgv(
  args: readonly string[],
  options?: { secretNames?: readonly string[]; policy?: RedactionPolicy },
): string[] {
  if (!redactsUnscopedInput(options?.policy)) return [...args];
  const secretNames = secretNameSet(options ?? {});
  const out: string[] = [];
  let redactNext = false;
  for (const arg of args) {
    if (redactNext) {
      redactNext = false;
      if (!arg.startsWith("-")) {
        out.push(REDACTED_MARKER);
        continue;
      }
    }
    if (arg.startsWith("--")) {
      const rest = arg.slice(2);
      const equals = rest.indexOf("=");
      if (equals >= 0) {
        const name = rest.slice(0, equals);
        if (isSecretFlagName(name, secretNames)) {
          out.push(`--${name}=${REDACTED_MARKER}`);
          continue;
        }
      } else if (isSecretFlagName(rest, secretNames)) {
        redactNext = true;
      }
    }
    out.push(arg);
  }
  return out;
}

/**
 * Whether a long flag's name is secret, normalizing the kebab-case flag
 * spelling to the snake_case field spelling the convention is defined in.
 */
function isSecretFlagName(flagName: string, secretNames: ReadonlySet<string>): boolean {
  const normalized = flagName.replace(/-/gu, "_");
  return isSecretKey(normalized, secretNames) || isSecretKey(flagName, secretNames);
}

/**
 * Normalize a fixed UTC offset string to "UTC" or ±HH:MM.
 * This helper handles fixed offsets only; IANA timezone names and DST rules
 * are intentionally out of scope.
 */
export function normalizeUtcOffset(value: string): string | null {
  const s = value.trim();
  if (s.toLowerCase() === "utc" || s.toLowerCase() === "z") return "UTC";
  if (!s || (s[0] !== "+" && s[0] !== "-")) return null;
  const parsed = parseUtcOffsetBody(s.slice(1));
  if (parsed === null) return null;
  const [hours, minutes] = parsed;
  if (hours > 23 || minutes > 59) return null;
  if (hours === 0 && minutes === 0) return "UTC";
  return `${s[0]}${hours.toString().padStart(2, "0")}:${minutes.toString().padStart(2, "0")}`;
}

/** Return true when value is an RFC 3339 full-date (YYYY-MM-DD). */
export function isValidRfc3339Date(value: string): boolean {
  if (value.length !== 10 || value[4] !== "-" || value[7] !== "-") return false;
  const year = parseFixedDigits(value.slice(0, 4));
  const month = parseFixedDigits(value.slice(5, 7));
  const day = parseFixedDigits(value.slice(8, 10));
  if (year === null || month === null || day === null) return false;
  return month >= 1 && month <= 12 && day >= 1 && day <= daysInMonth(year, month);
}

/** Return true when value is an RFC 3339 partial-time (HH:MM:SS[.fraction]). */
export function isValidRfc3339Time(value: string): boolean {
  if (value.length < 8 || value[2] !== ":" || value[5] !== ":") return false;
  const hour = parseFixedDigits(value.slice(0, 2));
  const minute = parseFixedDigits(value.slice(3, 5));
  const second = parseFixedDigits(value.slice(6, 8));
  if (hour === null || minute === null || second === null) return false;
  if (hour > 23 || minute > 59 || second > 59) return false;
  if (value.length === 8) return true;
  return value[8] === "." && value.length > 9 && /^[0-9]+$/.test(value.slice(9));
}

/**
 * Return true when value is a complete RFC 3339 date-time, such as
 * 2026-02-14T10:30:00Z or 2026-02-14T10:30:00.5+08:00. Composed from
 * isValidRfc3339Date and isValidRfc3339Time: a full-date, a T/t separator, a
 * partial-time (with optional fractional seconds), and a mandatory time-offset
 * (Z/z or ±HH:MM with HH in 00..23 and MM in 00..59). The offset is required, so
 * a bare 2026-02-14T10:30:00 is rejected; a space separator is rejected; and a
 * leap second (:60) is rejected, matching isValidRfc3339Time. Non-ASCII is rejected.
 */
export function isValidRfc3339(value: string): boolean {
  if (value.length < 20 || !/^[\x00-\x7F]*$/.test(value)) return false;
  if (!isValidRfc3339Date(value.slice(0, 10))) return false;
  if (value[10] !== "T" && value[10] !== "t") return false;
  const rest = value.slice(11);
  const last = rest[rest.length - 1];
  let partial: string;
  if (last === "Z" || last === "z") {
    partial = rest.slice(0, -1);
  } else {
    if (rest.length < 6 || !isRfc3339NumOffset(rest.slice(-6))) return false;
    partial = rest.slice(0, -6);
  }
  return isValidRfc3339Time(partial);
}

function isRfc3339NumOffset(offset: string): boolean {
  if (offset.length !== 6 || (offset[0] !== "+" && offset[0] !== "-") || offset[3] !== ":") {
    return false;
  }
  const hours = parseFixedDigits(offset.slice(1, 3));
  const minutes = parseFixedDigits(offset.slice(4, 6));
  if (hours === null || minutes === null) return false;
  return hours <= 23 && minutes <= 59;
}

/**
 * Return true when value is a structurally well-formed BCP 47 (RFC 5646) language tag.
 * A grammar-level check, not a registry lookup: hyphen-separated ASCII-alphanumeric
 * subtags (each 1-8 chars) whose primary subtag is a 2-3 letter language code or the
 * x/i privateuse/grandfathered lead. Rejects the POSIX underscore form (zh_CN), empty
 * or misplaced hyphens, non-ASCII, and out-of-range primaries such as chinese. Does not
 * verify that subtags are registered with IANA.
 */
export function isValidBcp47(value: string): boolean {
  if (!value) return false;
  const subtags = value.split("-");
  for (let index = 0; index < subtags.length; index++) {
    const subtag = subtags[index];
    if (subtag.length < 1 || subtag.length > 8 || !/^[A-Za-z0-9]+$/.test(subtag)) {
      return false;
    }
    if (index === 0) {
      const isLanguage = subtag.length >= 2 && subtag.length <= 3 && /^[A-Za-z]+$/.test(subtag);
      const isSpecial = subtag === "x" || subtag === "i";
      if (!isLanguage && !isSpecial) return false;
    }
  }
  return true;
}

function parseUtcOffsetBody(body: string): [number, number] | null {
  if (!body) return null;
  if (body.includes(":")) {
    const parts = body.split(":");
    if (parts.length !== 2) return null;
    const [hours, minutes] = parts;
    if (!hours || hours.length > 2 || minutes.length !== 2) return null;
    if (!/^[0-9]+$/.test(hours) || !/^[0-9]+$/.test(minutes)) return null;
    return [Number(hours), Number(minutes)];
  }
  if (!/^[0-9]+$/.test(body)) return null;
  if (body.length === 1 || body.length === 2) return [Number(body), 0];
  if (body.length === 4) return [Number(body.slice(0, 2)), Number(body.slice(2))];
  return null;
}

function parseFixedDigits(value: string): number | null {
  if (!/^[0-9]+$/.test(value)) return null;
  return Number(value);
}

function daysInMonth(year: number, month: number): number {
  if ([1, 3, 5, 7, 8, 10, 12].includes(month)) return 31;
  if ([4, 6, 9, 11].includes(month)) return 30;
  if (month === 2) return isLeapYear(year) ? 29 : 28;
  return 0;
}

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

/**
 * The scalar every redacted span, value, and subtree is replaced with. Also the
 * signal plain rendering reads to tell a hidden field from a live one.
 */
const REDACTED_MARKER = "***";

/**
 * True when a field already carries the redaction marker, i.e. redaction ran
 * over it and hid the value.
 */
function isRedacted(value: JsonValue): boolean {
  return value === REDACTED_MARKER;
}

/** Internal shape shared by the inlined redaction option parameters. */
type RedactionOpts = {
  policy?: RedactionPolicy;
  secretNames?: readonly string[];
  urlNames?: readonly string[];
};

type RedactionContext = {
  secretNames: ReadonlySet<string>;
  urlNames: ReadonlySet<string>;
};

const DEFAULT_CONTEXT: RedactionContext = {
  secretNames: new Set<string>(),
  urlNames: new Set<string>(),
};

function secretNameSet(redactionOptions: RedactionOpts): ReadonlySet<string> {
  return new Set(redactionOptions.secretNames ?? []);
}

function contextFromOptions(redactionOptions: RedactionOpts): RedactionContext {
  return {
    secretNames: secretNameSet(redactionOptions),
    urlNames: new Set(redactionOptions.urlNames ?? []),
  };
}

function keyHasSecretSuffix(key: string): boolean {
  return key.endsWith("_secret") || key.endsWith("_SECRET");
}

function keyHasUrlSuffix(key: string): boolean {
  return key.endsWith("_url") || key.endsWith("_URL");
}

function isSecretKey(key: string, secretNames: ReadonlySet<string>): boolean {
  return keyHasSecretSuffix(key) || secretNames.has(key);
}

function isUrlKey(key: string, context: RedactionContext): boolean {
  return keyHasUrlSuffix(key) || context.urlNames.has(key);
}

const MAX_DEPTH = 256;
const MAX_DEPTH_MARKER = "<afdata:max-depth>";

function redactSecrets(value: JsonValue, context: RedactionContext = DEFAULT_CONTEXT, depth = 0): void {
  if (depth >= MAX_DEPTH) return;
  if (isObject(value)) {
    for (const k of Object.keys(value)) {
      const v = value[k];
      if (isSecretKey(k, context.secretNames)) {
        // A null secret is an absent secret. Masking it would manufacture the
        // appearance of a configured credential — readers cannot tell
        // "***"-because-set from "***"-because-null. Redaction hides a value
        // that exists; it does not invent one.
        if (v !== null) {
          value[k] = REDACTED_MARKER;
        }
      } else if (isUrlKey(k, context)) {
        value[k] = redactUrlValue(v, context, depth + 1);
      } else {
        value[k] = depth + 1 >= MAX_DEPTH ? MAX_DEPTH_MARKER : (redactSecrets(v, context, depth + 1), v);
      }
    }
  } else if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      if (depth + 1 >= MAX_DEPTH) value[i] = MAX_DEPTH_MARKER;
      else redactSecrets(value[i], context, depth + 1);
    }
  }
}

function redactUrlValue(value: JsonValue, context: RedactionContext, depth: number): JsonValue {
  if (depth >= MAX_DEPTH) return MAX_DEPTH_MARKER;
  if (typeof value === "string") {
    return redactUrlFieldValue(value, context.secretNames);
  }
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index++) {
      value[index] = redactUrlValue(value[index], context, depth + 1);
    }
    return value;
  }
  if (isObject(value)) {
    for (const key of Object.keys(value)) {
      const item = value[key];
      if (isSecretKey(key, context.secretNames)) {
        if (item !== null) value[key] = REDACTED_MARKER;
      } else {
        value[key] = redactUrlValue(item, context, depth + 1);
      }
    }
  }
  return value;
}

function applyRedactionOptions(value: JsonValue, redactionOptions: RedactionOpts): void {
  applyRedactionPolicyWithContext(value, redactionOptions.policy, contextFromOptions(redactionOptions));
}

function applyRedactionPolicyWithContext(
  value: JsonValue,
  redactionPolicy: RedactionPolicy | undefined,
  context: RedactionContext,
): void {
  switch (redactionPolicy) {
    case RedactionPolicy.TraceOnly:
      if (isObject(value) && value.trace !== undefined) {
        redactSecrets(value.trace, context);
      }
      break;
    case RedactionPolicy.Off:
      break;
    default:
      // undefined or RedactionPolicy.All = full redaction (default).
      redactSecrets(value, context);
      break;
  }
}

// ═══════════════════════════════════════════
// URL Secret Redaction
// ═══════════════════════════════════════════

/**
 * Redact secret components of a single URL string, returning the redacted URL
 * when s is a processable URL, or null when it is not (so callers can keep the
 * original). Only secret spans change; all other bytes are preserved.
 */
function redactUrlInStr(s: string, secretNames: ReadonlySet<string>): string | null {
  // Precondition (spec): a single, whitespace-free, scheme-prefixed URL.
  // The gate is scheme + no-whitespace only — NOT "parses via `new URL`". Span
  // location below is purely byte-wise (we never re-serialize the URL), so a
  // `new URL` gate would only diverge across languages — it rejects inputs
  // (ports > 65535, empty host) that other languages' URL libraries accept, and
  // rejecting here would silently leak secrets in the values it rejects.
  if (!s.includes("://") || !isSingleUrl(s)) {
    return null;
  }
  const schemeSep = s.indexOf("://");
  const scheme = s.slice(0, schemeSep);
  const rest = s.slice(schemeSep + 3);

  // Authority runs from after "://" to the first '/', '?', or '#'.
  let authEnd = rest.length;
  for (let i = 0; i < rest.length; i++) {
    const c = rest[i];
    if (c === "/" || c === "?" || c === "#") {
      authEnd = i;
      break;
    }
  }
  const authority = rest.slice(0, authEnd);
  const remainder = rest.slice(authEnd);

  const newAuthority = redactUserinfoPassword(authority);

  // `remainder` is `path[?query][#fragment]`; '#' ends the query, so split the
  // fragment off first and the query out of what is left.
  const hash = remainder.indexOf("#");
  const beforeFragment = hash >= 0 ? remainder.slice(0, hash) : remainder;
  const fragment = hash >= 0 ? remainder.slice(hash + 1) : null;

  const q = beforeFragment.indexOf("?");
  const newBeforeFragment =
    q >= 0
      ? `${beforeFragment.slice(0, q)}?${redactQuery(beforeFragment.slice(q + 1), secretNames)}`
      : beforeFragment;
  // A fragment gets the same treatment as the query: `k=v&k=v` after the '#' is
  // exactly how an OAuth implicit-flow response hands back a token, so a
  // secret-named fragment parameter must not survive where the identically
  // named query parameter would not. A fragment that is not in that shape has
  // no '=' in its segments and passes through byte-for-byte.
  const newFragment = fragment === null ? "" : `#${redactQuery(fragment, secretNames)}`;

  return `${scheme}://${newAuthority}${newBeforeFragment}${newFragment}`;
}

function redactUrlsInTextWithContext(text: string, context: RedactionContext): string {
  const output: string[] = [];
  let copiedThrough = 0;
  let searchFrom = 0;
  while (true) {
    const span = nextSchemeUrlSpan(text, searchFrom);
    if (span === null) break;
    const [start, end] = span;
    output.push(text.slice(copiedThrough, start));
    const url = text.slice(start, end);
    output.push(redactUrlInStr(url, context.secretNames) ?? url);
    copiedThrough = end;
    searchFrom = end;
  }
  if (output.length === 0) return text;
  output.push(text.slice(copiedThrough));
  return output.join("");
}

function nextSchemeUrlSpan(text: string, startAt: number): [number, number] | null {
  let start = startAt;
  while (start < text.length) {
    // Only consider the head of a maximal scheme-byte run: a scheme cannot begin
    // mid-run, so a position whose predecessor is a scheme byte was already
    // covered by the run that contains it.
    const atRunHead = start === 0 || !isSchemeCodeUnit(text.charCodeAt(start - 1));
    if (atRunHead) {
      let schemeEnd = start;
      while (schemeEnd < text.length && isSchemeCodeUnit(text.charCodeAt(schemeEnd))) {
        schemeEnd += 1;
      }
      // A scheme must start with a letter, but the run need not: prose like
      // "2https://h/?token_secret=x" glues a digit onto the URL. Retry from the
      // first letter *inside* the run instead of abandoning the span — giving up
      // there fails open and leaks the whole query.
      let schemeStart = -1;
      for (let i = start; i < schemeEnd; i++) {
        if (isAsciiAlpha(text.charCodeAt(i))) {
          schemeStart = i;
          break;
        }
      }
      if (schemeStart >= 0 && text.startsWith("://", schemeEnd)) {
        let end = schemeEnd + 3;
        while (
          end < text.length
          && !isSpanTerminatorWhitespace(text.charCodeAt(end))
          && !isTextUrlDelimiter(text[end])
        ) {
          end += 1;
        }
        while (
          end > schemeEnd + 3
          && isTrailingTextUrlPunctuation(text[end - 1])
        ) {
          end -= 1;
        }
        if (end > schemeEnd + 3) return [schemeStart, end];
      }
    }
    start += 1;
  }
  return null;
}

function isSchemeCodeUnit(code: number): boolean {
  return isAsciiAlphanumeric(code) || code === 0x2b || code === 0x2d || code === 0x2e;
}

/**
 * True for a code unit that ends a URL span in prose: the Unicode `White_Space`
 * property, written out here instead of delegated to `/\s/u`.
 *
 * The four AFDATA implementations must cut the span at the same byte, and each
 * language's native predicate covers a different set — `\s` counts U+FEFF but
 * not U+0085, Python's `str.isspace()` counts U+001C–U+001F, Rust and Go count
 * neither. Divergence is a defect in both directions: ending a span early leaves
 * the rest of the query readable, and ending it late pulls the following prose
 * into the URL, where redacting the parameter it lands in deletes it. So the set
 * is enumerated, and it is generous — a redaction helper exists to keep a log
 * line readable, so anything a human reads as a space must end the URL.
 *
 * Two exclusions are deliberate, not oversights:
 * - **U+FEFF** (zero-width no-break space) is not `White_Space`. `\s` counts it,
 *   which is what used to cut a URL carrying it short and leave the secret after
 *   it in the clear. It must not creep back in.
 * - **U+001C–U+001F** (the file/group/record/unit separators) are not
 *   `White_Space` either; only Python's `str.isspace()` counted them.
 *
 * This is a different question from `isSingleUrl`, which asks whether a whole
 * string is one URL and stays ASCII-only. A span this scanner produces contains
 * no whitespace at all, so it satisfies that stricter gate either way.
 */
function isSpanTerminatorWhitespace(code: number): boolean {
  return (
    // TAB, LF, VT, FF, CR, SPACE
    (code >= 0x09 && code <= 0x0d)
    || code === 0x20
    // NEL, NBSP, OGHAM SPACE MARK
    || code === 0x85
    || code === 0xa0
    || code === 0x1680
    // EN QUAD .. HAIR SPACE
    || (code >= 0x2000 && code <= 0x200a)
    // LINE SEPARATOR, PARAGRAPH SEPARATOR, NARROW NO-BREAK SPACE,
    // MEDIUM MATHEMATICAL SPACE, IDEOGRAPHIC SPACE
    || code === 0x2028
    || code === 0x2029
    || code === 0x202f
    || code === 0x205f
    || code === 0x3000
  );
}

function isTextUrlDelimiter(character: string): boolean {
  return "\"'<>`，。；！？）》】」』".includes(character);
}

function isTrailingTextUrlPunctuation(character: string): boolean {
  return ".,;!)]}，。；！）》】".includes(character);
}

function redactUrlFieldValue(s: string, secretNames: ReadonlySet<string>): string {
  const redacted = redactUrlInStr(s, secretNames);
  if (redacted !== null) return redacted;
  const trimmed = s.trim();
  if (trimmed !== s) {
    const trimmedRedacted = redactUrlInStr(trimmed, secretNames);
    if (trimmedRedacted !== null) return trimmedRedacted;
  }
  // Fail closed: a _url value we could not parse as a clean scheme-prefixed
  // URL, yet which carries a credential sigil ('@' userinfo) or internal
  // whitespace, is redacted wholesale rather than passed through. A schemeless
  // connection string like user:pass@host/db has no scheme anchor for the
  // surgical span logic above, so blanket redaction is the safe default.
  if (/\s/.test(s) || s.includes("@")) return REDACTED_MARKER;
  return s;
}

/**
 * Replace the userinfo password (user:pass@) with "***", preserving the
 * username. Authority without '@', or userinfo without ':', is unchanged.
 */
function redactUserinfoPassword(authority: string): string {
  const at = authority.lastIndexOf("@");
  if (at < 0) return authority;
  const userinfo = authority.slice(0, at);
  const colon = userinfo.indexOf(":");
  if (colon < 0) return authority;
  return `${authority.slice(0, colon)}:${REDACTED_MARKER}${authority.slice(at)}`;
}

/**
 * Redact the values of secret-named query parameters, preserving raw bytes of
 * every other segment (keys, benign values, encoding, ordering, separators).
 */
function redactQuery(query: string, secretNames: ReadonlySet<string>): string {
  return query
    .split("&")
    .map((segment) => {
      const eq = segment.indexOf("=");
      if (eq < 0) return segment;
      const rawKey = segment.slice(0, eq);
      const name = formDecode(rawKey);
      if (isSecretKey(name, secretNames)) {
        return `${rawKey}=${REDACTED_MARKER}`;
      }
      return segment;
    })
    .join("&");
}

/** Form-decode a query-parameter name: '+' → space, then percent-decode. */
function formDecode(rawKey: string): string {
  const withSpaces = rawKey.replace(/\+/g, " ");
  try {
    return decodeURIComponent(withSpaces);
  } catch {
    return withSpaces;
  }
}

/**
 * True when s begins with a URL scheme (ALPHA *(ALPHA / DIGIT / "+" / "-" /
 * ".") "://") and contains no ASCII whitespace — i.e. a single bare URL, not a
 * URL embedded in prose.
 */
function isSingleUrl(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    // ASCII whitespace: tab, LF, VT, FF, CR, space.
    if (c === 0x09 || c === 0x0a || c === 0x0b || c === 0x0c || c === 0x0d || c === 0x20) {
      return false;
    }
  }
  if (s.length === 0) return false;
  const first = s.charCodeAt(0);
  if (!isAsciiAlpha(first)) return false;
  let i = 1;
  while (i < s.length) {
    const c = s.charCodeAt(i);
    if (isAsciiAlphanumeric(c) || c === 0x2b /* + */ || c === 0x2d /* - */ || c === 0x2e /* . */) {
      i += 1;
    } else {
      break;
    }
  }
  return s.slice(i).startsWith("://");
}

function isAsciiAlpha(c: number): boolean {
  return (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a);
}

function isAsciiAlphanumeric(c: number): boolean {
  return isAsciiAlpha(c) || (c >= 0x30 && c <= 0x39);
}

// ═══════════════════════════════════════════
// Suffix Processing
// ═══════════════════════════════════════════

function stripSuffixCI(key: string, suffixLower: string): string | null {
  if (key.endsWith(suffixLower)) return key.slice(0, -suffixLower.length);
  const suffixUpper = suffixLower.toUpperCase();
  if (key.endsWith(suffixUpper)) return key.slice(0, -suffixUpper.length);
  return null;
}

function tryStripGenericCents(key: string): [string, string] | null {
  const code = extractCurrencyCode(key);
  if (code === null) return null;
  const suffixLen = code.length + "_cents".length + 1; // _{code}_cents
  const stripped = key.slice(0, -suffixLen);
  if (!stripped) return null;
  return [stripped, code];
}

function tryStripGenericMicro(key: string): [string, string] | null {
  const code = extractCurrencyCodeMicro(key);
  if (code === null) return null;
  const suffixLen = code.length + "_micro".length + 1; // _{code}_micro
  const stripped = key.slice(0, -suffixLen);
  if (!stripped) return null;
  return [stripped, code];
}

function isInt(value: JsonValue): value is number {
  return typeof value === "number" && Number.isInteger(value);
}

function decimalIntText(value: JsonValue): string | null {
  if (typeof value === "string" && /^-?\d+$/.test(value)) return value;
  if (typeof value === "bigint") return value.toString();
  if (isInt(value) && Number.isSafeInteger(value)) return String(value);
  if (isLosslessNumber(value)) {
    const text = value.toString();
    return /^-?\d+$/.test(text) ? text : null;
  }
  return null;
}

const MIN_I64 = -9_223_372_036_854_775_808n;
const MAX_I64 = 9_223_372_036_854_775_807n;

function signedI64(value: JsonValue): bigint | null {
  let text: string | null = null;
  if (typeof value === "bigint") {
    text = value.toString();
  } else if (typeof value === "number" && Number.isSafeInteger(value)) {
    text = String(value);
  } else if (isLosslessNumber(value)) {
    const literal = value.toString();
    if (/^-?\d+$/.test(literal)) text = literal;
  }
  if (text === null) return null;
  try {
    const integer = BigInt(text);
    return integer >= MIN_I64 && integer <= MAX_I64 ? integer : null;
  } catch {
    return null;
  }
}

function formatCents(value: bigint, symbol: string): string {
  const sign = value < 0n ? "-" : "";
  const magnitude = value < 0n ? -value : value;
  return `${sign}${symbol}${magnitude / 100n}.${String(magnitude % 100n).padStart(2, "0")}`;
}

function epochNsToMs(value: JsonValue): number | null {
  const text = decimalIntText(value);
  if (text === null) return null;
  try {
    const ns = BigInt(text);
    const divisor = 1_000_000n;
    let ms = ns / divisor;
    if (ns < 0n && ns % divisor !== 0n) ms -= 1n;
    const asNumber = Number(ms);
    if (!Number.isSafeInteger(asNumber)) return null;
    return asNumber;
  } catch {
    return null;
  }
}

function isNum(value: JsonValue): value is number {
  return typeof value === "number";
}

function tryProcessField(key: string, value: JsonValue): [string, string] | null {
  let stripped: string | null;

  // Plain-format suffix processing below does real arithmetic (division,
  // modulo, Math.floor/abs) that only works on a native JS number. A decoded
  // LosslessNumber is normalized to one here via Number() \u2014 which never
  // throws, unlike LosslessNumber's own valueOf() (documented to throw for
  // values it can't safely represent as number/bigint) \u2014 so an ordinary
  // decoded value (e.g. duration_ms: 42) keeps formatting exactly as before,
  // and an exotic decoded value degrades gracefully (loses precision here,
  // same as it always would for a >2^53 native number) rather than crashing.
  // This is Plain's documented lossy/human path; JSON/YAML stay exact via
  // yamlScalar/plainScalar/canonicalJsonString, and decimalIntText below
  // (msats/sats/epoch_ns) keeps exactness by reading `value` directly instead
  // of `numeric`.
  const numeric: JsonValue = isLosslessNumber(value) ? Number(value.toString()) : value;

  // Group 1: compound timestamp suffixes
  stripped = stripSuffixCI(key, "_epoch_ms");
  if (stripped !== null) {
    if (isInt(numeric)) { const formatted = formatRfc3339Ms(numeric); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_s");
  if (stripped !== null) {
    if (isInt(numeric)) { const formatted = formatRfc3339Ms(numeric * 1000); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_ns");
  if (stripped !== null) {
    const ms = epochNsToMs(value);
    if (ms !== null) { const formatted = formatRfc3339Ms(ms); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }

  // Group 2: compound currency suffixes
  stripped = stripSuffixCI(key, "_usd_cents");
  if (stripped !== null) {
    const integer = signedI64(value);
    if (integer !== null) return [stripped, formatCents(integer, "$")];
    return null;
  }
  stripped = stripSuffixCI(key, "_eur_cents");
  if (stripped !== null) {
    const integer = signedI64(value);
    if (integer !== null) return [stripped, formatCents(integer, "\u20ac")];
    return null;
  }
  const gc = tryStripGenericCents(key);
  if (gc !== null) {
    const [gcStripped, code] = gc;
    const integer = signedI64(value);
    if (integer !== null) {
      const sign = integer < 0n ? "-" : "";
      const magnitude = integer < 0n ? -integer : integer;
      return [gcStripped, `${sign}${magnitude / 100n}.${String(magnitude % 100n).padStart(2, "0")} ${code.toUpperCase()}`];
    }
    return null;
  }
  const gm = tryStripGenericMicro(key);
  if (gm !== null) {
    const [gmStripped, code] = gm;
    const integer = signedI64(value);
    if (integer !== null) {
      const sign = integer < 0n ? "-" : "";
      const magnitude = integer < 0n ? -integer : integer;
      return [gmStripped, `${sign}${magnitude / 1_000_000n}.${String(magnitude % 1_000_000n).padStart(6, "0")} ${code.toUpperCase()}`];
    }
    return null;
  }

  // Group 3: multi-char suffixes
  stripped = stripSuffixCI(key, "_rfc3339");
  if (stripped !== null) {
    if (typeof value === "string") return [stripped, value];
    return null;
  }
  stripped = stripSuffixCI(key, "_minutes");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)} minutes`];
    return null;
  }
  stripped = stripSuffixCI(key, "_hours");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)} hours`];
    return null;
  }
  stripped = stripSuffixCI(key, "_days");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)} days`];
    return null;
  }

  // Group 4: single-unit suffixes
  stripped = stripSuffixCI(key, "_msats");
  if (stripped !== null) {
    const text = decimalIntText(value);
    if (text !== null) return [stripped, `${text}msats`];
    return null;
  }
  stripped = stripSuffixCI(key, "_sats");
  if (stripped !== null) {
    const text = decimalIntText(value);
    if (text !== null) return [stripped, `${text}sats`];
    return null;
  }
  stripped = stripSuffixCI(key, "_bytes");
  if (stripped !== null) {
    if (isInt(numeric) && numeric >= 0) return [stripped, formatBytesHuman(numeric)];
    return null;
  }
  stripped = stripSuffixCI(key, "_percent");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)}%`];
    return null;
  }
  // Group 5: short suffixes (last to avoid false positives)
  stripped = stripSuffixCI(key, "_jpy");
  if (stripped !== null) {
    const integer = signedI64(value);
    if (integer !== null) {
      const sign = integer < 0n ? "-" : "";
      const magnitude = integer < 0n ? -integer : integer;
      return [stripped, `${sign}\u00a5${formatWithCommas(magnitude)}`];
    }
    return null;
  }
  stripped = stripSuffixCI(key, "_ns");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)}ns`];
    return null;
  }
  stripped = stripSuffixCI(key, "_us");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)}\u03bcs`];
    return null;
  }
  stripped = stripSuffixCI(key, "_ms");
  if (stripped !== null) {
    const fv = formatMsValue(numeric);
    if (fv !== null) return [stripped, fv];
    return null;
  }
  stripped = stripSuffixCI(key, "_s");
  if (stripped !== null) {
    if (isNum(numeric)) return [stripped, `${plainScalar(numeric)}s`];
    return null;
  }

  return null;
}

type ProcessedField = {
  key: string;
  value: JsonValue;
  formatted: string | null;
};

function processObjectFields(obj: { [key: string]: JsonValue }): ProcessedField[] {
  const entries: { stripped: string; original: string; value: JsonValue; formatted: string | null }[] = [];
  for (const [k, v] of Object.entries(obj)) {
    const secretStripped = stripSuffixCI(k, "_secret");
    if (secretStripped !== null) {
      // Dropping `_secret` is the readable half of an actual redaction: the
      // marker has done its job once the value reads `***`. When the value was
      // *not* redacted — policy `Off`, or a field outside `trace` under
      // `TraceOnly` — the suffix is the only thing telling a downstream reader
      // that `api_key_secret=sk-live-xxx` is a credential, so it stays.
      const displayKey = isRedacted(v) ? secretStripped : k;
      entries.push({ stripped: displayKey, original: k, value: v, formatted: null });
      continue;
    }
    const result = tryProcessField(k, v);
    if (result !== null) {
      entries.push({ stripped: result[0], original: k, value: v, formatted: result[1] });
    } else {
      entries.push({ stripped: k, original: k, value: v, formatted: null });
    }
  }

  // Detect collisions
  const counts = new Map<string, number>();
  for (const e of entries) {
    counts.set(e.stripped, (counts.get(e.stripped) ?? 0) + 1);
  }

  // Resolve collisions: revert both key and formatted value
  const result: ProcessedField[] = entries.map((e) => {
    if ((counts.get(e.stripped) ?? 0) > 1 && e.original !== e.stripped) {
      return { key: e.original, value: e.value, formatted: null };
    }
    return { key: e.stripped, value: e.value, formatted: e.formatted };
  });

  // Sort by display key (JCS order)
  result.sort((a, b) => jcsCompare(a.key, b.key));
  return result;
}

// ═══════════════════════════════════════════
// Formatting Helpers
// ═══════════════════════════════════════════

function formatMsAsSeconds(ms: number): string {
  const formatted = (ms / 1000).toFixed(3);
  const trimmed = formatted.replace(/0+$/, "");
  if (trimmed.endsWith(".")) return trimmed + "0s";
  return trimmed + "s";
}

function formatMsValue(value: JsonValue): string | null {
  if (!isNum(value)) return null;
  if (Math.abs(value) >= 1000) return formatMsAsSeconds(value);
  return `${plainScalar(value)}ms`;
}

const MIN_RFC3339_MS = -62135596800000;
const MAX_RFC3339_MS = 253402300799999;

function formatRfc3339Ms(ms: number): string | null {
  if (ms < MIN_RFC3339_MS || ms > MAX_RFC3339_MS) return null;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return null;
  return d.toISOString().replace(/(\.\d{3})\d*Z$/, "$1Z");
}

/**
 * Round to `digits` decimals using round-half-to-even (banker's rounding) and
 * format with a fixed decimal count. `Number.toFixed` rounds half away from
 * zero, which diverges from the printf `%.1f` used by Rust/Go/Python on exact
 * ties (e.g. 1280 bytes = 1.25 KiB → "1.2KiB", not "1.3KiB").
 */
function toFixedHalfEven(x: number, digits: number): string {
  const factor = 10 ** digits;
  const scaled = x * factor;
  const floor = Math.floor(scaled);
  const diff = scaled - floor;
  const eps = 1e-9;
  let rounded: number;
  if (diff > 0.5 + eps) rounded = floor + 1;
  else if (diff < 0.5 - eps) rounded = floor;
  else rounded = floor % 2 === 0 ? floor : floor + 1;
  return (rounded / factor).toFixed(digits);
}

function formatBytesHuman(bytes: number): string {
  const KiB = 1024;
  const MiB = KiB * 1024;
  const GiB = MiB * 1024;
  const TiB = GiB * 1024;
  const b = bytes;
  if (b >= TiB) return `${toFixedHalfEven(b / TiB, 1)}TiB`;
  if (b >= GiB) return `${toFixedHalfEven(b / GiB, 1)}GiB`;
  if (b >= MiB) return `${toFixedHalfEven(b / MiB, 1)}MiB`;
  if (b >= KiB) return `${toFixedHalfEven(b / KiB, 1)}KiB`;
  return `${bytes}B`;
}

function formatWithCommas(n: bigint): string {
  const s = String(n);
  const result: string[] = [];
  for (let i = 0; i < s.length; i++) {
    if (i > 0 && (s.length - i) % 3 === 0) result.push(",");
    result.push(s[i]);
  }
  return result.join("");
}

/** Extract currency code from _{code}_cents / _{CODE}_CENTS suffix. */
function extractCurrencyCode(key: string): string | null {
  let withoutCents: string;
  if (key.endsWith("_cents")) withoutCents = key.slice(0, -6);
  else if (key.endsWith("_CENTS")) withoutCents = key.slice(0, -6);
  else return null;
  return extractCurrencyCodeFromStem(withoutCents);
}

/** Extract currency code from _{code}_micro / _{CODE}_MICRO suffix. */
function extractCurrencyCodeMicro(key: string): string | null {
  let withoutMicro: string;
  if (key.endsWith("_micro")) withoutMicro = key.slice(0, -6);
  else if (key.endsWith("_MICRO")) withoutMicro = key.slice(0, -6);
  else return null;
  return extractCurrencyCodeFromStem(withoutMicro);
}

function extractCurrencyCodeFromStem(stem: string): string | null {
  const idx = stem.lastIndexOf("_");
  if (idx < 0) return null;
  const code = stem.slice(idx + 1);
  if (!/^[A-Za-z]{3,4}$/.test(code)) return null;
  return code;
}

// ═══════════════════════════════════════════
// YAML Rendering
// ═══════════════════════════════════════════

function renderYamlRaw(value: JsonValue, indent: number, lines: string[]): void {
  const prefix = "  ".repeat(indent);
  if (isObject(value)) {
    for (const key of sortedObjectKeys(value)) {
      renderYamlFieldRaw(prefix, key, value[key], indent, lines);
    }
  } else if (Array.isArray(value)) {
    renderYamlArrayRaw(value, indent, lines);
  } else {
    lines.push(`${prefix}${yamlScalar(value)}`);
  }
}

function renderYamlFieldRaw(prefix: string, key: string, value: JsonValue, indent: number, lines: string[]): void {
  if (isObject(value)) {
    if (Object.keys(value).length > 0) {
      lines.push(`${prefix}${yamlKey(key)}:`);
      renderYamlRaw(value, indent + 1, lines);
    } else {
      lines.push(`${prefix}${yamlKey(key)}: {}`);
    }
  } else if (Array.isArray(value)) {
    if (value.length > 0) {
      lines.push(`${prefix}${yamlKey(key)}:`);
      renderYamlArrayRaw(value, indent + 1, lines);
    } else {
      lines.push(`${prefix}${yamlKey(key)}: []`);
    }
  } else {
    lines.push(`${prefix}${yamlKey(key)}: ${yamlScalar(value)}`);
  }
}

function renderYamlArrayRaw(arr: JsonValue[], indent: number, lines: string[]): void {
  const prefix = "  ".repeat(indent);
  for (const item of arr) {
    if (isObject(item)) {
      if (Object.keys(item).length > 0) {
        lines.push(`${prefix}-`);
        renderYamlRaw(item, indent + 1, lines);
      } else {
        lines.push(`${prefix}- {}`);
      }
    } else if (Array.isArray(item)) {
      if (item.length > 0) {
        lines.push(`${prefix}-`);
        renderYamlArrayRaw(item, indent + 1, lines);
      } else {
        lines.push(`${prefix}- []`);
      }
    } else {
      lines.push(`${prefix}- ${yamlScalar(item)}`);
    }
  }
}

function escapeYamlStr(s: string): string {
  return s
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/[\x00-\x1f]/g, (character) => {
      switch (character) {
        case "\0": return "\\0";
        case "\b": return "\\b";
        case "\t": return "\\t";
        case "\n": return "\\n";
        case "\v": return "\\v";
        case "\f": return "\\f";
        case "\r": return "\\r";
        default: return `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`;
      }
    });
}

function yamlKey(key: string): string {
  return /^[A-Za-z0-9_.-]+$/.test(key) && !isAmbiguousYamlKey(key)
    ? key
    : `"${escapeYamlStr(key)}"`;
}

function isAmbiguousYamlKey(key: string): boolean {
  if (["true", "false", "null", "~", ".nan", ".inf", "+.inf", "-.inf"].includes(key.toLowerCase())) {
    return true;
  }
  return key.trim() !== "" && Number.isFinite(Number(key));
}

function yamlScalar(value: JsonValue): string {
  if (typeof value === "string") return `"${escapeYamlStr(value)}"`;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  if (typeof value === "bigint") return value.toString();
  // Decoded number: emit the exact source literal unquoted, matching JSON's
  // structure-preserving fidelity contract for YAML too.
  if (isLosslessNumber(value)) return value.toString();
  return `"${escapeYamlStr(canonicalJsonString(sortJsonValue(value)))}"`;
}


// ═══════════════════════════════════════════
// Plain Rendering (logfmt)
// ═══════════════════════════════════════════

function collectPlainPairs(value: JsonValue, prefix: string, pairs: [string, string][]): void {
  if (!isObject(value)) return;
  for (const pf of processObjectFields(value)) {
    const fullKey = prefix ? `${prefix}.${pf.key}` : pf.key;
    if (pf.formatted !== null) {
      pairs.push([fullKey, pf.formatted]);
    } else if (isObject(pf.value)) {
      if (Object.keys(pf.value).length === 0) pairs.push([fullKey, "{}"]);
      else collectPlainPairs(pf.value, fullKey, pairs);
    } else if (Array.isArray(pf.value)) {
      pairs.push([fullKey, pf.value.length === 0 ? "[]" : pf.value.map((i) => plainScalar(i)).join(",")]);
    } else if (pf.value === null) {
      pairs.push([fullKey, ""]);
    } else {
      pairs.push([fullKey, plainScalar(pf.value)]);
    }
  }
}

function collectPlainPairsRaw(value: JsonValue, prefix: string, pairs: [string, string][]): void {
  if (!isObject(value)) return;
  for (const key of sortedObjectKeys(value)) {
    const v = value[key];
    const fullKey = prefix ? `${prefix}.${key}` : key;
    if (isObject(v)) {
      if (Object.keys(v).length === 0) pairs.push([fullKey, "{}"]);
      else collectPlainPairsRaw(v, fullKey, pairs);
    } else if (Array.isArray(v)) {
      pairs.push([fullKey, v.length === 0 ? "[]" : v.map((i) => plainScalarRaw(i)).join(",")]);
    } else if (v === null) {
      pairs.push([fullKey, ""]);
    } else {
      pairs.push([fullKey, plainScalar(v)]);
    }
  }
}

function plainScalar(value: JsonValue): string {
  if (typeof value === "string") return value;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  if (typeof value === "bigint") return value.toString();
  // Decoded number: emit the exact source literal (Plain is documented lossy
  // for arithmetic-derived formatting, but a bare pass-through scalar still
  // gets its exact digits, not a mangled float64 round-trip).
  if (isLosslessNumber(value)) return value.toString();
  return canonicalJsonString(sortJsonValue(value));
}

function plainScalarRaw(value: JsonValue): string {
  if (isObject(value) || Array.isArray(value)) {
    return canonicalJsonString(sortJsonValue(value));
  }
  return plainScalar(value);
}

function quoteLogfmtValue(value: string): string {
  if (value === "") return "";
  if (!/[\s="\\]/.test(value)) return value;
  const escaped = value
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t")
    .replace(/\f/g, "\\f")
    .replace(/\v/g, "\\v");
  return `"${escaped}"`;
}

function quoteLogfmtKey(key: string): string {
  return /^[A-Za-z0-9_.-]+$/.test(key) ? key : quoteLogfmtValue(key);
}


// ═══════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════

function sanitizeForJson(value: unknown, stack = new WeakSet<object>(), depth = 0): JsonValue {
  if (depth >= MAX_DEPTH) return MAX_DEPTH_MARKER;
  if (value === null) return null;
  const t = typeof value;
  if (t === "string") return value as string;
  if (t === "boolean") return value as boolean;
  if (t === "number") {
    const n = value as number;
    if (!Number.isFinite(n)) return "<unsupported:number>";
    if (Number.isInteger(n) && !Number.isSafeInteger(n)) return "<unsupported:unsafe-integer>";
    return n;
  }
  if (t === "bigint") return value as bigint;
  if (t === "undefined") return "<unsupported:undefined>";
  if (t === "function") return "<unsupported:function>";
  if (t === "symbol") return "<unsupported:symbol>";

  // A LosslessNumber (from decodeProtocolEvent) is typeof "object" but must
  // stay opaque: the generic object branch below would otherwise flatten it
  // into its internal {value, isLosslessNumber} fields via Object.entries.
  if (isLosslessNumber(value)) return value as unknown as JsonValue;

  if (value instanceof Error) return value.message;
  if (value instanceof Date) return value.toISOString();

  if (Array.isArray(value)) {
    if (stack.has(value)) return "<unsupported:circular>";
    stack.add(value);
    const out = value.map((item) => sanitizeForJson(item, stack, depth + 1));
    stack.delete(value);
    return out;
  }

  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    if (stack.has(obj)) return "<unsupported:circular>";
    stack.add(obj);
    const out: { [key: string]: JsonValue } = {};
    for (const [k, v] of Object.entries(obj)) {
      out[k] = sanitizeForJson(v, stack, depth + 1);
    }
    stack.delete(obj);
    return out;
  }

  return "<unsupported:unknown>";
}

function isObject(value: unknown): value is { [key: string]: JsonValue } {
  // isLosslessNumber excludes decoded numbers: a LosslessNumber is
  // typeof "object" but is an opaque scalar leaf, never a plain JSON object —
  // every object-walking function below (redaction, YAML/plain rendering,
  // sortJsonValue, validation) funnels through this check, so excluding it
  // here is the single choke point that keeps LosslessNumber from being
  // destructured into its internal {value, isLosslessNumber} fields.
  return typeof value === "object" && value !== null && !Array.isArray(value) && !isLosslessNumber(value);
}

function sortedObjectKeys(obj: { [key: string]: JsonValue }): string[] {
  return Object.keys(obj).sort(jcsCompare);
}

function sortJsonValue(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(sortJsonValue);
  if (isObject(value)) {
    const out: { [key: string]: JsonValue } = {};
    for (const key of sortedObjectKeys(value)) {
      out[key] = sortJsonValue(value[key]);
    }
    return out;
  }
  return value;
}

function jcsCompare(a: string, b: string): number {
  const ua = Array.from(a).flatMap((c) => {
    const cp = c.codePointAt(0)!;
    if (cp > 0xffff) {
      const offset = cp - 0x10000;
      return [0xd800 + (offset >> 10), 0xdc00 + (offset & 0x3ff)];
    }
    return [cp];
  });
  const ub = Array.from(b).flatMap((c) => {
    const cp = c.codePointAt(0)!;
    if (cp > 0xffff) {
      const offset = cp - 0x10000;
      return [0xd800 + (offset >> 10), 0xdc00 + (offset & 0x3ff)];
    }
    return [cp];
  });
  for (let i = 0; i < ua.length && i < ub.length; i++) {
    if (ua[i] !== ub[i]) return ua[i] - ub[i];
  }
  return ua.length - ub.length;
}
