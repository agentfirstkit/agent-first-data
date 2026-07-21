package afdata

import (
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"syscall"
)

// ═══════════════════════════════════════════
// Public API: CLI Helpers
// ═══════════════════════════════════════════

// OutputFormat represents the output format for CLI and pipe/MCP modes.
type OutputFormat string

const (
	OutputFormatJson  OutputFormat = "json"
	OutputFormatYaml  OutputFormat = "yaml"
	OutputFormatPlain OutputFormat = "plain"
)

// CliParseOutput parses the --output flag value into an OutputFormat.
// Returns an error with a message suitable for BuildCliError on unknown values.
func CliParseOutput(s string) (OutputFormat, error) {
	switch s {
	case "json":
		return OutputFormatJson, nil
	case "yaml":
		return OutputFormatYaml, nil
	case "plain":
		return OutputFormatPlain, nil
	default:
		return "", fmt.Errorf("invalid --output format %q: expected json, yaml, or plain", s)
	}
}

// LogFilters represents parsed log filter entries with enabled/prefix matching.
type LogFilters struct {
	filters []string
}

// CliParseLogFilters normalizes --log flag entries: trim, lowercase, deduplicate, remove empty.
// Accepts pre-split entries (e.g. after strings.Split(flag, ",")).
// Returns a LogFilters type that supports Enabled() prefix matching.
func CliParseLogFilters(entries []string) LogFilters {
	var out []string
	for _, entry := range entries {
		s := strings.ToLower(strings.TrimSpace(entry))
		if s == "" {
			continue
		}
		duplicate := false
		for _, existing := range out {
			if existing == s {
				duplicate = true
				break
			}
		}
		if !duplicate {
			out = append(out, s)
		}
	}
	return LogFilters{filters: out}
}

// Enabled returns true if the event is enabled by the filters.
// An empty filter list returns false (filtering is opt-in); the single wildcard
// word "all" returns true ("*" is not special); otherwise the event is
// prefix-matched (lowercased) against each filter, so a mistyped filter matches
// nothing and silently emits no output.
func (lf LogFilters) Enabled(event string) bool {
	if len(lf.filters) == 0 {
		return false
	}
	lower := strings.ToLower(event)
	for _, filter := range lf.filters {
		if filter == "all" {
			return true
		}
		if strings.HasPrefix(lower, filter) {
			return true
		}
	}
	return false
}

// IsEmpty returns true if no filters are set.
func (lf LogFilters) IsEmpty() bool {
	return len(lf.filters) == 0
}

// Values returns the underlying slice of filter values.
func (lf LogFilters) Values() []string {
	return append([]string(nil), lf.filters...)
}

// Render renders a value as a string in the given format with the given
// options. The single value × format × options → string entry point,
// replacing the former OutputJson/OutputYaml/OutputPlain (+WithOptions) and
// CliOutput/CliOutputWithOptions families. Pass a zero OutputOptions{} for
// defaults.
//
// JSON and YAML are structure-preserving and ignore PlainStyle: they keep
// original keys and values after redaction. Plain (logfmt) honors PlainStyle.
func Render(value any, format OutputFormat, options OutputOptions) string {
	switch format {
	case OutputFormatYaml:
		return renderYaml(value, options)
	case OutputFormatPlain:
		return renderPlain(value, options)
	default:
		return renderJSON(value, options)
	}
}

// OutputTo selects where a CliEmitter sends its events, as chosen by the
// --output-to flag. The stream an event lands on follows the program's
// consumption mode, not the event's shape (see the spec's CLI Event Framing):
//
//   - OutputToSplit (the default) is finite one-shot mode: result goes to
//     stdout, while error/progress/log go to stderr. stdout therefore carries
//     only successful payloads, so a shell capture or pipe never mistakes a
//     failure for data.
//   - OutputToStdout / OutputToStderr are event-stream mode: every event,
//     including error, is collapsed onto that one stream so a consumer reading
//     it in order (branching on kind) sees preserved ordering.
type OutputTo string

const (
	// OutputToSplit is finite one-shot: result → stdout, error/progress/log → stderr.
	OutputToSplit OutputTo = "split"
	// OutputToStdout is event-stream mode: every event onto stdout.
	OutputToStdout OutputTo = "stdout"
	// OutputToStderr is event-stream mode: every event onto stderr.
	OutputToStderr OutputTo = "stderr"
)

// ParseOutputTo parses an --output-to value: split (default), stdout, or stderr.
// It returns an error with a message suitable for BuildCliError on unknown values.
func ParseOutputTo(value string) (OutputTo, error) {
	switch value {
	case "split":
		return OutputToSplit, nil
	case "stdout":
		return OutputToStdout, nil
	case "stderr":
		return OutputToStderr, nil
	default:
		return "", fmt.Errorf("unsupported --output-to `%s`; expected split, stdout, or stderr", value)
	}
}

// CliEmitter is a stateful emitter for structured CLI executions.
//
// Routing follows the consumption mode. When a diagnostic writer is present
// (the finite constructors), it is one-shot mode: result stays on the primary
// writer (stdout) while error/progress/log are diagnostics routed to the
// diagnostic writer (stderr). When no diagnostic writer is present (the stream
// constructors), it is event-stream mode: every event, including error, goes to
// the single writer, preserving interleaved ordering. Routing follows the
// event kind, not the exit code.
type CliEmitter struct {
	writer          io.Writer
	diagnostic      io.Writer
	format          OutputFormat
	outputOptions   OutputOptions
	terminalEmitted bool
	logFieldsFunc   func() map[string]any
}

// NewCliEmitter creates an event-stream (unified) emitter: every event,
// including error, goes to writer. Alias for the stream form. Use
// NewCliEmitterFinite for a one-shot command that should split result/error
// across stdout/stderr.
func NewCliEmitter(writer io.Writer, format OutputFormat) *CliEmitter {
	return NewCliEmitterWithOptions(writer, format, OutputOptions{})
}

// NewCliEmitterWithOptions creates an event-stream (unified) emitter with
// explicit output options.
func NewCliEmitterWithOptions(writer io.Writer, format OutputFormat, outputOptions OutputOptions) *CliEmitter {
	return &CliEmitter{
		writer:        writer,
		format:        format,
		outputOptions: outputOptions,
	}
}

// NewCliEmitterFinite creates a finite one-shot emitter with explicit sinks:
// result goes to resultWriter, while error/progress/log go to diagnostic.
func NewCliEmitterFinite(resultWriter, diagnostic io.Writer, format OutputFormat) *CliEmitter {
	return NewCliEmitterFiniteWithOptions(resultWriter, diagnostic, format, OutputOptions{})
}

// NewCliEmitterFiniteWithOptions creates a finite one-shot emitter with explicit
// sinks and output options.
func NewCliEmitterFiniteWithOptions(resultWriter, diagnostic io.Writer, format OutputFormat, outputOptions OutputOptions) *CliEmitter {
	return &CliEmitter{
		writer:        resultWriter,
		diagnostic:    diagnostic,
		format:        format,
		outputOptions: outputOptions,
	}
}

// NewCliEmitterFromOutputTo builds an emitter from a parsed OutputTo selector,
// wired to the process streams: OutputToSplit is finite mode (result → stdout,
// everything else → stderr); OutputToStdout/OutputToStderr are event-stream mode
// onto that one stream.
func NewCliEmitterFromOutputTo(selector OutputTo, format OutputFormat) *CliEmitter {
	return NewCliEmitterFromOutputToWithOptions(selector, format, OutputOptions{})
}

// NewCliEmitterFromOutputToWithOptions is NewCliEmitterFromOutputTo with custom
// output options.
func NewCliEmitterFromOutputToWithOptions(selector OutputTo, format OutputFormat, outputOptions OutputOptions) *CliEmitter {
	switch selector {
	case OutputToStdout:
		return NewCliEmitterWithOptions(os.Stdout, format, outputOptions)
	case OutputToStderr:
		return NewCliEmitterWithOptions(os.Stderr, format, outputOptions)
	default: // OutputToSplit
		return NewCliEmitterFiniteWithOptions(os.Stdout, os.Stderr, format, outputOptions)
	}
}

// WithLogFields sets a provider function that returns default fields for every log event.
// The provider is called for each log emission. Explicit fields in the log take precedence
// over provider fields.
func (e *CliEmitter) WithLogFields(provider func() map[string]any) *CliEmitter {
	e.logFieldsFunc = provider
	return e
}

// Emit accepts a typed Event and writes it as one line, routed by kind.
func (e *CliEmitter) Emit(event Event) error {
	return e.writeEvent(event.Value())
}

// EmitValidatedValue accepts untyped JSON and applies strict validation before emitting.
// Use only when dynamic JSON must be accepted. Prefer the typed Emit() for normal use.
func (e *CliEmitter) EmitValidatedValue(value any) error {
	if err := ValidateProtocolEvent(value, true); err != nil {
		return err
	}
	envelope := value.(map[string]any)
	return e.writeEvent(envelope)
}

// writeEvent enforces terminal-lifecycle ordering, then renders and writes the
// event to the stream selected by kind. In finite mode (a diagnostic writer is
// present) result stays on the primary writer while error/progress/log are
// routed to the diagnostic writer; in event-stream mode (no diagnostic writer)
// every event goes to the single writer. Routing follows kind, not exit code.
func (e *CliEmitter) writeEvent(envelope map[string]any) error {
	kind, _ := envelope["kind"].(string)
	switch kind {
	case "log", "progress":
		if e.terminalEmitted {
			return fmt.Errorf("cannot emit non-terminal event after terminal event")
		}
	case "result", "error":
		if e.terminalEmitted {
			return fmt.Errorf("cannot emit duplicate terminal event")
		}
	default:
		return fmt.Errorf("unsupported event kind %q", kind)
	}
	sink := e.writer
	if e.diagnostic != nil && kind != "result" {
		sink = e.diagnostic
	}
	_, err := io.WriteString(sink, Render(envelope, e.format, e.outputOptions)+"\n")
	if err != nil {
		return fmt.Errorf("failed to write CLI event: %w", err)
	}
	if kind == "result" || kind == "error" {
		e.terminalEmitted = true
	}
	return nil
}

// EmitResult emits a result event with the given payload.
func (e *CliEmitter) EmitResult(payload any) error {
	event := NewJSONResult(payload).Build()
	return e.Emit(event)
}

// EmitError emits an error event with the given code and message.
func (e *CliEmitter) EmitError(code string, message string) error {
	event, err := NewJSONError(code, message).Build()
	if err != nil {
		return err
	}
	return e.Emit(event)
}

// EmitProgress emits a progress event with the given message.
func (e *CliEmitter) EmitProgress(message string) error {
	event := NewJSONProgress(map[string]any{"message": message}).Build()
	return e.Emit(event)
}

// EmitLog emits a log event with the given level and message.
// Default log fields (if configured via WithLogFields) are merged, with explicit
// fields taking precedence.
func (e *CliEmitter) EmitLog(level LogLevel, message string) error {
	payload := map[string]any{"level": string(level), "message": message}

	// Merge log fields provider if configured
	if e.logFieldsFunc != nil {
		providerFields := e.logFieldsFunc()
		for k, v := range providerFields {
			// Don't overwrite if already set; provider fields have lower precedence
			if _, alreadySet := payload[k]; !alreadySet {
				payload[k] = v
			}
		}
	}

	event := NewJSONLog(payload).Build()
	return e.Emit(event)
}

// Finish emits event (routed by kind exactly like Emit) and maps the outcome to
// a process exit code, collapsing the "emit a terminal event, then turn it into
// an exit status" boilerplate. It returns successCode when the event is written;
// 0 when the write failed because the reader hung up (a broken pipe, i.e.
// errors.Is(err, syscall.EPIPE) — the conventional clean shutdown when a
// consumer stops reading); and 4 on any other emit or write failure.
//
// A library must not call os.Exit itself: Finish returns the code and leaves the
// decision to the caller (typically os.Exit(code)).
func (e *CliEmitter) Finish(event Event, successCode int) int {
	err := e.Emit(event)
	switch {
	case err == nil:
		return successCode
	case errors.Is(err, syscall.EPIPE):
		return 0
	default:
		return 4
	}
}

// FinishResult builds a result event for payload, emits it to the primary
// (stdout) sink, and returns the exit code: 0 on success or a broken-pipe
// shutdown, 4 on any other write failure.
//
// Errors — simple or rich — go through the error builder instead: build the
// event with NewJSONError(code, message).Hint(...).RetryableIf(...).Build() (the
// builder is the error "type") and pass it to Finish(event, exitCode).
func (e *CliEmitter) FinishResult(payload any) int {
	return e.Finish(NewJSONResult(payload).Build(), 0)
}

// BuildCliVersion builds a standard CLI version value as a map: a protocol-v1
// "version"-coded result plus an empty trace, following the shape shared by the
// other SDKs. The result payload is {code:"version", name, version}, plus
// display_name and build only when non-empty. name is the short/bin identity
// (e.g. "afdata"); displayName is an optional human-facing product name (e.g.
// "Agent-First Data"); build is an opaque caller-supplied identifier (a git
// commit SHA, for example) whose meaning is entirely up to the caller. Passing
// "" for displayName or build means absent — the field is omitted from the
// payload.
func BuildCliVersion(name, displayName, version, build string) map[string]any {
	result := map[string]any{
		"code":    "version",
		"name":    name,
		"version": version,
	}
	if displayName != "" {
		result["display_name"] = displayName
	}
	if build != "" {
		result["build"] = build
	}
	return map[string]any{
		"kind":   "result",
		"result": result,
		"trace":  map[string]any{},
	}
}

// CliRenderVersion renders a CLI version response as a protocol-v1 event in the
// given format. There is no conventional "<name> <version>" text path: --version
// always answers with a structured event.
func CliRenderVersion(name, displayName, version, build string, format OutputFormat) string {
	rendered := Render(BuildCliVersion(name, displayName, version, build), format, OutputOptions{})
	return strings.TrimRight(rendered, "\n") + "\n"
}

// splitVersionFlag splits a flag arg into its long name (leading dashes and any
// "=inline" suffix stripped) and the inline value. hasInline reports whether an
// "=" was present. A non-flag arg (or a bare "-") yields an empty name.
func splitVersionFlag(arg string) (name string, value string, hasInline bool) {
	if !strings.HasPrefix(arg, "-") || arg == "-" {
		return "", "", false
	}
	body := arg
	if idx := strings.IndexByte(arg, '='); idx >= 0 {
		body = arg[:idx]
		value = arg[idx+1:]
		hasInline = true
	}
	return strings.TrimLeft(body, "-"), value, hasInline
}

// CliHandleVersionOrContinue renders version output if --version/-V is present.
// It returns handled=false when no version flag was present. --version always
// answers with a structured protocol-v1 version event — JSON by default, or the
// format requested via --output json|yaml|plain (or the --json alias). There is
// no conventional bare-text path.
//
// valueFlags is the caller's own value-taking global long flags (with or without
// leading dashes; each is normalized by stripping leading '-'). The pre-parser
// consumes a recognized value flag's space-separated value so it is never
// mistaken for the subcommand boundary — afdata's own --output/--output-to are
// handled here without needing to be listed.
//
// Only a top-level version request is recognized: scanning stops at the first
// positional argument (the subcommand), so "tool sub --version <value>" leaves
// --version for the subcommand's parser rather than printing the tool version.
func CliHandleVersionOrContinue(args []string, valueFlags []string, name, displayName, version, build string) (out string, handled bool, err error) {
	valueFlagSet := make(map[string]struct{}, len(valueFlags))
	for _, flag := range valueFlags {
		valueFlagSet[strings.TrimLeft(flag, "-")] = struct{}{}
	}

	versionRequested := false
	outputFormat := OutputFormat("")
	outputExplicit := false

	for i := 0; i < len(args); {
		arg := args[i]
		if arg == "--" {
			break
		}
		// The first positional argument marks the subcommand boundary. Past it,
		// --version and -V belong to the subcommand's own parser, matching
		// git/cargo/clap: this pre-parser only owns a top-level version request.
		if !strings.HasPrefix(arg, "-") {
			break
		}
		if arg == "--version" || arg == "-V" {
			versionRequested = true
			i++
			continue
		}
		if arg == "--json" {
			if outputExplicit && outputFormat != OutputFormatJson {
				err = fmt.Errorf("conflicting output formats: --json conflicts with previous output format")
			} else {
				outputFormat = OutputFormatJson
				outputExplicit = true
			}
			i++
			continue
		}

		flagName, inlineValue, hasInline := splitVersionFlag(arg)

		// --output-to takes a value but does not affect version output. Consume
		// its space-separated value so it is not mistaken for the subcommand
		// boundary (which would hide a later --version/--output).
		if flagName == "output-to" {
			if !hasInline && i+1 < len(args) && !strings.HasPrefix(args[i+1], "-") {
				i += 2
			} else {
				i++
			}
			continue
		}

		if flagName == "output" {
			var value string
			haveValue := false
			switch {
			case hasInline:
				value = inlineValue
				haveValue = true
				i++
			case i+1 < len(args) && !strings.HasPrefix(args[i+1], "-"):
				value = args[i+1]
				haveValue = true
				i += 2
			default:
				i++
			}
			if !haveValue {
				err = fmt.Errorf("missing value for --output: expected json, yaml, or plain")
				continue
			}
			parsed, parseErr := CliParseOutput(value)
			if parseErr != nil {
				err = parseErr
			} else if outputExplicit && outputFormat != parsed {
				err = fmt.Errorf("conflicting output formats: --output %s conflicts with previous output format", value)
			} else {
				outputFormat = parsed
				outputExplicit = true
			}
			continue
		}

		// Any other flag: consume a space-separated value only when the caller
		// declared this long flag as value-taking, so its value is never mistaken
		// for the subcommand boundary above.
		_, isValueFlag := valueFlagSet[flagName]
		if isValueFlag && !hasInline && i+1 < len(args) && !strings.HasPrefix(args[i+1], "-") {
			i += 2
		} else {
			i++
		}
	}

	if !versionRequested {
		return "", false, nil
	}
	if err != nil {
		return "", true, err
	}
	format := outputFormat
	if !outputExplicit {
		format = OutputFormatJson
	}
	return CliRenderVersion(name, displayName, version, build, format), true, nil
}
