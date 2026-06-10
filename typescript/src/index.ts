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
