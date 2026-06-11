package afdata

import (
	"fmt"
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

// CliParseLogFilters normalizes --log flag entries: trim, lowercase, deduplicate, remove empty.
// Accepts pre-split entries (e.g. after strings.Split(flag, ",")).
func CliParseLogFilters(entries []string) []string {
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
	return out
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

// BuildCliVersion builds a standard CLI version value.
func BuildCliVersion(version string) map[string]any {
	return BuildJson("version", map[string]any{"version": version}, nil)
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

// BuildCliError builds a standard CLI parse error value.
// Use when flag parsing fails or a flag value is invalid.
// Print with OutputJson and exit with code 2.
// Pass empty string for hint to omit it.
func BuildCliError(message string, hint string) map[string]any {
	m := map[string]any{
		"code":  "error",
		"error": message,
	}
	if hint != "" {
		m["hint"] = hint
	}
	return m
}
