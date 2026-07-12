/**
 * AFDATA CLI helpers — output format parsing, log filter normalization, error building.
 */

import {
  JsonValue,
  type OutputOptions,
  Event,
  jsonError,
  jsonResult,
  outputJson,
  outputYaml,
  outputPlain,
  validateProtocolEvent,
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
 * Normalized log filter result with enabled() matching and readonly filters property.
 */
export class LogFilters {
  readonly filters: readonly string[];

  constructor(entries: readonly string[]) {
    const out: string[] = [];
    for (const entry of entries) {
      const s = entry.trim().toLowerCase();
      if (s && !out.includes(s)) {
        out.push(s);
      }
    }
    this.filters = Object.freeze([...out]);
  }

  /**
   * Check if an event matches the filter set.
   * Returns false for empty filters, true if "all" or "*" is present,
   * otherwise checks if event.toLowerCase() starts with any filter.
   *
   * @example
   * const lf = new LogFilters(["query", "error"]);
   * lf.enabled("QueryStarted") // → true (starts with "query")
   * lf.enabled("debug") // → false
   *
   * const all = new LogFilters([]);
   * all.enabled("anything") // → false (empty)
   *
   * const wild = new LogFilters(["*"]);
   * wild.enabled("anything") // → true
   */
  enabled(event: string): boolean {
    if (this.filters.length === 0) return false;
    if (this.filters.includes("all") || this.filters.includes("*")) return true;
    const eventLower = event.toLowerCase();
    for (const filter of this.filters) {
      if (eventLower.startsWith(filter)) {
        return true;
      }
    }
    return false;
  }
}

/**
 * Normalize --output flag entries: trim, lowercase, deduplicate, remove empty.
 * Accepts pre-split entries (e.g. after splitting on comma).
 *
 * @example
 * const lf = cliParseLogFilters(["Query", " error ", "query"]);
 * lf.enabled("QueryStarted") // → true
 * lf.filters // → ["query", "error"]
 */
export function cliParseLogFilters(entries: string[]): LogFilters {
  return new LogFilters(entries);
}

/**
 * Dispatch output formatting by OutputFormat, with optional explicit redaction and style.
 * JSON ignores OutputStyle and preserves original keys and values after redaction.
 *
 * @example
 * cliOutput(jsonResult({ ok: true }).build(), "plain") // → "kind=result result.ok=true"
 */
export function cliOutput(value: JsonValue | Event, format: OutputFormat, options: OutputOptions = {}): string {
  const jsonValue = value instanceof Event ? (value.toJSON() as JsonValue) : value;
  if (format === "yaml") return outputYaml(jsonValue, options);
  if (format === "plain") return outputPlain(jsonValue, options);
  return outputJson(jsonValue, options);
}

export type CliEventWriter = (line: string) => void;

/** Stateful emitter for finite structured CLI executions. */
export class CliEmitter {
  private terminalEmitted = false;
  private logFieldsProvider?: () => Record<string, JsonValue>;

  constructor(
    private readonly writer: CliEventWriter,
    private readonly format: OutputFormat,
    private readonly outputOptions: OutputOptions = {},
  ) {}

  withLogFields(provider: () => Record<string, JsonValue>): this {
    this.logFieldsProvider = provider;
    return this;
  }

  emit(event: Event): void {
    const jsonValue = event.toJSON() as Record<string, JsonValue>;
    validateProtocolEvent(jsonValue);
    const kind = jsonValue.kind as string;
    if (kind === "log" || kind === "progress") {
      if (this.terminalEmitted) {
        throw new Error("cannot emit non-terminal event after terminal event");
      }
    } else if (kind === "result" || kind === "error") {
      if (this.terminalEmitted) {
        throw new Error("cannot emit duplicate terminal event");
      }
    } else {
      throw new Error(`unsupported event kind ${JSON.stringify(kind)}`);
    }
    this.writer(`${cliOutput(jsonValue, this.format, this.outputOptions)}\n`);
    if (kind === "result" || kind === "error") this.terminalEmitted = true;
  }

  emitValidatedValue(value: JsonValue): void {
    validateProtocolEvent(value);
    const kind = (value as Record<string, JsonValue>).kind;
    if (kind === "log" || kind === "progress") {
      if (this.terminalEmitted) {
        throw new Error("cannot emit non-terminal event after terminal event");
      }
    } else if (kind === "result" || kind === "error") {
      if (this.terminalEmitted) {
        throw new Error("cannot emit duplicate terminal event");
      }
    } else {
      throw new Error(`unsupported event kind ${JSON.stringify(kind)}`);
    }
    this.writer(`${cliOutput(value, this.format, this.outputOptions)}\n`);
    if (kind === "result" || kind === "error") this.terminalEmitted = true;
  }
}

/** Build a standard CLI version value. */
export function buildCliVersion(version: string): Event {
  return jsonResult({ version }).build();
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
    if (allowOutputFormat && arg === "--json") {
      if (outputFormat !== undefined && outputFormat !== "json") {
        outputError = new Error("conflicting output formats: --json conflicts with previous output format");
      } else {
        outputFormat = "json";
      }
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
          const parsedOutput = cliParseOutput(value);
          if (outputFormat !== undefined && outputFormat !== parsedOutput) {
            outputError = new Error(`conflicting output formats: ${outputFlag} ${value} conflicts with previous output format`);
          } else {
            outputFormat = parsedOutput;
          }
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
export function buildCliError(message: string, hint?: string): Event {
  return jsonError("cli_error", message).hintIfSome(hint).build();
}
