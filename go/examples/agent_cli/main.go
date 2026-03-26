// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: complete --help (all subcommands in one output),
// CliParseOutput, CliParseLogFilters, CliOutput, BuildCliError,
// --dry-run, and error hints.
//
// Run: go run ./examples/agent_cli --help
//
//	go run ./examples/agent_cli echo --help
//	go run ./examples/agent_cli echo
//	go run ./examples/agent_cli echo --dry-run
//	go run ./examples/agent_cli ping
package main

import (
	"fmt"
	"os"
	"strings"

	afdata "github.com/cmnspore/agent-first-data/go"
)

type subcommand struct {
	name  string
	about string
	flags string
}

var subcommands = []subcommand{
	{name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing"},
	{name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping"},
}

// formatCompleteHelp returns help for the root command and all subcommands.
func formatCompleteHelp() string {
	var b strings.Builder
	b.WriteString("agent-cli — Minimal agent-first CLI example\n\n")
	b.WriteString("Usage: agent-cli [OPTIONS] <COMMAND>\n\n")
	b.WriteString("Options:\n")
	b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json)\n")
	b.WriteString("  --log <FILTERS>    Log categories (comma-separated)\n")
	b.WriteString("  --help             Show this help\n\n")
	b.WriteString("Commands:\n")
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "  %-8s %s\n", sc.name, sc.about)
	}
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "\n%s\n%s\n\n", strings.Repeat("=", 60), "agent-cli "+sc.name)
		fmt.Fprintf(&b, "%s\n%s\n\nFlags:\n%s\n", strings.Repeat("=", 60), sc.about, sc.flags)
	}
	return b.String()
}

// formatSubcommandHelp returns help for a single subcommand.
func formatSubcommandHelp(name string) string {
	for _, sc := range subcommands {
		if sc.name == name {
			var b strings.Builder
			fmt.Fprintf(&b, "agent-cli %s — %s\n\nFlags:\n%s\n", sc.name, sc.about, sc.flags)
			return b.String()
		}
	}
	return ""
}

func main() {
	command := ""
	output := "json"
	dryRun := false
	logArg := ""
	host := ""
	showHelp := false

	args := os.Args[1:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--help", "-h":
			showHelp = true
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
		case "--host":
			i++
			if i < len(args) {
				host = args[i]
			}
		default:
			if command == "" && !strings.HasPrefix(args[i], "--") {
				command = args[i]
			}
		}
	}

	// Complete help: --help expands all subcommands in one output.
	// Subcommand --help expands only that subcommand.
	if showHelp {
		if command != "" {
			fmt.Print(formatSubcommandHelp(command))
		} else {
			fmt.Print(formatCompleteHelp())
		}
		return
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

	// 3. No subcommand → error with hint.
	if command == "" {
		fmt.Println(afdata.OutputJson(afdata.BuildCliError("no subcommand provided", "try: agent-cli --help")))
		os.Exit(2)
	}

	switch command {
	case "echo":
		// 4. --dry-run → preview without executing.
		if dryRun {
			preview := afdata.BuildJson("dry_run", map[string]any{
				"action": "echo",
				"log":    filters,
			}, map[string]any{"duration_ms": 0})
			fmt.Println(afdata.CliOutput(preview, format))
			return
		}

		result := afdata.BuildJsonOk(map[string]any{
			"action": "echo",
			"log":    filters,
		}, nil)
		fmt.Println(afdata.CliOutput(result, format))

	case "ping":
		// 5. Demonstrate BuildJsonError with hint on failure.
		if host == "" {
			errVal := afdata.BuildJsonError("ping target not configured", "set PING_HOST or pass --host", map[string]any{"duration_ms": 0})
			fmt.Println(afdata.CliOutput(errVal, format))
			os.Exit(1)
		}

	default:
		hint := "valid commands: echo, ping"
		fmt.Println(afdata.OutputJson(afdata.BuildCliError("unknown command: "+command, hint)))
		os.Exit(2)
	}
}
