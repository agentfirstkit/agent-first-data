/**
 * Agent-First Data (AFDATA) — suffix-driven output formatting and protocol templates.
 */

export {
  buildJsonOk,
  buildJsonError,
  buildJson,
  RedactionPolicy,
  type RedactionOptions,
  OutputStyle,
  type OutputOptions,
  outputJson,
  outputJsonWith,
  outputJsonWithOptions,
  outputYaml,
  outputYamlWithOptions,
  outputPlain,
  outputPlainWithOptions,
  internalRedactSecrets,
  internalRedactSecretsWithOptions,
  redactedValue,
  redactedValueWith,
  redactedValueWithOptions,
  redactUrlSecrets,
  redactUrlSecretsWithOptions,
  parseSize,
} from "./format.js";
export { log, span, initJson, initPlain, initYaml } from "./afdata_logging.js";
export {
  type OutputFormat,
  cliParseOutput,
  cliParseLogFilters,
  cliOutput,
  cliOutputWithOptions,
  buildCliError,
} from "./cli.js";
