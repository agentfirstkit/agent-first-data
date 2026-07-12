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
// Empty filter list returns false; "all" or "*" return true; otherwise prefix-matched against lowercase event.
func (lf LogFilters) Enabled(event string) bool {
	if len(lf.filters) == 0 {
		return false
	}
	lower := strings.ToLower(event)
	for _, filter := range lf.filters {
		if filter == "all" || filter == "*" {
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

// CliOutput dispatches output formatting by OutputFormat.
// Equivalent to calling OutputJson, OutputYaml, or OutputPlain directly.
func CliOutput(value any, format OutputFormat) string {
	switch format {
	case OutputFormatYaml:
		return OutputYaml(value)
	case OutputFormatPlain:
		return OutputPlain(value)
	default:
		return OutputJson(value)
	}
}

// CliOutputWithOptions dispatches output formatting with explicit redaction and style.
// JSON ignores OutputStyle and preserves original keys and values after redaction.
func CliOutputWithOptions(value any, format OutputFormat, outputOptions OutputOptions) string {
	switch format {
	case OutputFormatYaml:
		return OutputYamlWithOptions(value, outputOptions)
	case OutputFormatPlain:
		return OutputPlainWithOptions(value, outputOptions)
	default:
		return OutputJsonWithOptions(value, outputOptions)
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
// over provider fields. If the provider returns a reserved field (message, level, code),
// Emit returns a typed error.
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
	_, err := io.WriteString(e.writer, CliOutputWithOptions(envelope, e.format, e.outputOptions)+"\n")
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
	_, err := io.WriteString(e.writer, CliOutputWithOptions(envelope, e.format, e.outputOptions)+"\n")
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
	event, err := NewJSONResult(payload).Build()
	if err != nil {
		return err
	}
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
	event, err := NewJSONProgress(message).Build()
	if err != nil {
		return err
	}
	return e.Emit(event)
}

// EmitLog emits a log event with the given level and message.
// Default log fields (if configured via WithLogFields) are merged, with explicit
// fields taking precedence. Returns an error if the provider or fields write to
// reserved field names.
func (e *CliEmitter) EmitLog(level LogLevel, message string) error {
	builder := NewJSONLog(level, message)

	// Merge log fields provider if configured
	if e.logFieldsFunc != nil {
		providerFields := e.logFieldsFunc()
		for k, v := range providerFields {
			if isReservedLogField(k) {
				return &BuilderError{msg: fmt.Sprintf("log fields provider returned reserved field %q", k)}
			}
			// Don't overwrite if already set; provider fields have lower precedence
			if _, alreadySet := builder.fields[k]; !alreadySet {
				builder.fields[k] = v
			}
		}
	}

	event, err := builder.Build()
	if err != nil {
		return err
	}
	return e.Emit(event)
}

// BuildCliVersion builds a standard CLI version value as a map (for compatibility).
// Returns a map without trace field for conventional CLI version output.
func BuildCliVersion(version string) map[string]any {
	return map[string]any{
		"kind": "result",
		"result": map[string]any{
			"version": version,
		},
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
		rendered = CliOutput(BuildCliVersion(version), format)
	}
	return strings.TrimRight(rendered, "\n") + "\n"
}

// CliHandleVersionOrContinue renders version output if --version/-V is present.
// It returns handled=false when no version flag was present. defaultOutput
// controls the bare --version format; pass OutputFormatJson for AFDATA-first
// CLIs or the empty string for conventional text.
func CliHandleVersionOrContinue(args []string, name string, version string, defaultOutput OutputFormat) (out string, handled bool, err error) {
	versionRequested := false
	outputFormat := OutputFormat("")
	outputExplicit := false

	for i := 0; i < len(args); {
		arg := args[i]
		if arg == "--" {
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
	return CliRenderVersion(name, version, defaultOutput), true, nil
}
