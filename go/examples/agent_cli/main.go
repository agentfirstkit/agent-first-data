// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: human --help (one-level) plus orthogonal --recursive scope and
// --output json|yaml|markdown format for full surface export,
// CliParseOutput, CliParseLogFilters, CliOutput, BuildCliError,
// --dry-run, error hints, and a `skill` subcommand that installs/uninstalls/
// reports status of an embedded Agent Skill across Codex, Claude Code, and opencode.
//
// Run: go run ./examples/agent_cli --help
//
//	go run ./examples/agent_cli --help --recursive
//	go run ./examples/agent_cli --help --recursive --output json
//	go run ./examples/agent_cli --help --recursive --output markdown
//	go run ./examples/agent_cli --version --output json
//	go run ./examples/agent_cli echo --help
//	go run ./examples/agent_cli echo
//	go run ./examples/agent_cli echo --dry-run
//	go run ./examples/agent_cli --log all ping   # or --verbose
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

const agentCliVersion = "0.13.0"

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

// formatRootHelp returns one-level help for the root command.
func formatRootHelp() string {
	var b strings.Builder
	b.WriteString("agent-cli — Minimal agent-first CLI example\n\n")
	b.WriteString("Usage: agent-cli [OPTIONS] <COMMAND>\n\n")
	b.WriteString("Options:\n")
	b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n")
	b.WriteString("  --log <FILTERS>    Log categories (comma-separated); --log all (or --verbose) enables every category\n")
	b.WriteString("  --verbose          Enable all log categories (shorthand for --log all)\n")
	b.WriteString("  --help             Show this help (one-level); add --recursive to expand all subcommands\n")
	b.WriteString("  --recursive        With --help, expand the full command tree; --output picks the format\n\n")
	b.WriteString("Commands:\n")
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "  %-8s %s\n", sc.name, sc.about)
	}
	return b.String()
}

// formatCompleteHelp returns recursive help for the root command and all subcommands.
func formatCompleteHelp() string {
	var b strings.Builder
	b.WriteString(formatRootHelp())
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "\n%s\n%s\n\n", strings.Repeat("=", 60), "agent-cli "+sc.name)
		fmt.Fprintf(&b, "%s\n%s\n\nFlags:\n%s\n", strings.Repeat("=", 60), sc.about, sc.flags)
	}
	return b.String()
}

// formatSubcommandHelp returns help for a single subcommand. When the
// subcommand is the help target (withGlobals), it also documents the global
// --output formats so even a leaf `--help` advertises the format options.
// Descendants in a recursive dump pass withGlobals=false: the root already
// documented the modifiers once, so repeating them per command is pure noise.
func formatSubcommandHelp(name string, withGlobals bool) string {
	for _, sc := range subcommands {
		if sc.name == name {
			var b strings.Builder
			fmt.Fprintf(&b, "agent-cli %s — %s\n\nFlags:\n%s\n", sc.name, sc.about, sc.flags)
			if withGlobals {
				b.WriteString("\nGlobal options:\n")
				b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n")
			}
			return b.String()
		}
	}
	return ""
}

func formatMarkdownHelp(command string, recursive bool) string {
	var b strings.Builder
	if command != "" {
		for _, sc := range subcommands {
			if sc.name == command {
				fmt.Fprintf(&b, "# agent-cli %s - %s\n\n", sc.name, sc.about)
				b.WriteString("```text\n")
				b.WriteString(formatSubcommandHelp(command, true))
				b.WriteString("```\n")
				return b.String()
			}
		}
	}
	b.WriteString("# agent-cli - Minimal agent-first CLI example\n\n")
	b.WriteString("```text\n")
	b.WriteString(formatRootHelp())
	b.WriteString("```\n")
	if !recursive {
		return b.String()
	}
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "\n## agent-cli %s - %s\n\n", sc.name, sc.about)
		b.WriteString("```text\n")
		b.WriteString(formatSubcommandHelp(sc.name, false))
		b.WriteString("```\n")
	}
	return b.String()
}

// globalHelpOptions documents the global flags so a structured (json/yaml) help
// dump advertises the help surface — the scope modifier and the output formats —
// just like the plain and markdown formats do. Only the target command carries
// it (descendants omit it) to keep a recursive dump lean. A leaf target omits
// --recursive, which has nothing to expand.
func globalHelpOptions(includeRecursive bool) []map[string]any {
	opts := []map[string]any{
		{"name": "--output", "help": "Output format: json, yaml, plain (default: json); help also accepts markdown"},
		{"name": "--log", "help": "Log categories (comma-separated); --log all (or --verbose) enables every category"},
		{"name": "--verbose", "help": "Enable all log categories (shorthand for --log all)"},
	}
	if includeRecursive {
		opts = append(opts, map[string]any{"name": "--recursive", "help": "With --help, expand the full command tree (a bare --recursive is ignored)"})
	}
	opts = append(opts, map[string]any{"name": "--help", "help": "Show this help (one-level)"})
	return opts
}

func helpSchema(command, scope string) map[string]any {
	commandPath := "agent-cli"
	if command != "" {
		commandPath += " " + command
		for _, sc := range subcommands {
			if sc.name == command {
				return map[string]any{
					"code":         "help",
					"scope":        scope,
					"command_path": commandPath,
					"name":         sc.name,
					"about":        sc.about,
					"flags":        sc.flags,
					"options":      globalHelpOptions(false),
				}
			}
		}
	}

	commands := make([]map[string]any, 0, len(subcommands))
	for _, sc := range subcommands {
		entry := map[string]any{"name": sc.name, "about": sc.about}
		if scope == "recursive" {
			entry["flags"] = sc.flags
		}
		commands = append(commands, entry)
	}
	return map[string]any{
		"code":         "help",
		"scope":        scope,
		"command_path": commandPath,
		"name":         "agent-cli",
		"about":        "Minimal agent-first CLI example",
		"options":      globalHelpOptions(true),
		"commands":     commands,
	}
}

func printHelp(command, output string, outputExplicit bool, outputMissing bool, recursive bool) int {
	if outputMissing {
		fmt.Println(afdata.OutputJson(afdata.BuildCliError("missing value for --output: expected plain, json, yaml, or markdown", "valid help output formats: plain, markdown, json, yaml")))
		return 2
	}
	// Scope (one-level vs recursive) is set by --recursive; --output only picks
	// the format. A specific subcommand is leaf-level here, so its scope is the
	// same either way.
	scope := "one_level"
	if recursive {
		scope = "recursive"
	}
	if !outputExplicit || output == "plain" {
		switch {
		case command != "":
			fmt.Print(formatSubcommandHelp(command, true))
		case recursive:
			fmt.Print(formatCompleteHelp())
		default:
			fmt.Print(formatRootHelp())
		}
		return 0
	}
	if output == "markdown" {
		fmt.Print(formatMarkdownHelp(command, recursive))
		return 0
	}
	format, err := afdata.CliParseOutput(output)
	if err != nil {
		fmt.Println(afdata.OutputJson(afdata.BuildCliError(err.Error(), "")))
		return 2
	}
	fmt.Println(afdata.CliOutput(helpSchema(command, scope), format))
	return 0
}

// logEnabled reports whether a diagnostic category should be emitted. `all` /
// `*` (what --verbose expands to) enable every category, including unnamed ones.
func logEnabled(filters []string, category string) bool {
	for _, f := range filters {
		if f == category || f == "all" || f == "*" {
			return true
		}
	}
	return false
}

func buildRequestLog(command string) map[string]any {
	if command == "" {
		command = "none"
	}
	return afdata.BuildJson("log", map[string]any{
		"category": "request",
		"command":  command,
	}, nil)
}

func buildStartupLog() map[string]any {
	return afdata.BuildJson("log", map[string]any{
		"category": "startup",
		"event":    "startup",
	}, nil)
}

func main() {
	output := "json"
	outputExplicit := false
	outputMissing := false
	dryRun := false
	logArg := ""
	host := ""
	agent := "all"
	scope := "personal"
	skillsDir := ""
	force := false
	showHelp := false
	recursive := false
	verbose := false
	var positionals []string

	args := os.Args[1:]
	if out, handled, err := afdata.CliHandleVersionOrContinue(args, "agent-cli", agentCliVersion, ""); handled {
		if err != nil {
			fmt.Println(afdata.OutputJson(afdata.BuildCliError(err.Error(), "valid version output formats: json, yaml, plain")))
			os.Exit(2)
		}
		fmt.Print(out)
		return
	}

	for i := 0; i < len(args); i++ {
		if strings.HasPrefix(args[i], "--output=") {
			output = strings.TrimPrefix(args[i], "--output=")
			outputExplicit = true
			continue
		}
		switch args[i] {
		case "--help", "-h":
			showHelp = true
		case "--recursive":
			// A help modifier only: it selects recursive scope when --help is
			// present and is otherwise ignored, so it never affects normal
			// command parsing.
			recursive = true
		case "--output":
			outputExplicit = true
			if i+1 < len(args) && !strings.HasPrefix(args[i+1], "-") {
				i++
				output = args[i]
			} else {
				outputMissing = true
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
		case "--verbose":
			verbose = true
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

	// --help is one-level plain; --recursive expands the tree and --output picks
	// the format. A bare --recursive (no --help) falls through to normal parsing.
	if showHelp {
		os.Exit(printHelp(command, output, outputExplicit, outputMissing, recursive))
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
	if verbose {
		// --verbose is shorthand for --log all.
		filters = append(filters, "all")
	}

	// Each diagnostic line self-tags with its `category`, so `--log all` reveals
	// the full set from real output rather than a static help list.
	if logEnabled(filters, "request") {
		fmt.Println(afdata.CliOutput(buildRequestLog(command), format))
	}
	if logEnabled(filters, "startup") {
		fmt.Println(afdata.CliOutput(buildStartupLog(), format))
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
