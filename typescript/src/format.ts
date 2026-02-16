/**
 * AFD output formatting and protocol templates.
 *
 * 9 public APIs: 4 protocol builders + 3 output formatters + 1 redaction + 1 utility.
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

/** Build {code: "startup", config, args, env}. */
export function buildJsonStartup(config: JsonValue, args: JsonValue, env: JsonValue): JsonValue {
  return { code: "startup", config, args, env };
}

/** Build {code: "ok", result, trace?}. */
export function buildJsonOk(result: JsonValue, trace?: JsonValue): JsonValue {
  const m: Record<string, JsonValue> = { code: "ok", result };
  if (trace !== undefined) m.trace = trace;
  return m;
}

/** Build {code: "error", error: message, trace?}. */
export function buildJsonError(message: string, trace?: JsonValue): JsonValue {
  const m: Record<string, JsonValue> = { code: "error", error: message };
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

/** Format as single-line JSON. Secrets redacted, original keys, raw values. */
export function outputJson(value: JsonValue): string {
  const v = JSON.parse(JSON.stringify(value));
  redactSecrets(v);
  return JSON.stringify(v);
}

/** Format as multi-line YAML. Keys stripped, values formatted, secrets redacted. */
export function outputYaml(value: JsonValue): string {
  const lines = ["---"];
  renderYamlProcessed(value, 0, lines);
  return lines.join("\n");
}

/** Format as single-line logfmt. Keys stripped, values formatted, secrets redacted. */
export function outputPlain(value: JsonValue): string {
  const pairs: [string, string][] = [];
  collectPlainPairs(value, "", pairs);
  pairs.sort(([a], [b]) => jcsCompare(a, b));
  return pairs
    .map(([k, v]) => (v.includes(" ") ? `${k}="${v}"` : `${k}=${v}`))
    .join(" ");
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/** Redact _secret fields in-place. */
export function internalRedactSecrets(value: JsonValue): void {
  redactSecrets(value);
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
  if (!numStr) return null;
  const n = Number(numStr);
  if (isNaN(n) || n < 0 || !isFinite(n)) return null;
  return Math.trunc(n * mult);
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

function redactSecrets(value: JsonValue): void {
  if (isObject(value)) {
    for (const k of Object.keys(value)) {
      if (k.endsWith("_secret") || k.endsWith("_SECRET")) {
        const v = value[k];
        if (isObject(v) || Array.isArray(v)) {
          redactSecrets(v);
        } else {
          value[k] = "***";
        }
      } else {
        redactSecrets(value[k]);
      }
    }
  } else if (Array.isArray(value)) {
    for (const item of value) {
      redactSecrets(item);
    }
  }
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
    if (isInt(value)) return [stripped, formatRfc3339Ms(value)];
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_s");
  if (stripped !== null) {
    if (isInt(value)) return [stripped, formatRfc3339Ms(value * 1000)];
    return null;
  }
  stripped = stripSuffixCI(key, "_epoch_ns");
  if (stripped !== null) {
    if (isInt(value)) return [stripped, formatRfc3339Ms(Math.floor(value / 1_000_000))];
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
  stripped = stripSuffixCI(key, "_secret");
  if (stripped !== null) return [stripped, "***"];

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

function formatRfc3339Ms(ms: number): string {
  try {
    const d = new Date(ms);
    return d.toISOString().replace(/(\.\d{3})\d*Z$/, "$1Z");
  } catch {
    return String(ms);
  }
}

function formatBytesHuman(bytes: number): string {
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  const TB = GB * 1024;
  const sign = bytes < 0 ? "-" : "";
  const b = Math.abs(bytes);
  if (b >= TB) return `${sign}${(b / TB).toFixed(1)}TB`;
  if (b >= GB) return `${sign}${(b / GB).toFixed(1)}GB`;
  if (b >= MB) return `${sign}${(b / MB).toFixed(1)}MB`;
  if (b >= KB) return `${sign}${(b / KB).toFixed(1)}KB`;
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
  if (!code) return null;
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
      lines.push(`${prefix}${pf.key}: "${escapeYamlStr(pf.formatted)}"`);
    } else if (isObject(pf.value)) {
      if (Object.keys(pf.value).length > 0) {
        lines.push(`${prefix}${pf.key}:`);
        renderYamlProcessed(pf.value, indent + 1, lines);
      } else {
        lines.push(`${prefix}${pf.key}: {}`);
      }
    } else if (Array.isArray(pf.value)) {
      if (pf.value.length === 0) {
        lines.push(`${prefix}${pf.key}: []`);
      } else {
        lines.push(`${prefix}${pf.key}:`);
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
      lines.push(`${prefix}${pf.key}: ${yamlScalar(pf.value)}`);
    }
  }
}

function escapeYamlStr(s: string): string {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t");
}

function yamlScalar(value: JsonValue): string {
  if (typeof value === "string") return `"${escapeYamlStr(value)}"`;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  return `"${String(value).replace(/"/g, '\\"')}"`;
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

function plainScalar(value: JsonValue): string {
  if (typeof value === "string") return value;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  return String(value);
}

// ═══════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════

function isObject(value: JsonValue): value is { [key: string]: JsonValue } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
