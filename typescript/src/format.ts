/**
 * AFD output formatting: JSON, YAML, plain text with suffix-driven transforms.
 */

type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

// ═══════════════════════════════════════════
// OutputFormat
// ═══════════════════════════════════════════

export enum OutputFormat {
  Json = "json",
  Yaml = "yaml",
  Plain = "plain",
}

export function formatValue(fmt: OutputFormat, value: JsonValue): string {
  switch (fmt) {
    case OutputFormat.Json:
      return JSON.stringify(value);
    case OutputFormat.Yaml:
      return toYaml(value);
    case OutputFormat.Plain:
      return toPlain(value);
  }
}

export function formatPretty(fmt: OutputFormat, value: JsonValue): string {
  switch (fmt) {
    case OutputFormat.Json:
      return JSON.stringify(value, null, 2);
    case OutputFormat.Yaml:
      return toYaml(value);
    case OutputFormat.Plain:
      return toPlain(value);
  }
}

// ═══════════════════════════════════════════
// YAML
// ═══════════════════════════════════════════

export function toYaml(value: JsonValue): string {
  const lines = ["---"];
  renderYaml(value, 0, lines);
  return lines.join("\n");
}

function renderYaml(
  value: JsonValue,
  indent: number,
  lines: string[],
): void {
  const prefix = "  ".repeat(indent);
  if (isObject(value)) {
    for (const [k, v] of Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) {
      if (isObject(v)) {
        if (Object.keys(v).length > 0) {
          lines.push(`${prefix}${k}:`);
          renderYaml(v, indent + 1, lines);
        } else {
          lines.push(`${prefix}${k}: {}`);
        }
      } else if (Array.isArray(v)) {
        if (v.length === 0) {
          lines.push(`${prefix}${k}: []`);
        } else {
          lines.push(`${prefix}${k}:`);
          for (const item of v) {
            if (isObject(item)) {
              lines.push(`${prefix}  -`);
              renderYaml(item, indent + 2, lines);
            } else {
              lines.push(`${prefix}  - ${yamlScalar(item)}`);
            }
          }
        }
      } else {
        lines.push(`${prefix}${k}: ${yamlScalar(v)}`);
      }
    }
  } else {
    lines.push(`${prefix}${yamlScalar(value)}`);
  }
}

function yamlScalar(value: JsonValue): string {
  if (typeof value === "string") {
    return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t")}"`;
  }
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  return `"${String(value).replace(/"/g, '\\"')}"`;
}

// ═══════════════════════════════════════════
// Plain
// ═══════════════════════════════════════════

export function toPlain(value: JsonValue): string {
  const lines: string[] = [];
  renderPlain(value, 0, lines);
  return lines.join("\n");
}

function renderPlain(
  value: JsonValue,
  indent: number,
  lines: string[],
): void {
  const prefix = "  ".repeat(indent);
  if (isObject(value)) {
    for (const [k, v] of Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) {
      if (isObject(v)) {
        lines.push(`${prefix}${k}:`);
        renderPlain(v, indent + 1, lines);
      } else if (Array.isArray(v)) {
        if (v.length === 0) {
          lines.push(`${prefix}${k}: []`);
        } else if (v.every((i) => !isObject(i) && !Array.isArray(i))) {
          lines.push(`${prefix}${k}:`);
          for (const item of v) {
            lines.push(`${prefix}  - ${plainScalar(item)}`);
          }
        } else {
          lines.push(`${prefix}${k}:`);
          for (const item of v) {
            if (isObject(item)) {
              lines.push(`${prefix}  -`);
              renderPlain(item, indent + 2, lines);
            } else {
              lines.push(`${prefix}  - ${plainScalar(item)}`);
            }
          }
        }
      } else {
        lines.push(`${prefix}${k}: ${formatPlainField(k, v)}`);
      }
    }
  } else {
    lines.push(`${prefix}${plainScalar(value)}`);
  }
}

function formatPlainField(
  key: string,
  value: JsonValue,
): string {
  const lower = key.toLowerCase();

  // Secret
  if (lower.endsWith("_secret")) return "***";

  // Timestamps
  if (lower.endsWith("_epoch_ms")) {
    if (typeof value === "number" && Number.isInteger(value))
      return formatRfc3339Ms(value);
  }
  if (lower.endsWith("_epoch_s")) {
    if (typeof value === "number" && Number.isInteger(value))
      return formatRfc3339Ms(value * 1000);
  }
  if (lower.endsWith("_epoch_ns")) {
    if (typeof value === "number" && Number.isInteger(value))
      return formatRfc3339Ms(Math.floor(value / 1_000_000));
  }
  if (lower.endsWith("_rfc3339")) return plainScalar(value);

  // Size
  if (lower.endsWith("_bytes")) {
    if (typeof value === "number" && Number.isInteger(value))
      return formatBytesHuman(value);
  }

  // Percentage
  if (lower.endsWith("_percent") && typeof value === "number")
    return `${plainScalar(value)}%`;

  // Currency — Bitcoin
  if (lower.endsWith("_msats") && typeof value === "number")
    return `${plainScalar(value)}msats`;
  if (lower.endsWith("_sats") && typeof value === "number")
    return `${plainScalar(value)}sats`;
  if (lower.endsWith("_btc") && typeof value === "number")
    return `${plainScalar(value)} BTC`;

  // Currency — Fiat
  if (lower.endsWith("_usd_cents") && typeof value === "number" && value >= 0 && Number.isInteger(value)) {
    return `$${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")}`;
  }
  if (lower.endsWith("_eur_cents") && typeof value === "number" && value >= 0 && Number.isInteger(value)) {
    return `\u20ac${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")}`;
  }
  if (lower.endsWith("_jpy") && typeof value === "number" && value >= 0 && Number.isInteger(value)) {
    return `\u00a5${formatWithCommas(value)}`;
  }
  if (lower.endsWith("_cents")) {
    const code = extractCurrencyCode(lower);
    if (code && typeof value === "number" && value >= 0 && Number.isInteger(value)) {
      return `${Math.floor(value / 100)}.${String(value % 100).padStart(2, "0")} ${code.toUpperCase()}`;
    }
  }

  // Duration — long
  if (lower.endsWith("_minutes") && typeof value === "number")
    return `${plainScalar(value)} minutes`;
  if (lower.endsWith("_hours") && typeof value === "number")
    return `${plainScalar(value)} hours`;
  if (lower.endsWith("_days") && typeof value === "number")
    return `${plainScalar(value)} days`;

  // Duration — ms
  if (lower.endsWith("_ms") && !lower.endsWith("_epoch_ms") && typeof value === "number") {
    if (value >= 1000) return `${(value / 1000).toFixed(2)}s`;
    return `${plainScalar(value)}ms`;
  }

  // Duration — ns, us, s
  if (lower.endsWith("_ns") && !lower.endsWith("_epoch_ns") && typeof value === "number")
    return `${plainScalar(value)}ns`;
  if (lower.endsWith("_us") && typeof value === "number")
    return `${plainScalar(value)}\u03bcs`;
  if (lower.endsWith("_s") && !lower.endsWith("_epoch_s") && typeof value === "number")
    return `${plainScalar(value)}s`;

  return plainScalar(value);
}

function plainScalar(value: JsonValue): string {
  if (typeof value === "string") return value;
  if (value === null) return "null";
  if (typeof value === "boolean") return value.toString();
  if (typeof value === "number") return value.toString();
  return String(value);
}

// ═══════════════════════════════════════════
// Secret redaction
// ═══════════════════════════════════════════

export function redactSecrets(value: JsonValue): JsonValue {
  if (isObject(value)) {
    for (const k of Object.keys(value)) {
      if (k.toLowerCase().endsWith("_secret") && typeof value[k] === "string") {
        value[k] = "***";
      }
      redactSecrets(value[k]);
    }
  } else if (Array.isArray(value)) {
    for (const item of value) {
      redactSecrets(item);
    }
  }
  return value;
}

// ═══════════════════════════════════════════
// AFD Protocol templates
// ═══════════════════════════════════════════

export function ok(result: JsonValue): JsonValue {
  return { code: "ok", result };
}

export function okTrace(result: JsonValue, trace: JsonValue): JsonValue {
  return { code: "ok", result, trace };
}

export function error(message: string): JsonValue {
  return { code: "error", error: message };
}

export function errorTrace(message: string, trace: JsonValue): JsonValue {
  return { code: "error", error: message, trace };
}

export function startup(
  config: JsonValue,
  args: JsonValue,
  env: JsonValue,
): JsonValue {
  return { code: "startup", config, args, env };
}

export function status(
  code: string,
  fields?: { [key: string]: JsonValue },
): JsonValue {
  return { ...fields, code };
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

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
  return n.toLocaleString("en-US");
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

function extractCurrencyCode(key: string): string | null {
  if (!key.endsWith("_cents")) return null;
  const withoutCents = key.slice(0, -6);
  const idx = withoutCents.lastIndexOf("_");
  if (idx < 0) return null;
  return withoutCents.slice(idx + 1);
}

function isObject(
  value: JsonValue,
): value is { [key: string]: JsonValue } {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
