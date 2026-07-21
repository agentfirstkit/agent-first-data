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

/**
 * Where a CliEmitter sends its events, selected by `--output-to`.
 *
 * The stream an event lands on follows the program's consumption mode, not the
 * event's shape (see the spec's CLI Event Framing):
 *
 * - "split" (the default) is finite one-shot mode: `result` → stdout, while
 *   `error`/`progress`/`log` → stderr. stdout therefore carries only successful
 *   payloads, so a shell capture or pipe never mistakes a failure for data.
 * - "stdout" / "stderr" are event-stream mode: every event, including `error`,
 *   is collapsed onto that one stream so a consumer reading it in order
 *   (branching on `kind`) sees preserved ordering.
 */
export type OutputTo = "split" | "stdout" | "stderr";

/**
 * Parse an `--output-to` value into an OutputTo: "split" (default), "stdout", or
 * "stderr". Throws on unknown values; catch and pass the message to
 * buildCliError.
 *
 * @example
 * parseOutputTo("split") // → "split"
 * parseOutputTo("both")  // throws Error
 */
export function parseOutputTo(value: string): OutputTo {
  if (value === "split" || value === "stdout" || value === "stderr") {
    return value;
  }
  throw new Error(`invalid --output-to '${value}': expected split, stdout, or stderr`);
}

/** Event writer bound to the process stdout stream. */
const processStdoutWriter: CliEventWriter = (line) => {
  process.stdout.write(line);
};

/**
 * Event writer bound to the process stderr stream. This is the CliEmitter's
 * blessed diagnostic sink: stderr is sanctioned here, and only here, precisely
 * because output flows through the emitter rather than an ad-hoc write. The
 * `stderr-sink` marker is what no_stderr_policy.test.ts allows through.
 */
const processStderrWriter: CliEventWriter = (line) => {
  process.stderr.write(line); // stderr-sink: CliEmitter diagnostic channel
};

/**
 * Stateful emitter for structured CLI executions.
 *
 * Routing follows the consumption mode (OutputTo):
 *
 * - Finite one-shot (`CliEmitter.finite` / `finiteWith` /
 *   `fromOutputTo("split")`): `result` → the primary writer (stdout);
 *   `error`/`progress`/`log` → the diagnostic writer (stderr). The recommended
 *   default for a one-shot CLI, so shell capture and pipelines never treat a
 *   failure as data.
 * - Event stream (`new CliEmitter(...)` / `CliEmitter.stream` /
 *   `fromOutputTo("stdout"|"stderr")`): every event, including `error`, goes to
 *   the single writer, preserving interleaved ordering.
 */
export class CliEmitter {
  private terminalEmitted = false;
  private logFieldsProvider?: () => Record<string, JsonValue>;
  private diagnostic?: CliEventWriter;

  /**
   * Create an event-stream emitter: every event, including `error`, goes to the
   * single `writer`. Use CliEmitter.finite for a one-shot command that should
   * split `result`/`error` across stdout/stderr.
   */
  constructor(
    private readonly writer: CliEventWriter,
    private readonly format: OutputFormat,
    private readonly outputOptions: OutputOptions = {},
  ) {}

  /**
   * Create an event-stream emitter: every event goes to `writer`, preserving
   * interleaved ordering. Pick this when the consumer reads one ordered stream
   * and branches on `kind`. Alias for the constructor's unified form.
   */
  static stream(writer: CliEventWriter, format: OutputFormat, outputOptions: OutputOptions = {}): CliEmitter {
    return new CliEmitter(writer, format, outputOptions);
  }

  /**
   * Create a finite one-shot emitter with explicit sinks: `result` goes to
   * `resultWriter`, while `error`/`progress`/`log` go to `diagnostic`.
   */
  static finiteWith(
    resultWriter: CliEventWriter,
    diagnostic: CliEventWriter,
    format: OutputFormat,
    outputOptions: OutputOptions = {},
  ): CliEmitter {
    const emitter = new CliEmitter(resultWriter, format, outputOptions);
    emitter.diagnostic = diagnostic;
    return emitter;
  }

  /**
   * Create a finite one-shot emitter wired to the process streams: `result` →
   * stdout, `error`/`progress`/`log` → stderr. The recommended default for a
   * one-shot CLI.
   */
  static finite(format: OutputFormat, outputOptions: OutputOptions = {}): CliEmitter {
    return CliEmitter.finiteWith(processStdoutWriter, processStderrWriter, format, outputOptions);
  }

  /**
   * Build an emitter from a parsed OutputTo selector, wired to the process
   * streams: "split" is finite mode (`result` → stdout, everything else →
   * stderr); "stdout"/"stderr" are event-stream mode onto that one stream.
   */
  static fromOutputTo(selector: OutputTo, format: OutputFormat, outputOptions: OutputOptions = {}): CliEmitter {
    if (selector === "split") return CliEmitter.finite(format, outputOptions);
    const writer = selector === "stdout" ? processStdoutWriter : processStderrWriter;
    return CliEmitter.stream(writer, format, outputOptions);
  }

  withLogFields(provider: () => Record<string, JsonValue>): this {
    this.logFieldsProvider = provider;
    return this;
  }

  /**
   * Select the sink for an event by `kind`. Finite mode (a diagnostic sink is
   * present) keeps `result` on the primary writer (stdout) and routes
   * `error`/`progress`/`log` to the diagnostic writer (stderr). Event-stream
   * mode (no diagnostic sink) keeps every event on the single writer.
   */
  private sinkFor(kind: string): CliEventWriter {
    return kind !== "result" && this.diagnostic ? this.diagnostic : this.writer;
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
    this.sinkFor(kind)(`${render(jsonValue, this.format, this.outputOptions)}\n`);
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
    this.sinkFor(kind as string)(`${render(value, this.format, this.outputOptions)}\n`);
    if (kind === "result" || kind === "error") this.terminalEmitted = true;
  }

  private static isBrokenPipe(err: unknown): boolean {
    return (
      typeof err === "object" && err !== null && (err as NodeJS.ErrnoException).code === "EPIPE"
    );
  }

  /**
   * Emit `event` as the terminal event and resolve the outcome to a process
   * exit code, so a one-shot CLI need not hand-roll the emit-then-exit dance.
   *
   * A successful write returns `successCode`; a broken pipe (the reader hung up)
   * returns `0`; any other write or validation failure returns `4`. A library
   * never calls process.exit itself — return this code and let the caller exit.
   */
  finish(event: Event, successCode: number): number {
    try {
      this.emit(event);
      return successCode;
    } catch (err) {
      return CliEmitter.isBrokenPipe(err) ? 0 : 4;
    }
  }

  /**
   * Convenience over finish: emit a `result` payload and return `0` on success.
   *
   * Errors — simple or rich — go through the builder instead: construct the event
   * with jsonError(code, message).hint(...)…build() (the builder is the error
   * "type", so it scales to `retryable`/extra fields) and call finish(event, code).
   */
  finishResult(payload: JsonValue): number {
    return this.finish(jsonResult(payload).build(), 0);
  }
}

/**
 * Build a standard CLI version event: a `kind:"result"` event whose payload is
 * `{ code: "version", name, version }`, plus `display_name`/`build` when given.
 * `name` is the short/bin identity (e.g. `"afdata"`); `displayName` is an
 * optional human-facing product name (e.g. `"Agent-First Data"`); `build` is an
 * opaque caller-supplied identifier (a git commit SHA, for example) — its
 * meaning is entirely up to the caller. Both are `undefined` when unavailable,
 * and simply absent from the payload. Note the JSON field is `display_name`
 * (snake_case) even though the parameter is `displayName`.
 */
export function buildCliVersion(
  name: string,
  displayName: string | undefined,
  version: string,
  build: string | undefined,
): Event {
  return jsonResult({
    code: "version",
    name,
    version,
    ...(displayName !== undefined ? { display_name: displayName } : {}),
    ...(build !== undefined ? { build } : {}),
  }).build();
}

/**
 * Render a CLI version response as a protocol-v1 event in `format`. There is no
 * conventional-text path — `format` is required.
 */
export function cliRenderVersion(
  name: string,
  displayName: string | undefined,
  version: string,
  build: string | undefined,
  format: OutputFormat,
): string {
  const rendered = render(buildCliVersion(name, displayName, version, build), format);
  return `${rendered.replace(/\n+$/u, "")}\n`;
}

/**
 * Split a flag argument into its long name (leading dashes stripped, any
 * `=value` suffix dropped) and inline value. Returns `undefined` name for a
 * non-flag or the bare `-`. `--output=` yields an empty-string inline value.
 */
function splitFlag(arg: string): { name: string | undefined; inlineValue: string | undefined } {
  if (!arg.startsWith("-") || arg === "-") return { name: undefined, inlineValue: undefined };
  const eq = arg.indexOf("=");
  const flag = eq === -1 ? arg : arg.slice(0, eq);
  const inlineValue = eq === -1 ? undefined : arg.slice(eq + 1);
  const name = flag.replace(/^-+/u, "");
  if (name === "") return { name: undefined, inlineValue: undefined };
  return { name, inlineValue };
}

/**
 * Render version output if --version/-V is present; otherwise return undefined.
 * Throws for malformed version requests, for example `--version --output xml`.
 *
 * The one blessed behavior: `--version` always answers with a protocol-v1
 * `kind:"result"` version event — JSON by default, or `--output yaml|plain` (or
 * `--json`) for another format. There is no conventional bare-text path.
 *
 * `valueFlags` is the caller's own value-taking global long flags (with or
 * without leading dashes; the leading dashes are stripped internally). It is the
 * TS analog of Rust's `&clap::Command`: the pre-parser consults it so a global
 * flag's space-separated value (e.g. `--log request,startup`) is never mistaken
 * for the subcommand boundary, which would hide a later `--version`/`--output`.
 *
 * Only a top-level version request is recognized: scanning stops at the first
 * positional argument (the subcommand) or `--`, so `tool sub --version <value>`
 * leaves `--version` for the subcommand's parser rather than printing the
 * tool version.
 */
export function cliHandleVersionOrContinue(
  rawArgs: string[],
  valueFlags: string[],
  name: string,
  displayName: string | undefined,
  version: string,
  build: string | undefined,
): string | undefined {
  const valueFlagSet = new Set(valueFlags.map((flag) => flag.replace(/^-+/u, "")));
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

    const { name: flagName, inlineValue } = splitFlag(arg);

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
    // `--output-to` takes a value but does not affect version text output.
    // Consume its space-separated value so it is not mistaken for the
    // subcommand boundary (which would hide a later `--version`/`--output`).
    if (flagName === "output-to") {
      const hasSpaceValue =
        inlineValue === undefined && rawArgs[i + 1] !== undefined && !rawArgs[i + 1]!.startsWith("-");
      i += hasSpaceValue ? 2 : 1;
      continue;
    }
    if (flagName === "output") {
      let value: string | undefined;
      if (inlineValue !== undefined) {
        value = inlineValue;
      } else if (rawArgs[i + 1] !== undefined && !rawArgs[i + 1]!.startsWith("-")) {
        value = rawArgs[i + 1];
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
      i += inlineValue !== undefined || value === undefined ? 1 : 2;
      continue;
    }

    // Any other flag: consult the caller's own value-taking global flags so a
    // flag's space-separated value is never mistaken for the subcommand
    // boundary above.
    const hasSpaceValue =
      inlineValue === undefined && rawArgs[i + 1] !== undefined && !rawArgs[i + 1]!.startsWith("-");
    i += hasSpaceValue && flagName !== undefined && valueFlagSet.has(flagName) ? 2 : 1;
  }

  if (!versionRequested) return undefined;
  if (outputError !== undefined) throw outputError;
  return cliRenderVersion(name, displayName, version, build, outputFormat ?? "json");
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
