/**
 * Agent-First Data (AFD) — suffix-driven output formatting and protocol templates.
 */

export { OutputFormat, formatValue, formatPretty, toYaml, toPlain, redactSecrets, parseSize } from "./format.js";
export { ok, okTrace, error, errorTrace, startup, status } from "./format.js";
