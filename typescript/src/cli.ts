/**
 * AFDATA CLI helpers — output format parsing, log filter normalization, error building.
 */

import {
  JsonValue,
  type OutputOptions,
  Event,
  jsonError,
  jsonResult,
  formatJsonValue,
  formatYamlValue,
  formatPlainValue,
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
   *
   * An empty filter set returns false (filtering is opt-in). The single
   * wildcard word "all" returns true ("*" is not special — one wildcard
   * spelling, not two). Otherwise returns true iff event.toLowerCase() starts
   * with any filter (prefix match); a mistyped filter simply matches nothing
   * and silently emits no output.
   *
   * @example
   * const lf = new LogFilters(["query", "error"]);
   * lf.enabled("QueryStarted") // → true (starts with "query")
   * lf.enabled("debug") // → false
   *
   * const none = new LogFilters([]);
   * none.enabled("anything") // → false (empty)
   *
   * const wild = new LogFilters(["all"]);
   * wild.enabled("anything") // → true
   */
  enabled(event: string): boolean {
    if (this.filters.length === 0) return false;
    if (this.filters.includes("all")) return true;
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
 * Single public render entry point: dispatch a value or Event to
 * JSON/YAML/plain text by OutputFormat, with optional explicit redaction and
 * style. `options.style` (PlainStyle) affects plain (logfmt) output only;
 * JSON and YAML are structure-preserving and ignore it.
 *
 * @example
 * render(jsonResult({ ok: true }).build(), "plain") // → "kind=result result.ok=true"
 */
export function render(value: JsonValue | Event, format: OutputFormat, options: OutputOptions = {}): string {
  const jsonValue = value instanceof Event ? (value.toJSON() as JsonValue) : value;
  if (format === "yaml") return formatYamlValue(jsonValue, options);
  if (format === "plain") return formatPlainValue(jsonValue, options);
  return formatJsonValue(jsonValue, options);
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
    this.writer(`${render(jsonValue, this.format, this.outputOptions)}\n`);
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
    this.writer(`${render(value, this.format, this.outputOptions)}\n`);
    if (kind === "result" || kind === "error") this.terminalEmitted = true;
  }
}

/** Build a standard CLI version value. */
export function buildCliVersion(version: string): Event {
  return jsonResult({ code: "version", version }).build();
}

/**
 * Render CLI version output.
 * Pass an OutputFormat for AFDATA JSON/YAML/plain. Pass null to preserve
 * conventional "<name> <version>" text.
 */
export function cliRenderVersion(name: string, version: string, format: OutputFormat | null = null): string {
  const rendered = format === null ? `${name} ${version}` : render(buildCliVersion(version), format);
  return `${rendered.replace(/\n+$/u, "")}\n`;
}

/**
 * Render version output if --version/-V is present; otherwise return undefined.
 * Throws for malformed version requests, for example `--version --output xml`.
 *
 * One blessed behavior, not configurable: a bare version request renders
 * conventional "<name> <version>" text; `--json` or `--output <fmt>` /
 * `--output=<fmt>` alongside it selects the structured AFDATA payload
 * instead.
 *
 * Only a top-level version request is recognized: scanning stops at the first
 * positional argument (the subcommand) or `--`, so `tool sub --version <value>`
 * leaves `--version` for the subcommand's parser rather than printing the
 * tool version.
 */
export function cliHandleVersionOrContinue(rawArgs: string[], name: string, version: string): string | undefined {
  let versionRequested = false;
  let outputFormat: OutputFormat | undefined;
  let outputError: Error | undefined;

  for (let i = 0; i < rawArgs.length;) {
    const arg = rawArgs[i]!;
    if (arg === "--") break;
    // The first positional argument marks the subcommand boundary. Past it,
    // --version and -V belong to the subcommand's own parser, matching
    // git/cargo/clap: this pre-parser only owns a top-level version request.
    if (!arg.startsWith("-")) break;
    if (arg === "--version" || arg === "-V") {
      versionRequested = true;
      i += 1;
      continue;
    }
    if (arg === "--json") {
      if (outputFormat !== undefined && outputFormat !== "json") {
        outputError = new Error("conflicting output formats: --json conflicts with previous output format");
      } else {
        outputFormat = "json";
      }
      i += 1;
      continue;
    }
    if (arg === "--output" || arg.startsWith("--output=")) {
      let value: string | undefined;
      let step = 1;
      if (arg.startsWith("--output=")) {
        value = arg.slice("--output=".length);
      } else if (rawArgs[i + 1] !== undefined && !rawArgs[i + 1]!.startsWith("-")) {
        value = rawArgs[i + 1];
        step = 2;
      }
      if (value === undefined) {
        outputError = new Error("missing value for --output: expected json, yaml, or plain");
      } else {
        try {
          const parsedOutput = cliParseOutput(value);
          if (outputFormat !== undefined && outputFormat !== parsedOutput) {
            outputError = new Error(`conflicting output formats: --output ${value} conflicts with previous output format`);
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
  return cliRenderVersion(name, version, outputFormat ?? null);
}

/**
 * Build a standard CLI parse error value. This function cannot fail.
 * Use when flag parsing fails or a flag value is invalid.
 * Print with render and exit with code 2.
 *
 * Always returns a strict-valid `kind:"error"` event with code `"cli_error"`.
 * An empty `message` is replaced with a generic placeholder so the returned
 * event stays strict-valid without throwing.
 *
 * @example
 * const err = buildCliError("--output: invalid value 'xml'");
 * console.log(render(err, "json"));
 * process.exit(2);
 */
export function buildCliError(message: string, hint?: string): Event {
  const msg = message && message !== "" ? message : "unspecified error";
  return jsonError("cli_error", msg).hintIfSome(hint).build();
}
