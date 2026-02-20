/**
 * AFDATA CLI helpers — output format parsing, log filter normalization, error building.
 */

import { JsonValue, outputJson, outputYaml, outputPlain } from "./format.js";

/** Output format for CLI and pipe/MCP modes. */
export type OutputFormat = "json" | "yaml" | "plain";

/**
 * Parse the --output flag value into an OutputFormat.
 * Throws on unknown values; catch and pass message to buildCliError.
 *
 * @example
 * cliParseOutput("json") // → "json"
 * cliParseOutput("xml")  // throws Error
 */
export function cliParseOutput(s: string): OutputFormat {
  if (s === "json" || s === "yaml" || s === "plain") {
    return s;
  }
  throw new Error(`invalid --output format '${s}': expected json, yaml, or plain`);
}

/**
 * Normalize --output flag entries: trim, lowercase, deduplicate, remove empty.
 * Accepts pre-split entries (e.g. after splitting on comma).
 *
 * @example
 * cliParseLogFilters(["Query", " error ", "query"]) // → ["query", "error"]
 */
export function cliParseLogFilters(entries: string[]): string[] {
  const out: string[] = [];
  for (const entry of entries) {
    const s = entry.trim().toLowerCase();
    if (s && !out.includes(s)) {
      out.push(s);
    }
  }
  return out;
}

/**
 * Dispatch output formatting by OutputFormat.
 * Equivalent to calling outputJson, outputYaml, or outputPlain directly.
 *
 * @example
 * cliOutput({ code: "ok" }, "plain") // → "code=ok"
 */
export function cliOutput(value: JsonValue, format: OutputFormat): string {
  if (format === "yaml") return outputYaml(value);
  if (format === "plain") return outputPlain(value);
  return outputJson(value);
}

/**
 * Build a standard CLI parse error value.
 * Use when flag parsing fails or a flag value is invalid.
 * Print with outputJson and exit with code 2.
 *
 * @example
 * const err = buildCliError("--output: invalid value 'xml'");
 * console.log(outputJson(err));
 * process.exit(2);
 */
export function buildCliError(message: string): JsonValue {
  return {
    code: "error",
    error_code: "invalid_request",
    error: message,
    retryable: false,
    trace: { duration_ms: 0 },
  };
}
