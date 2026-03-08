// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: CliParseOutput, CliParseLogFilters, CliOutput, BuildCliError,
// --dry-run, and error hints.
//
// Run: go run ./examples/agent_cli echo
//      go run ./examples/agent_cli echo --dry-run
//      go run ./examples/agent_cli ping
package main

import (
	"fmt"
	"os"
	"strings"

	afdata "github.com/cmnspore/agent-first-data/go"
)

var validActions = []string{"echo", "ping"}

func main() {
	// Minimal flag parsing (real tools use cobra/pflag).
	action := ""
	output := "json"
	dryRun := false
	logArg := ""

	args := os.Args[1:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--output":
			i++
			if i < len(args) {
				output = args[i]
			}
		case "--log":
			i++
			if i < len(args) {
				logArg = args[i]
			}
		case "--dry-run":
			dryRun = true
		default:
			if action == "" && !strings.HasPrefix(args[i], "--") {
				action = args[i]
			}
		}
	}

	// 1. Parse --output flag with structured error on failure.
	format, err := afdata.CliParseOutput(output)
	if err != nil {
		fmt.Println(afdata.OutputJson(afdata.BuildCliError(err.Error(), "")))
		os.Exit(2)
	}

	// 2. Normalize --log filters: trim, lowercase, deduplicate.
	var filters []string
	if logArg != "" {
		filters = afdata.CliParseLogFilters(strings.Split(logArg, ","))
	}

	// 3. Validate action — demonstrate BuildCliError with hint.
	if action == "" {
		action = "echo"
	}
	if !contains(validActions, action) {
		msg := fmt.Sprintf("unknown action: %s", action)
		hint := fmt.Sprintf("valid actions: %s", strings.Join(validActions, ", "))
		fmt.Println(afdata.OutputJson(afdata.BuildCliError(msg, hint)))
		os.Exit(2)
	}

	// 4. --dry-run → preview without executing.
	if dryRun {
		preview := afdata.BuildJson("dry_run", map[string]any{
			"action": action,
			"log":    filters,
		}, map[string]any{"duration_ms": 0})
		fmt.Println(afdata.CliOutput(preview, format))
		return
	}

	// 5. Do work — demonstrate BuildJsonError with hint on failure.
	if action == "ping" {
		errVal := afdata.BuildJsonError("ping target not configured", "set PING_HOST or pass --host", map[string]any{"duration_ms": 0})
		fmt.Println(afdata.CliOutput(errVal, format))
		os.Exit(1)
	}

	// 6. Format and emit output.
	result := afdata.BuildJsonOk(map[string]any{
		"action": action,
		"log":    filters,
	}, nil)
	fmt.Println(afdata.CliOutput(result, format))
	_ = filters
}

func contains(ss []string, s string) bool {
	for _, v := range ss {
		if v == s {
			return true
		}
	}
	return false
}
