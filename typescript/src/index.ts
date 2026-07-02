/**
 * Agent-First Data (AFDATA) — suffix-driven output formatting and protocol templates.
 */

export {
  buildJsonOk,
  buildJsonError,
  buildJson,
  type JsonValue,
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
  redactSecretsInPlace,
  redactSecretsInPlaceWithOptions,
  redactedValue,
  redactedValueWith,
  redactedValueWithOptions,
  redactUrlSecrets,
  redactUrlSecretsWithOptions,
  parseSize,
  normalizeUtcOffset,
  isValidRfc3339Date,
  isValidRfc3339Time,
} from "./format.js";
export { log, span, initJson, initPlain, initYaml } from "./afdata_logging.js";
export {
  type OutputFormat,
  cliParseOutput,
  cliParseLogFilters,
  cliOutput,
  cliOutputWithOptions,
  buildCliError,
  buildCliVersion,
  cliRenderVersion,
  cliHandleVersionOrContinue,
} from "./cli.js";
export {
  type SkillSpec,
  type SkillAction,
  type SkillScope,
  type SkillAgentSelection,
  type SkillAgent,
  type SkillOptions,
  type SkillTargetStatus,
  type SkillUninstallStatus,
  type SkillReport,
  SkillError,
  runSkillAdmin,
} from "./skill.js";
export {
  STDOUT_FILE_ARG,
  STDERR_FILE_ARG,
  type StreamRedirectConfig,
  type InstalledStreamRedirect,
  configFromRawArgs as streamRedirectConfigFromRawArgs,
  installStreamRedirect,
  installStreamRedirectFromRawArgs,
} from "./stream_redirect.js";
