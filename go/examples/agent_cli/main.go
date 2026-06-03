// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: complete --help (all subcommands in one output),
// CliParseOutput, CliParseLogFilters, CliOutput, BuildCliError,
// --dry-run, error hints, and a `skill` subcommand that installs/uninstalls/
// reports status of an embedded Agent Skill across Codex, Claude Code, and opencode.
//
// Run: go run ./examples/agent_cli --help
//
//	go run ./examples/agent_cli echo --help
//	go run ./examples/agent_cli echo
//	go run ./examples/agent_cli echo --dry-run
//	go run ./examples/agent_cli ping
//	go run ./examples/agent_cli skill status --agent opencode --skills-dir /tmp/ex
//	go run ./examples/agent_cli skill install --agent opencode --skills-dir /tmp/ex
package main

import (
	"fmt"
	"os"
	"strings"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

// A fictional spore's embedded Agent Skill, used by the `skill` subcommand to
// demonstrate afdata.RunSkillAdmin.
const widgetSkill = "---\nname: agent-first-widget\ndescription: Example skill bundled by the agent-cli demo.\n---\n\n# Agent-First Widget\n\nExample behavior rules go here.\n"

var widgetSpec = afdata.SkillSpec{
	Name:       "agent-first-widget",
	Source:     widgetSkill,
	Title:      "Agent-First Widget",
	MarkerSlug: "afwidget",
}

type subcommand struct {
	name  string
	about string
	flags string
}

var subcommands = []subcommand{
	{name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing"},
	{name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping"},
	{name: "skill", about: "Manage this tool's embedded Agent Skill", flags: "  status|install|uninstall  Skill action\n  --agent      all, codex, claude-code, opencode (default: all)\n  --scope      personal, project (default: personal)\n  --skills-dir Skills directory (requires a single concrete --agent)\n  --force      Overwrite or remove a skill this tool did not manage"},
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
	output := "json"
	dryRun := false
	logArg := ""
	host := ""
	agent := "all"
	scope := "personal"
	skillsDir := ""
	force := false
	showHelp := false
	var positionals []string

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
		case "--agent":
			i++
			if i < len(args) {
				agent = args[i]
			}
		case "--scope":
			i++
			if i < len(args) {
				scope = args[i]
			}
		case "--skills-dir":
			i++
			if i < len(args) {
				skillsDir = args[i]
			}
		case "--force":
			force = true
		default:
			if !strings.HasPrefix(args[i], "--") {
				positionals = append(positionals, args[i])
			}
		}
	}

	command := ""
	if len(positionals) > 0 {
		command = positionals[0]
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

	case "skill":
		os.Exit(runSkill(positionals, agent, scope, skillsDir, force, format))

	default:
		hint := "valid commands: echo, ping, skill"
		fmt.Println(afdata.OutputJson(afdata.BuildCliError("unknown command: "+command, hint)))
		os.Exit(2)
	}
}

// runSkill wires the parsed `skill` subcommand to the library and prints the result.
// Returns the process exit code (0 ok, 1 action error, 2 bad flag value).
func runSkill(positionals []string, agentStr, scopeStr, skillsDir string, force bool, format afdata.OutputFormat) int {
	verb := ""
	if len(positionals) > 1 {
		verb = positionals[1]
	}
	var action afdata.SkillAction
	switch verb {
	case "status":
		action = afdata.SkillActionStatus
	case "install":
		action = afdata.SkillActionInstall
	case "uninstall":
		action = afdata.SkillActionUninstall
	default:
		errVal := afdata.BuildCliError("skill requires a subcommand: status, install, uninstall", "example: agent-cli skill status --agent opencode")
		fmt.Println(afdata.CliOutput(errVal, format))
		return 2
	}

	opts, message, hint := buildSkillOptions(agentStr, scopeStr, skillsDir, force)
	if message != "" {
		fmt.Println(afdata.CliOutput(afdata.BuildCliError(message, hint), format))
		return 2
	}

	report, serr := afdata.RunSkillAdmin(widgetSpec, action, opts)
	if serr != nil {
		fmt.Println(afdata.CliOutput(afdata.BuildCliError(serr.Message, serr.Hint), format))
		return 1
	}
	fmt.Println(afdata.CliOutput(report, format))
	return 0
}

// buildSkillOptions parses the --agent/--scope string flags into the library enums.
// Returns a non-empty message (and optional hint) on an unknown value.
func buildSkillOptions(agentStr, scopeStr, skillsDir string, force bool) (afdata.SkillOptions, string, string) {
	var agent afdata.SkillAgentSelection
	switch agentStr {
	case "all":
		agent = afdata.SkillAgentAll
	case "codex":
		agent = afdata.SkillAgentCodex
	case "claude-code":
		agent = afdata.SkillAgentClaudeCode
	case "opencode":
		agent = afdata.SkillAgentOpencode
	default:
		return afdata.SkillOptions{}, fmt.Sprintf("invalid --agent '%s'", agentStr), "valid values: all, codex, claude-code, opencode"
	}
	var sc afdata.SkillScope
	switch scopeStr {
	case "personal":
		sc = afdata.SkillScopePersonal
	case "project":
		sc = afdata.SkillScopeProject
	default:
		return afdata.SkillOptions{}, fmt.Sprintf("invalid --scope '%s'", scopeStr), "valid values: personal, project"
	}
	return afdata.SkillOptions{Agent: agent, Scope: sc, SkillsDir: skillsDir, Force: force}, "", ""
}
