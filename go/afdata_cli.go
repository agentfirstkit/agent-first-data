package afdata

import (
	"fmt"
	"io"
	"strings"
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

// CliEmitter is a stateful emitter for finite structured CLI executions.
type CliEmitter struct {
	writer          io.Writer
	format          OutputFormat
	outputOptions   OutputOptions
	terminalEmitted bool
	logFieldsFunc   func() map[string]any
}

// NewCliEmitter creates an emitter with default output options.
func NewCliEmitter(writer io.Writer, format OutputFormat) *CliEmitter {
	return NewCliEmitterWithOptions(writer, format, OutputOptions{})
}

// NewCliEmitterWithOptions creates an emitter with explicit output options.
func NewCliEmitterWithOptions(writer io.Writer, format OutputFormat, outputOptions OutputOptions) *CliEmitter {
	return &CliEmitter{
		writer:        writer,
		format:        format,
		outputOptions: outputOptions,
	}
}

// WithLogFields sets a provider function that returns default fields for every log event.
// The provider is called for each log emission. Explicit fields in the log take precedence
// over provider fields.
func (e *CliEmitter) WithLogFields(provider func() map[string]any) *CliEmitter {
	e.logFieldsFunc = provider
	return e
}

// Emit accepts a typed Event and writes it as one line.
func (e *CliEmitter) Emit(event Event) error {
	envelope := event.Value()
	kind := envelope["kind"].(string)
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
	_, err := io.WriteString(e.writer, Render(envelope, e.format, e.outputOptions)+"\n")
	if err != nil {
		return fmt.Errorf("failed to write CLI event: %w", err)
	}
	if kind == "result" || kind == "error" {
		e.terminalEmitted = true
	}
	return nil
}

// EmitValidatedValue accepts untyped JSON and applies strict validation before emitting.
// Use only when dynamic JSON must be accepted. Prefer the typed Emit() for normal use.
func (e *CliEmitter) EmitValidatedValue(value any) error {
	if err := ValidateProtocolEvent(value, true); err != nil {
		return err
	}
	envelope := value.(map[string]any)
	kind := envelope["kind"].(string)
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
	_, err := io.WriteString(e.writer, Render(envelope, e.format, e.outputOptions)+"\n")
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

// BuildCliVersion builds a standard CLI version value as a map (for compatibility).
// The structured version event follows the protocol-v1 shape shared by the other
// SDKs: a "version"-coded result plus an empty trace.
func BuildCliVersion(version string) map[string]any {
	return map[string]any{
		"kind": "result",
		"result": map[string]any{
			"code":    "version",
			"version": version,
		},
		"trace": map[string]any{},
	}
}

// CliRenderVersion renders CLI version output.
// Pass an OutputFormat for AFDATA JSON/YAML/plain. Pass the empty string to
// preserve conventional "<name> <version>" text.
func CliRenderVersion(name string, version string, format OutputFormat) string {
	var rendered string
	if format == "" {
		rendered = fmt.Sprintf("%s %s", name, version)
	} else {
		rendered = Render(BuildCliVersion(version), format, OutputOptions{})
	}
	return strings.TrimRight(rendered, "\n") + "\n"
}

// CliHandleVersionOrContinue renders version output if --version/-V is present.
// It returns handled=false when no version flag was present. A bare --version/-V
// always prints conventional "<name> <version>" text; a structured event is
// emitted only when the output format is requested explicitly via --output or
// --json.
//
// Only a top-level version request is recognized: scanning stops at the first
// positional argument (the subcommand), so "tool sub --version <value>" leaves
// --version for the subcommand's parser rather than printing the tool version.
func CliHandleVersionOrContinue(args []string, name string, version string) (out string, handled bool, err error) {
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
		if arg == "--output" || strings.HasPrefix(arg, "--output=") {
			var value string
			if strings.HasPrefix(arg, "--output=") {
				value = strings.TrimPrefix(arg, "--output=")
				i++
			} else if i+1 < len(args) && !strings.HasPrefix(args[i+1], "-") {
				value = args[i+1]
				i += 2
			} else {
				err = fmt.Errorf("missing value for --output: expected json, yaml, or plain")
				i++
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
		i++
	}

	if !versionRequested {
		return "", false, nil
	}
	if err != nil {
		return "", true, err
	}
	if outputExplicit {
		return CliRenderVersion(name, version, outputFormat), true, nil
	}
	return CliRenderVersion(name, version, ""), true, nil
}
