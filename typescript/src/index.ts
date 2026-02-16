/**
 * Agent-First Data (AFD) — suffix-driven output formatting and protocol templates.
 */

export { buildJsonStartup, buildJsonOk, buildJsonError, buildJson, outputJson, outputYaml, outputPlain, internalRedactSecrets, parseSize } from "./format.js";
export { log, span, initJson, initPlain, initYaml } from "./afd_logging.js";
