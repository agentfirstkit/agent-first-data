/**
 * AFDATA CLI helpers — output format parsing, log filter normalization, error building.
 */

import {
  JsonValue,
  type OutputOptions,
  buildJson,
  outputJson,
  outputJsonWithOptions,
  outputYaml,
  outputYamlWithOptions,
  outputPlain,
  outputPlainWithOptions,
} from "./format.js";

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
 * Dispatch output formatting with explicit redaction and style.
 * JSON ignores OutputStyle and preserves original keys and values after redaction.
 */
export function cliOutputWithOptions(value: JsonValue, format: OutputFormat, outputOptions: OutputOptions): string {
  if (format === "yaml") return outputYamlWithOptions(value, outputOptions);
  if (format === "plain") return outputPlainWithOptions(value, outputOptions);
  return outputJsonWithOptions(value, outputOptions);
}

/** Build a standard CLI version value. */
export function buildCliVersion(version: string): JsonValue {
  return buildJson("version", { version });
}

/**
 * Render CLI version output.
 * Pass an OutputFormat for AFDATA JSON/YAML/plain. Pass null to preserve
 * conventional "<name> <version>" text.
 */
export function cliRenderVersion(name: string, version: string, format: OutputFormat | null = null): string {
  const rendered = format === null ? `${name} ${version}` : cliOutput(buildCliVersion(version), format);
  return `${rendered.replace(/\n+$/u, "")}\n`;
}

/**
 * Render version output if --version/-V is present; otherwise return undefined.
 * Throws for malformed version requests, for example `--version --output xml`.
 */
export function cliHandleVersionOrContinue(
  rawArgs: string[],
  name: string,
  version: string,
  defaultOutput: OutputFormat | null = null,
  outputFlag = "--output",
  allowOutputFormat = true,
): string | undefined {
  let versionRequested = false;
  let outputFormat: OutputFormat | undefined;
  let outputError: Error | undefined;

  for (let i = 0; i < rawArgs.length;) {
    const arg = rawArgs[i]!;
    if (arg === "--") break;
    if (arg === "--version" || arg === "-V") {
      versionRequested = true;
      i += 1;
      continue;
    }
    if (allowOutputFormat && (arg === outputFlag || arg.startsWith(`${outputFlag}=`))) {
      let value: string | undefined;
      let step = 1;
      if (arg.startsWith(`${outputFlag}=`)) {
        value = arg.slice(outputFlag.length + 1);
      } else if (rawArgs[i + 1] !== undefined && !rawArgs[i + 1]!.startsWith("-")) {
        value = rawArgs[i + 1];
        step = 2;
      }
      if (value === undefined) {
        outputError = new Error(`missing value for ${outputFlag}: expected json, yaml, or plain`);
      } else {
        try {
          outputFormat = cliParseOutput(value);
        } catch (e) {
          outputError = e as Error;
        }
      }
      i += step;
      continue;
    }
    i += 1;
  }

  if (!versionRequested) return undefined;
  if (outputError !== undefined) throw outputError;
  return cliRenderVersion(name, version, allowOutputFormat && outputFormat !== undefined ? outputFormat : defaultOutput);
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
export function buildCliError(message: string, hint?: string): JsonValue {
  const m: Record<string, JsonValue> = {
    code: "error",
    error: message,
  };
  if (hint !== undefined) m.hint = hint;
  return m;
}
