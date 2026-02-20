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

// BuildCliError builds a standard CLI parse error value.
// Use when flag parsing fails or a flag value is invalid.
// Print with OutputJson and exit with code 2.
func BuildCliError(message string) map[string]any {
	return map[string]any{
		"code":       "error",
		"error_code": "invalid_request",
		"error":      message,
		"retryable":  false,
		"trace":      map[string]any{"duration_ms": 0},
	}
}
