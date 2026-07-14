// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: human --help (one-level) plus orthogonal --recursive scope and
// --output json|yaml|markdown format for full surface export,
// CliParseOutput, CliParseLogFilters, CliOutput, BuildCliError,
// --dry-run, error hints, and a `skill` subcommand that installs/uninstalls/
// reports status of an embedded Agent Skill across Codex, Claude Code, opencode, and Hermes.
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
//	go run ./examples/agent_cli --stdout-file /tmp/agent-cli.out --stderr-file /tmp/agent-cli.err ping
//	go run ./examples/agent_cli ping
//	go run ./examples/agent_cli skill status --agent opencode --skills-dir /tmp/ex
//	go run ./examples/agent_cli skill install --agent opencode --skills-dir /tmp/ex
package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"strings"

	afdata "github.com/agentfirstkit/agent-first-data/go"
	"github.com/agentfirstkit/agent-first-data/go/skill"
	"github.com/agentfirstkit/agent-first-data/go/streamredirect"
)

// A fictional spore's embedded Agent Skill, used by the `skill` subcommand to
// demonstrate skill.RunSkillAdmin.
const widgetSkill = "---\nname: agent-first-widget\ndescription: Example skill bundled by the agent-cli demo.\n---\n\n# Agent-First Widget\n\nExample behavior rules go here.\n"

var widgetSpec = skill.SkillSpec{
	Name:       "agent-first-widget",
	Source:     widgetSkill,
	Title:      "Agent-First Widget",
	MarkerSlug: "afwidget",
}

const agentCliVersion = "0.13.0"
const helpDefaultAPIKeySecret = "sk-help-default"
const pingHostEnv = "PING_HOST"

type subcommand struct {
	name  string
	about string
	flags string
}

var subcommands = []subcommand{
	{name: "echo", about: "Echo back the input as structured output", flags: "  --dry-run    Preview without executing"},
	{name: "ping", about: "Ping a remote target", flags: "  --host       Target host to ping"},
	{name: "cancel", about: "Return a tool-defined cancellation error", flags: "  (no flags)"},
	{name: "skill", about: "Manage this tool's embedded Agent Skill", flags: "  status|install|uninstall  Skill action\n  --agent      all, codex, claude-code, opencode, hermes (default: all)\n  --scope      personal, workspace (default: personal)\n  --skills-dir Skills directory (requires a single concrete --agent)\n  --force      Overwrite or remove a skill this tool did not manage"},
}

// formatRootHelp returns one-level help for the root command. Markdown
// rendering passes withTitle=false: the `# agent-cli - <about>` heading already
// carries the summary, so repeating it as the first line of the fenced block is
// duplication.
func formatRootHelp(withTitle bool) string {
	var b strings.Builder
	if withTitle {
		b.WriteString("agent-cli — Minimal agent-first CLI example\n\n")
	}
	b.WriteString("Usage: agent-cli [OPTIONS] <COMMAND>\n\n")
	b.WriteString("Options:\n")
	b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n")
	b.WriteString("  --json             Equivalent to --output json\n")
	b.WriteString("  --log <FILTERS>    Log categories (comma-separated); --log all (or --verbose) enables every category\n")
	b.WriteString("  --verbose          Enable all log categories (shorthand for --log all)\n")
	fmt.Fprintf(&b, "  --api-key-secret <VALUE> API key used by examples (default: %s)\n", redactHelpDefault("--api-key-secret", helpDefaultAPIKeySecret))
	b.WriteString("  --stdout-file <PATH> Redirect stdout to a file\n")
	b.WriteString("  --stderr-file <PATH> Redirect stderr to a file\n")
	b.WriteString("  --help             Show this help (one-level); add --recursive to expand all subcommands\n")
	b.WriteString("  --recursive        With --help, expand the full command tree; --output picks the format\n\n")
	b.WriteString("Commands:\n")
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "  %-8s %s\n", sc.name, sc.about)
	}
	fmt.Fprintf(&b, "\nAFDATA: %s\n", afdata.Version)
	return b.String()
}

// formatCompleteHelp returns recursive help for the root command and all subcommands.
func formatCompleteHelp() string {
	var b strings.Builder
	b.WriteString(formatRootHelp(true))
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
func formatSubcommandHelp(name string, withGlobals, withTitle bool) string {
	for _, sc := range subcommands {
		if sc.name == name {
			var b strings.Builder
			// Markdown rendering passes withTitle=false: the heading already
			// shows the `agent-cli <name> - <about>` summary, so the fenced
			// block skips it.
			if withTitle {
				fmt.Fprintf(&b, "agent-cli %s — %s\n\n", sc.name, sc.about)
			}
			fmt.Fprintf(&b, "Flags:\n%s\n", sc.flags)
			if withGlobals {
				b.WriteString("\nGlobal options:\n")
				b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n")
				b.WriteString("  --json             Equivalent to --output json\n")
			}
			if withGlobals || withTitle {
				fmt.Fprintf(&b, "\nAFDATA: %s\n", afdata.Version)
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
				b.WriteString(formatSubcommandHelp(command, true, false))
				b.WriteString("```\n")
				return b.String()
			}
		}
	}
	b.WriteString("# agent-cli - Minimal agent-first CLI example\n\n")
	b.WriteString("```text\n")
	b.WriteString(formatRootHelp(false))
	b.WriteString("```\n")
	if !recursive {
		return b.String()
	}
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "\n## agent-cli %s - %s\n\n", sc.name, sc.about)
		b.WriteString("```text\n")
		b.WriteString(formatSubcommandHelp(sc.name, false, false))
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
		{"name": "--json", "help": "Equivalent to --output json"},
		{"name": "--log", "help": "Log categories (comma-separated); --log all (or --verbose) enables every category"},
		{"name": "--verbose", "help": "Enable all log categories (shorthand for --log all)"},
		{"name": "--api-key-secret", "help": "API key used by examples", "default_values": []string{redactHelpDefault("--api-key-secret", helpDefaultAPIKeySecret)}},
		{"name": "--stdout-file", "help": "Redirect stdout to a file"},
		{"name": "--stderr-file", "help": "Redirect stderr to a file"},
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
					"versions":     map[string]any{"afdata": afdata.Version},
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
		"versions":     map[string]any{"afdata": afdata.Version},
		"command_path": commandPath,
		"name":         "agent-cli",
		"about":        "Minimal agent-first CLI example",
		"options":      globalHelpOptions(true),
		"commands":     commands,
	}
}

func printHelp(command, output string, outputExplicit bool, outputMissing bool, recursive bool) int {
	if outputMissing {
		event, _ := afdata.BuildCLIError("missing value for --output: expected plain, json, yaml, or markdown", "valid help output formats: plain, markdown, json, yaml")
		fmt.Println(afdata.OutputJson(event.Value()))
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
			fmt.Print(formatSubcommandHelp(command, true, true))
		case recursive:
			fmt.Print(formatCompleteHelp())
		default:
			fmt.Print(formatRootHelp(true))
		}
		return 0
	}
	if output == "markdown" {
		fmt.Print(formatMarkdownHelp(command, recursive))
		return 0
	}
	format, err := afdata.CliParseOutput(output)
	if err != nil {
		event, _ := afdata.BuildCLIError(err.Error(), "")
		fmt.Println(afdata.OutputJson(event.Value()))
		return 2
	}
	fmt.Println(afdata.CliOutput(helpSchema(command, scope), format))
	return 0
}

func redactHelpDefault(name, value string) string {
	normalized := strings.ReplaceAll(strings.TrimLeft(name, "-"), "-", "_")
	if strings.HasSuffix(normalized, "_secret") || strings.HasSuffix(normalized, "_SECRET") {
		return "***"
	}
	return value
}

// logEnabled reports whether a diagnostic category should be emitted.
func logEnabled(filters afdata.LogFilters, category string) bool {
	return filters.Enabled(category)
}

func buildRequestLog(command string) map[string]any {
	if command == "" {
		command = "none"
	}
	builder := afdata.NewJSONLog(map[string]any{
		"level":    "info",
		"message":  "event",
		"category": "request",
		"command":  command,
	})
	event, _ := builder.Build()
	return event.Value()
}

func buildStartupLog(args []string, command string, output string, filters afdata.LogFilters, verbose bool) map[string]any {
	if command == "" {
		command = "none"
	}
	builder := afdata.NewJSONLog(map[string]any{
		"level":    "info",
		"message":  "event",
		"category": "startup",
		"event":    "startup",
		"argv":     afdata.RedactedValue(args),
		"parsed": map[string]any{
			"command": command,
			"output":  output,
			"log":     filters.Values(),
			"verbose": verbose,
		},
		"effective_config": map[string]any{
			"output": output,
			"log":    filters.Values(),
		},
		"env": startupEnvSnapshot(),
	})
	event, _ := builder.Build()
	return event.Value()
}

func startupEnvSnapshot() []map[string]any {
	item := map[string]any{"key": pingHostEnv}
	value, ok := os.LookupEnv(pingHostEnv)
	item["present"] = ok
	if ok {
		item["value"] = value
	}
	return []map[string]any{item}
}

func containsArg(args []string, want string) bool {
	for _, arg := range args {
		if arg == want {
			return true
		}
	}
	return false
}

func outputFlagMissing(args []string) bool {
	for i, arg := range args {
		if arg == "--output" {
			return i+1 >= len(args) || strings.HasPrefix(args[i+1], "-")
		}
		if strings.HasPrefix(arg, "--output=") {
			return strings.TrimPrefix(arg, "--output=") == ""
		}
	}
	return false
}

func validateStrictArgs(args []string) (string, string) {
	root := flag.NewFlagSet("agent-cli", flag.ContinueOnError)
	root.SetOutput(io.Discard)
	root.Bool("help", false, "")
	root.Bool("h", false, "")
	root.Bool("recursive", false, "")
	root.String("output", "json", "")
	root.Bool("json", false, "")
	root.String("log", "", "")
	root.Bool("verbose", false, "")
	root.String("api-key-secret", helpDefaultAPIKeySecret, "")
	root.String("stdout-file", "", "")
	root.String("stderr-file", "", "")
	if err := root.Parse(args); err != nil {
		return err.Error(), "try: agent-cli --help"
	}
	positionals := root.Args()
	if len(positionals) == 0 {
		return "", ""
	}
	command := positionals[0]
	rest := positionals[1:]
	switch command {
	case "echo":
		fs := flag.NewFlagSet("agent-cli echo", flag.ContinueOnError)
		fs.SetOutput(io.Discard)
		fs.Bool("help", false, "")
		fs.Bool("h", false, "")
		fs.Bool("dry-run", false, "")
		if err := fs.Parse(rest); err != nil {
			return err.Error(), "try: agent-cli echo --help"
		}
		if fs.NArg() > 0 {
			return "unexpected positional argument: " + fs.Arg(0), "usage: agent-cli echo [--dry-run]"
		}
	case "ping":
		fs := flag.NewFlagSet("agent-cli ping", flag.ContinueOnError)
		fs.SetOutput(io.Discard)
		fs.Bool("help", false, "")
		fs.Bool("h", false, "")
		fs.String("host", "", "")
		if err := fs.Parse(rest); err != nil {
			return err.Error(), "try: agent-cli ping --help"
		}
		if fs.NArg() > 0 {
			return "unexpected positional argument: " + fs.Arg(0), "usage: agent-cli ping [--host HOST]"
		}
	case "cancel":
		fs := flag.NewFlagSet("agent-cli cancel", flag.ContinueOnError)
		fs.SetOutput(io.Discard)
		fs.Bool("help", false, "")
		fs.Bool("h", false, "")
		if err := fs.Parse(rest); err != nil {
			return err.Error(), "try: agent-cli cancel --help"
		}
		if fs.NArg() > 0 {
			return "unexpected positional argument: " + fs.Arg(0), "usage: agent-cli cancel"
		}
	case "skill":
		fs := flag.NewFlagSet("agent-cli skill", flag.ContinueOnError)
		fs.SetOutput(io.Discard)
		fs.Bool("help", false, "")
		fs.Bool("h", false, "")
		fs.String("agent", "all", "")
		fs.String("scope", "personal", "")
		fs.String("skills-dir", "", "")
		fs.Bool("force", false, "")
		verb := ""
		flagArgs := make([]string, 0, len(rest))
		for _, arg := range rest {
			if verb == "" && !strings.HasPrefix(arg, "-") {
				verb = arg
				continue
			}
			flagArgs = append(flagArgs, arg)
		}
		if err := fs.Parse(flagArgs); err != nil {
			return err.Error(), "try: agent-cli skill --help"
		}
		if verb == "" {
			return "skill requires a subcommand: status, install, uninstall", "example: agent-cli skill status --agent opencode"
		}
		if fs.NArg() > 0 {
			return "unexpected positional argument: " + fs.Arg(0), "usage: agent-cli skill status|install|uninstall [OPTIONS]"
		}
	case "config", "service":
		return "command not implemented in Go example: " + command, "valid commands: echo, ping, cancel, skill"
	default:
		return "unknown command: " + command, "valid commands: echo, ping, cancel, skill"
	}
	return "", ""
}

func main() {
	output := "json"
	outputExplicit := false
	outputConflict := ""
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
	outputMissing := outputFlagMissing(args)
	if _, err := streamredirect.InstallStreamRedirectFromArgs(args); err != nil {
		event, _ := afdata.BuildCLIError(err.Error(), "")
		fmt.Println(afdata.OutputJson(event.Value()))
		os.Exit(2)
	}

	if out, handled, err := afdata.CliHandleVersionOrContinue(args, "agent-cli", agentCliVersion, ""); handled {
		if err != nil {
			event, _ := afdata.BuildCLIError(err.Error(), "valid version output formats: json, yaml, plain")
			fmt.Println(afdata.OutputJson(event.Value()))
			os.Exit(2)
		}
		fmt.Print(out)
		return
	}

	for i := 0; i < len(args); i++ {
		if strings.HasPrefix(args[i], "--output=") {
			output = strings.TrimPrefix(args[i], "--output=")
			outputExplicit = true
			if output == "" {
				outputMissing = true
			}
			if output != "json" && containsArg(args[:i], "--json") {
				outputConflict = "conflicting output formats: --output " + output + " conflicts with --json"
			}
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
				if output != "json" && containsArg(args[:i], "--json") {
					outputConflict = "conflicting output formats: --output " + output + " conflicts with --json"
				}
			} else {
				outputMissing = true
			}
		case "--json":
			if outputExplicit && output != "json" {
				outputConflict = "conflicting output formats: --json conflicts with --output " + output
			}
			output = "json"
			outputExplicit = true
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
		case "--api-key-secret", "--stdout-file", "--stderr-file":
			i++
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
		if outputConflict != "" {
			event, _ := afdata.BuildCLIError(outputConflict, "valid output formats: json, yaml, plain")
			fmt.Println(afdata.OutputJson(event.Value()))
			os.Exit(2)
		}
		os.Exit(printHelp(command, output, outputExplicit, outputMissing, recursive))
	}

	// 1. Parse --output flag with structured error on failure.
	if outputMissing {
		event, _ := afdata.BuildCLIError("missing value for --output: expected json, yaml, or plain", "valid output formats: json, yaml, plain")
		fmt.Println(afdata.OutputJson(event.Value()))
		os.Exit(2)
	}
	if outputConflict != "" {
		event, _ := afdata.BuildCLIError(outputConflict, "valid output formats: json, yaml, plain")
		fmt.Println(afdata.OutputJson(event.Value()))
		os.Exit(2)
	}
	format, err := afdata.CliParseOutput(output)
	if err != nil {
		event, _ := afdata.BuildCLIError(err.Error(), "")
		fmt.Println(afdata.OutputJson(event.Value()))
		os.Exit(2)
	}
	if message, hint := validateStrictArgs(args); message != "" {
		event, _ := afdata.BuildCLIError(message, hint)
		fmt.Println(afdata.CliOutput(event.Value(), format))
		os.Exit(2)
	}

	// 2. Normalize --log filters: trim, lowercase, deduplicate.
	var filters afdata.LogFilters
	if logArg != "" {
		filters = afdata.CliParseLogFilters(strings.Split(logArg, ","))
	}
	if verbose {
		// --verbose is shorthand for --log all.
		// Create new filters with "all" added
		allValues := filters.Values()
		allValues = append(allValues, "all")
		filters = afdata.CliParseLogFilters(allValues)
	}

	// Each diagnostic line self-tags with its `category`, so `--log all` reveals
	// the full set from real output rather than a static help list.
	if logEnabled(filters, "request") {
		fmt.Println(afdata.CliOutput(buildRequestLog(command), format))
	}
	if logEnabled(filters, "startup") {
		fmt.Println(afdata.CliOutput(buildStartupLog(args, command, output, filters, verbose), format))
	}

	// 3. No subcommand → error with hint.
	if command == "" {
		event, _ := afdata.BuildCLIError("no subcommand provided", "try: agent-cli --help")
		fmt.Println(afdata.CliOutput(event.Value(), format))
		os.Exit(2)
	}

	switch command {
	case "echo":
		// 4. --dry-run → preview without executing.
		if dryRun {
			builder := afdata.NewJSONResult(map[string]any{
				"action": "echo",
				"log":    filters,
			}).Trace(map[string]any{"duration_ms": 0})
			preview, _ := builder.Build()
			fmt.Println(afdata.CliOutput(preview.Value(), format))
			return
		}

		resultBuilder := afdata.NewJSONResult(map[string]any{
			"action": "echo",
			"log":    filters,
		})
		result, _ := resultBuilder.Build()
		fmt.Println(afdata.CliOutput(result.Value(), format))

	case "ping":
		// 5. Demonstrate a protocol v1 error with hint on failure.
		if host == "" {
			host = os.Getenv(pingHostEnv)
		}
		if host == "" {
			errBuilder := afdata.NewJSONError("ping_target_not_configured", "ping target not configured").
				Hint("set PING_HOST or pass --host").
				Trace(map[string]any{"duration_ms": 0})
			errVal, _ := errBuilder.Build()
			fmt.Println(afdata.CliOutput(errVal.Value(), format))
			os.Exit(1)
		}

	case "cancel":
		errBuilder := afdata.NewJSONError("cancelled", "operation cancelled").
			Hint("the operation was cancelled before completion").
			Trace(map[string]any{"duration_ms": 0})
		errVal, _ := errBuilder.Build()
		fmt.Println(afdata.CliOutput(errVal.Value(), format))
		os.Exit(1)

	case "skill":
		os.Exit(runSkill(positionals, agent, scope, skillsDir, force, format))

	default:
		hint := "valid commands: echo, ping, cancel, skill"
		event, _ := afdata.BuildCLIError("unknown command: "+command, hint)
		fmt.Println(afdata.CliOutput(event.Value(), format))
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
	var action skill.SkillAction
	switch verb {
	case "status":
		action = skill.SkillActionStatus
	case "install":
		action = skill.SkillActionInstall
	case "uninstall":
		action = skill.SkillActionUninstall
	default:
		event, _ := afdata.BuildCLIError("skill requires a subcommand: status, install, uninstall", "example: agent-cli skill status --agent opencode")
		fmt.Println(afdata.CliOutput(event.Value(), format))
		return 2
	}

	opts, message, hint := buildSkillOptions(agentStr, scopeStr, skillsDir, force)
	if message != "" {
		event, _ := afdata.BuildCLIError(message, hint)
		fmt.Println(afdata.CliOutput(event.Value(), format))
		return 2
	}

	report, serr := skill.RunSkillAdmin(widgetSpec, action, opts)
	if serr != nil {
		event, _ := afdata.BuildCLIError(serr.Message, serr.Hint)
		fmt.Println(afdata.CliOutput(event.Value(), format))
		return 1
	}
	fmt.Println(afdata.CliOutput(report, format))
	return 0
}

// buildSkillOptions parses the --agent/--scope string flags into the library enums.
// Returns a non-empty message (and optional hint) on an unknown value.
func buildSkillOptions(agentStr, scopeStr, skillsDir string, force bool) (skill.SkillOptions, string, string) {
	var agent skill.SkillAgentSelection
	switch agentStr {
	case "all":
		agent = skill.SkillAgentAll
	case "codex":
		agent = skill.SkillAgentCodex
	case "claude-code":
		agent = skill.SkillAgentClaudeCode
	case "opencode":
		agent = skill.SkillAgentOpencode
	case "hermes":
		agent = skill.SkillAgentHermes
	default:
		return skill.SkillOptions{}, fmt.Sprintf("invalid --agent '%s'", agentStr), "valid values: all, codex, claude-code, opencode, hermes"
	}
	var sc skill.SkillScope
	switch scopeStr {
	case "personal":
		sc = skill.SkillScopePersonal
	case "workspace":
		sc = skill.SkillScopeWorkspace
	default:
		return skill.SkillOptions{}, fmt.Sprintf("invalid --scope '%s'", scopeStr), "valid values: personal, workspace"
	}
	return skill.SkillOptions{Agent: agent, Scope: sc, SkillsDir: skillsDir, Force: force}, "", ""
}
