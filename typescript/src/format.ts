/**
 * AFDATA output formatting and protocol templates.
 *
 * 21 public APIs and 4 types: protocol builders, value redactors (copy and
 * in-place; cover _secret and _url fields), output formatters, URL-string
 * redactors (redactUrlSecrets / WithOptions), parseSize, normalizeUtcOffset,
 * isValidRfc3339Date, isValidRfc3339Time, RedactionPolicy, RedactionOptions,
 * OutputStyle, and OutputOptions.
 */

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

// ═══════════════════════════════════════════
// Public API: Protocol Builders
// ═══════════════════════════════════════════

/** Build {code: "ok", result, trace?}. */
export function buildJsonOk(result: JsonValue, trace?: JsonValue): JsonValue {
  const m: Record<string, JsonValue> = { code: "ok", result };
  if (trace !== undefined) m.trace = trace;
  return m;
}

/** Build {code: "error", error: message, hint?, trace?}. */
export function buildJsonError(message: string, hint?: string, trace?: JsonValue): JsonValue {
  const m: Record<string, JsonValue> = { code: "error", error: message };
  if (hint !== undefined) m.hint = hint;
  if (trace !== undefined) m.trace = trace;
  return m;
}

/** Build {code: "<custom>", ...fields, trace?}. */
export function buildJson(code: string, fields: JsonValue, trace?: JsonValue): JsonValue {
  const result: Record<string, JsonValue> = isObject(fields) ? { ...fields } : {};
  result.code = code;
  if (trace !== undefined) result.trace = trace;
  return result;
}

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

export enum RedactionPolicy {
  RedactionTraceOnly = "RedactionTraceOnly",
  RedactionNone = "RedactionNone",
}

export type RedactionOptions = {
  /** Optional scoped policy. Omitted means default full redaction. */
  policy?: RedactionPolicy;
  /**
   * Field names to treat as secrets in addition to _secret suffixes.
   * Matching is exact field-name equality at any nesting level. The same
   * list also matches URL query-parameter names inside _url fields (see
   * redactUrlSecrets).
   */
  secretNames?: readonly string[];
};

export enum OutputStyle {
  Readable = "Readable",
  Raw = "Raw",
}

export type OutputOptions = {
  /** Redaction options applied before rendering. */
  redaction?: RedactionOptions;
  /** Rendering style for YAML/plain output. Omitted means readable. */
  style?: OutputStyle;
};

/** Format as single-line JSON. Secrets redacted, original keys, raw values. */
export function outputJson(value: JsonValue): string {
  return JSON.stringify(redactedValue(value));
}

/** Format as single-line JSON with explicit redaction policy. */
export function outputJsonWith(value: JsonValue, redactionPolicy: RedactionPolicy): string {
  return JSON.stringify(redactedValueWith(value, redactionPolicy));
}

/** Format as single-line JSON with explicit output options. */
export function outputJsonWithOptions(value: JsonValue, outputOptions: OutputOptions): string {
  return JSON.stringify(redactedValueWithOptions(value, outputOptions.redaction ?? {}));
}

/** Format as multi-line YAML. Keys stripped, values formatted, secrets redacted. */
export function outputYaml(value: JsonValue): string {
  return outputYamlWithOptions(value, {});
}

/** Format as multi-line YAML with explicit output options. */
export function outputYamlWithOptions(value: JsonValue, outputOptions: OutputOptions): string {
  value = redactedValueWithOptions(value, outputOptions.redaction ?? {});
  const lines = ["---"];
  if (outputOptions.style === OutputStyle.Raw) {
    renderYamlRaw(value, 0, lines);
  } else {
    renderYamlProcessed(value, 0, lines);
  }
  return lines.join("\n");
}

/** Format as single-line logfmt. Keys stripped, values formatted, secrets redacted. */
export function outputPlain(value: JsonValue): string {
  return outputPlainWithOptions(value, {});
}

/** Format as single-line logfmt with explicit output options. */
export function outputPlainWithOptions(value: JsonValue, outputOptions: OutputOptions): string {
  value = redactedValueWithOptions(value, outputOptions.redaction ?? {});
  const pairs: [string, string][] = [];
  if (outputOptions.style === OutputStyle.Raw) {
    collectPlainPairsRaw(value, "", pairs);
  } else {
    collectPlainPairs(value, "", pairs);
  }
  pairs.sort(([a], [b]) => jcsCompare(a, b));
  return pairs
    .map(([k, v]) => `${quoteLogfmtKey(k)}=${quoteLogfmtValue(v)}`)
    .join(" ");
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/** Redact _secret fields in-place. */
export function redactSecretsInPlace(value: JsonValue): void {
  redactSecrets(value);
}

/** Redact secret fields in-place using explicit redaction options. */
export function redactSecretsInPlaceWithOptions(value: JsonValue, redactionOptions: RedactionOptions): void {
  applyRedactionOptions(value, redactionOptions);
}

/** Return a JSON-safe copy with default _secret redaction applied. */
export function redactedValue(value: unknown): JsonValue {
  const v = sanitizeForJson(value);
  redactSecrets(v);
  return v;
}

/** Return a JSON-safe copy with an explicit redaction policy applied. */
export function redactedValueWith(value: unknown, redactionPolicy: RedactionPolicy): JsonValue {
  const v = sanitizeForJson(value);
  applyRedactionPolicy(v, redactionPolicy);
  return v;
}

/** Return a JSON-safe copy with explicit redaction options applied. */
export function redactedValueWithOptions(value: unknown, redactionOptions: RedactionOptions): JsonValue {
  const v = sanitizeForJson(value);
  applyRedactionOptions(v, redactionOptions);
  return v;
}

/**
 * Redact secret components of a single URL string, using default options.
 *
 * Returns url with its userinfo password and any _secret-suffixed query
 * parameter values replaced by "***". See redactUrlSecretsWithOptions.
 */
export function redactUrlSecrets(url: string): string {
  return redactUrlSecretsWithOptions(url, {});
}

/**
 * Redact secret components of a single URL string.
 *
 * A query parameter is redacted iff its (form-decoded) name ends in
 * _secret/_SECRET or matches an exact entry in secretNames. The userinfo
 * password (scheme://user:pass@host) is always redacted as a structural rule.
 * Only the secret spans are replaced with "***"; every other byte is preserved.
 * A string that is not a single, whitespace-free, scheme-prefixed URL
 * (including a URL embedded in surrounding prose) is returned unchanged.
 */
export function redactUrlSecretsWithOptions(url: string, redactionOptions: RedactionOptions): string {
  const redacted = redactUrlInStr(url, secretNameSet(redactionOptions));
  return redacted ?? url;
}

/**
 * Parse a human-readable size string into bytes.
 * Accepts bare numbers or numbers followed by a unit letter (B/K/M/G/T).
 * Case-insensitive. Trims whitespace. Returns null for invalid input.
 */
export function parseSize(s: string): number | null {
  s = s.trim();
  if (!s) return null;
  const multipliers: Record<string, number> = {
    b: 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3, t: 1024 ** 4,
  };
  const last = s[s.length - 1].toLowerCase();
  let numStr: string;
  let mult: number;
  if (last in multipliers) {
    numStr = s.slice(0, -1);
    mult = multipliers[last];
  } else if ((last >= "0" && last <= "9") || last === ".") {
    numStr = s;
    mult = 1;
  } else {
    return null;
  }
  if (!numStr || !/^(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(numStr)) return null;
  const n = Number(numStr);
  if (isNaN(n) || n < 0 || !isFinite(n)) return null;
  const result = Math.trunc(n * mult);
  if (!Number.isSafeInteger(result)) return null;
  return result;
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

type RedactionContext = {
  secretNames: ReadonlySet<string>;
};

const DEFAULT_CONTEXT: RedactionContext = {
  secretNames: new Set<string>(),
};

function secretNameSet(redactionOptions: RedactionOptions): ReadonlySet<string> {
  return new Set(redactionOptions.secretNames ?? []);
}

function contextFromOptions(redactionOptions: RedactionOptions): RedactionContext {
  return { secretNames: secretNameSet(redactionOptions) };
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

const MAX_DEPTH = 256;

function redactSecrets(value: JsonValue, context: RedactionContext = DEFAULT_CONTEXT, depth = 0): void {
  if (depth >= MAX_DEPTH) return;
  if (isObject(value)) {
    for (const k of Object.keys(value)) {
      const v = value[k];
      if (isSecretKey(k, context.secretNames)) {
        value[k] = "***";
      } else if (keyHasUrlSuffix(k)) {
        if (typeof v === "string") {
          value[k] = redactUrlFieldValue(v, context.secretNames);
        } else {
          value[k] = depth + 1 >= MAX_DEPTH ? "***" : (redactSecrets(v, context, depth + 1), v);
        }
      } else {
        value[k] = depth + 1 >= MAX_DEPTH ? "***" : (redactSecrets(v, context, depth + 1), v);
      }
    }
  } else if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      if (depth + 1 >= MAX_DEPTH) value[i] = "***";
      else redactSecrets(value[i], context, depth + 1);
    }
  }
}

function applyRedactionPolicy(value: JsonValue, redactionPolicy: RedactionPolicy): void {
  applyRedactionPolicyWithContext(value, redactionPolicy, DEFAULT_CONTEXT);
}

function applyRedactionOptions(value: JsonValue, redactionOptions: RedactionOptions): void {
  applyRedactionPolicyWithContext(value, redactionOptions.policy, contextFromOptions(redactionOptions));
}

function applyRedactionPolicyWithContext(
  value: JsonValue,
  redactionPolicy: RedactionPolicy | undefined,
  context: RedactionContext,
): void {
  switch (redactionPolicy) {
    case RedactionPolicy.RedactionTraceOnly:
      if (isObject(value) && value.trace !== undefined) {
        redactSecrets(value.trace, context);
      }
      break;
    case RedactionPolicy.RedactionNone:
      break;
    default:
      // Empty/unknown policy falls back to default full redaction.
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

  // Query runs from the first '?' to the first '#' (or end).
  let newRemainder: string;
  const q = remainder.indexOf("?");
  if (q >= 0) {
    const path = remainder.slice(0, q);
    const qOnwards = remainder.slice(q + 1);
    const hash = qOnwards.indexOf("#");
    let query: string;
    let fragment: string;
    if (hash >= 0) {
      query = qOnwards.slice(0, hash);
      fragment = qOnwards.slice(hash);
    } else {
      query = qOnwards;
      fragment = "";
    }
    newRemainder = `${path}?${redactQuery(query, secretNames)}${fragment}`;
  } else {
    newRemainder = remainder;
  }

  return `${scheme}://${newAuthority}${newRemainder}`;
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
  if (/\s/.test(s) || s.includes("@")) return "***";
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
  return `${authority.slice(0, colon)}:***${authority.slice(at)}`;
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
        return `${rawKey}=***`;
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

function isInt(value: JsonValue): value is number {
  return typeof value === "number" && Number.isInteger(value);
}

function isNum(value: JsonValue): value is number {
  return typeof value === "number";
}

function tryProcessField(key: string, value: JsonValue): [string, string] | null {
  let stripped: string | null;

  // Group 1: compound timestamp suffixes
  stripped = stripSuffixCI(key, "_epoch_ms");
  if (stripped !== null) {
    if (isInt(value)) { const formatted = formatRfc3339Ms(value); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_s");
  if (stripped !== null) {
    if (isInt(value)) { const formatted = formatRfc3339Ms(value * 1000); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_ns");
  if (stripped !== null) {
    if (isInt(value)) { const formatted = formatRfc3339Ms(Math.floor(value / 1_000_000)); if (formatted !== null) return [stripped, formatted]; }
    return null;
  }

  // Group 2: compound currency suffixes
  stripped = stripSuffixCI(key, "_usd_cents");
  if (stripped !== null) {
    if (isInt(value) && value >= 0) return [stripped, `$${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")}`];
    return null;
  }
  stripped = stripSuffixCI(key, "_eur_cents");
  if (stripped !== null) {
    if (isInt(value) && value >= 0) return [stripped, `\u20ac${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")}`];
    return null;
  }
  const gc = tryStripGenericCents(key);
  if (gc !== null) {
    const [gcStripped, code] = gc;
    if (isInt(value) && value >= 0) return [gcStripped, `${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")} ${code.toUpperCase()}`];
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
    if (isNum(value)) return [stripped, `${plainScalar(value)} minutes`];
    return null;
  }
  stripped = stripSuffixCI(key, "_hours");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)} hours`];
    return null;
  }
  stripped = stripSuffixCI(key, "_days");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)} days`];
    return null;
  }

  // Group 4: single-unit suffixes
  stripped = stripSuffixCI(key, "_msats");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}msats`];
    return null;
  }
  stripped = stripSuffixCI(key, "_sats");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}sats`];
    return null;
  }
  stripped = stripSuffixCI(key, "_bytes");
  if (stripped !== null) {
    if (isInt(value)) return [stripped, formatBytesHuman(value)];
    return null;
  }
  stripped = stripSuffixCI(key, "_percent");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}%`];
    return null;
  }
  // Group 5: short suffixes (last to avoid false positives)
  stripped = stripSuffixCI(key, "_btc");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)} BTC`];
    return null;
  }
  stripped = stripSuffixCI(key, "_jpy");
  if (stripped !== null) {
    if (isInt(value) && value >= 0) return [stripped, `\u00a5${formatWithCommas(value)}`];
    return null;
  }
  stripped = stripSuffixCI(key, "_ns");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}ns`];
    return null;
  }
  stripped = stripSuffixCI(key, "_us");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}\u03bcs`];
    return null;
  }
  stripped = stripSuffixCI(key, "_ms");
  if (stripped !== null) {
    const fv = formatMsValue(value);
    if (fv !== null) return [stripped, fv];
    return null;
  }
  stripped = stripSuffixCI(key, "_s");
  if (stripped !== null) {
    if (isNum(value)) return [stripped, `${plainScalar(value)}s`];
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
      entries.push({ stripped: secretStripped, original: k, value: v, formatted: null });
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
 * ties (e.g. 1280 bytes = 1.25 KB → "1.2KB", not "1.3KB").
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
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  const TB = GB * 1024;
  const sign = bytes < 0 ? "-" : "";
  const b = Math.abs(bytes);
  if (b >= TB) return `${sign}${toFixedHalfEven(b / TB, 1)}TB`;
  if (b >= GB) return `${sign}${toFixedHalfEven(b / GB, 1)}GB`;
  if (b >= MB) return `${sign}${toFixedHalfEven(b / MB, 1)}MB`;
  if (b >= KB) return `${sign}${toFixedHalfEven(b / KB, 1)}KB`;
  return `${bytes}B`;
}

function formatWithCommas(n: number): string {
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
  const idx = withoutCents.lastIndexOf("_");
  if (idx < 0) return null;
  const code = withoutCents.slice(idx + 1);
  if (!/^[A-Za-z]{3,4}$/.test(code)) return null;
  return code;
}

// ═══════════════════════════════════════════
// YAML Rendering
// ═══════════════════════════════════════════

function renderYamlProcessed(value: JsonValue, indent: number, lines: string[]): void {
  const prefix = "  ".repeat(indent);
  if (!isObject(value)) {
    lines.push(`${prefix}${yamlScalar(value)}`);
    return;
  }

  for (const pf of processObjectFields(value)) {
    if (pf.formatted !== null) {
      lines.push(`${prefix}${yamlKey(pf.key)}: "${escapeYamlStr(pf.formatted)}"`);
    } else if (isObject(pf.value)) {
      if (Object.keys(pf.value).length > 0) {
        lines.push(`${prefix}${yamlKey(pf.key)}:`);
        renderYamlProcessed(pf.value, indent + 1, lines);
      } else {
        lines.push(`${prefix}${yamlKey(pf.key)}: {}`);
      }
    } else if (Array.isArray(pf.value)) {
      if (pf.value.length === 0) {
        lines.push(`${prefix}${yamlKey(pf.key)}: []`);
      } else {
        lines.push(`${prefix}${yamlKey(pf.key)}:`);
        for (const item of pf.value) {
          if (isObject(item)) {
            lines.push(`${prefix}  -`);
            renderYamlProcessed(item, indent + 2, lines);
          } else {
            lines.push(`${prefix}  - ${yamlScalar(item)}`);
          }
        }
      }
    } else {
      lines.push(`${prefix}${yamlKey(pf.key)}: ${yamlScalar(pf.value)}`);
    }
  }
}

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
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t")
    .replace(/\f/g, "\\f")
    .replace(/\v/g, "\\v");
}

function yamlKey(key: string): string {
  return /^[A-Za-z0-9_.-]+$/.test(key) ? key : `"${escapeYamlStr(key)}"`;
}

function yamlScalar(value: JsonValue): string {
  if (typeof value === "string") return `"${escapeYamlStr(value)}"`;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  return `"${escapeYamlStr(JSON.stringify(sortJsonValue(value)))}"`;
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
      collectPlainPairs(pf.value, fullKey, pairs);
    } else if (Array.isArray(pf.value)) {
      pairs.push([fullKey, pf.value.map((i) => plainScalar(i)).join(",")]);
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
      collectPlainPairsRaw(v, fullKey, pairs);
    } else if (Array.isArray(v)) {
      pairs.push([fullKey, v.map((i) => plainScalarRaw(i)).join(",")]);
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
  return JSON.stringify(sortJsonValue(value));
}

function plainScalarRaw(value: JsonValue): string {
  if (isObject(value) || Array.isArray(value)) {
    return JSON.stringify(sortJsonValue(value));
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
  if (depth >= MAX_DEPTH) return "***";
  if (value === null) return null;
  const t = typeof value;
  if (t === "string") return value as string;
  if (t === "boolean") return value as boolean;
  if (t === "number") {
    const n = value as number;
    return Number.isFinite(n) ? n : "<unsupported:number>";
  }
  if (t === "bigint") return "<unsupported:bigint>";
  if (t === "undefined") return "<unsupported:undefined>";
  if (t === "function") return "<unsupported:function>";
  if (t === "symbol") return "<unsupported:symbol>";

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
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
