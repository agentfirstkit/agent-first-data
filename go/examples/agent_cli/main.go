// Command agent_cli demonstrates canonical CLI helper usage for agent tools.
//
// Demonstrates: output-aware --help (one-level) plus orthogonal --recursive
// scope and --output plain|json|yaml|markdown format for full surface export,
// CliParseOutput, CliParseLogFilters, Render, BuildCliError,
// --dry-run, and error hints.
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
package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"strings"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

const agentCliVersion = "0.13.0"
const helpDefaultAPIKeySecret = "sk-help-default"
const pingHostEnv = "PING_HOST"

type subcommand struct {
	name      string
	about     string
	usage     string
	arguments []helpArgument
}

type helpArgument struct {
	name       string
	short      string
	help       string
	value      string
	defaultVal string
	required   bool
	repeatable bool
}

var subcommands = []subcommand{
	{
		name:  "echo",
		about: "Echo back the input as structured output",
		usage: "[OPTIONS]",
		arguments: []helpArgument{
			{name: "--dry-run", help: "Preview without executing"},
		},
	},
	{
		name:  "ping",
		about: "Ping a remote target",
		usage: "[OPTIONS]",
		arguments: []helpArgument{
			{name: "--host", value: "HOST", help: "Target host to ping"},
		},
	},
	{name: "cancel", about: "Return a tool-defined cancellation error"},
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
	b.WriteString("  --version          Print version\n")
	b.WriteString("  --help             Show this help (one-level); add --recursive to expand all subcommands\n")
	b.WriteString("  --recursive        With --help, expand the full command tree; --output picks the format\n\n")
	b.WriteString("Commands:\n")
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "  %-8s %s\n", sc.name, sc.about)
	}
	b.WriteString("\n")
	return b.String()
}

// formatCompleteHelp returns recursive help for the root command and all subcommands.
func formatCompleteHelp() string {
	var b strings.Builder
	b.WriteString(formatRootHelp(true))
	for _, sc := range subcommands {
		fmt.Fprintf(&b, "\n%s\n%s\n\n", strings.Repeat("=", 60), "agent-cli "+sc.name)
		fmt.Fprintf(&b, "%s\n%s\n\nFlags:\n%s", strings.Repeat("=", 60), sc.about, formatPlainArguments(sc.arguments))
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
			fmt.Fprintf(&b, "Flags:\n%s", formatPlainArguments(sc.arguments))
			if withGlobals {
				b.WriteString("\nGlobal options:\n")
				b.WriteString("  --output <FORMAT>  Output format: json, yaml, plain (default: json); help also accepts markdown\n")
				b.WriteString("  --json             Equivalent to --output json\n")
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

func formatPlainArguments(arguments []helpArgument) string {
	if len(arguments) == 0 {
		return "  (no flags)\n"
	}
	var b strings.Builder
	for _, argument := range arguments {
		fmt.Fprintf(&b, "  %s", argument.name)
		if argument.value != "" {
			fmt.Fprintf(&b, " <%s>", argument.value)
		}
		fmt.Fprintf(&b, "    %s\n", argument.help)
	}
	return b.String()
}

func (argument helpArgument) schema() map[string]any {
	out := map[string]any{"name": argument.name}
	if argument.short != "" {
		out["short"] = argument.short
	}
	if argument.help != "" {
		out["help"] = argument.help
	}
	if argument.value != "" {
		out["value"] = argument.value
	}
	if argument.defaultVal != "" {
		out["default"] = argument.defaultVal
	}
	if argument.required {
		out["required"] = true
	}
	if argument.repeatable {
		out["repeatable"] = true
	}
	return out
}

func argumentSchemas(arguments []helpArgument) []map[string]any {
	out := make([]map[string]any, 0, len(arguments))
	for _, argument := range arguments {
		out = append(out, argument.schema())
	}
	return out
}

func rootHelpArguments() []helpArgument {
	return []helpArgument{
		{name: "--output", value: "FORMAT", defaultVal: "json", help: "Output format: json, yaml, plain; help also accepts markdown"},
		{name: "--json", help: "Equivalent to --output json"},
		{name: "--log", value: "FILTERS", help: "Log categories (comma-separated); --log all (or --verbose) enables every category"},
		{name: "--verbose", help: "Enable all log categories (shorthand for --log all)"},
		{name: "--api-key-secret", value: "VALUE", defaultVal: redactHelpDefault("--api-key-secret", helpDefaultAPIKeySecret), help: "API key used by examples"},
		{name: "--version", help: "Print version"},
		{name: "--help", help: "Show one-level help; add --recursive for the full command tree and --output plain|json|yaml|markdown to choose the format"},
		{name: "--recursive", help: "With --help, expand the full command tree"},
	}
}

func targetHelpArguments(sc subcommand) []helpArgument {
	arguments := append([]helpArgument{}, sc.arguments...)
	return append(arguments, helpArgument{
		name: "--help",
		help: "Show help; add --output plain|json|yaml|markdown to choose the format",
	})
}

func helpSchema(command, scope string) map[string]any {
	commandPath := "agent-cli"
	if command != "" {
		commandPath += " " + command
		for _, sc := range subcommands {
			if sc.name == command {
				schema := map[string]any{
					"scope":        scope,
					"command_path": commandPath,
					"name":         sc.name,
					"about":        sc.about,
					"arguments":    argumentSchemas(targetHelpArguments(sc)),
				}
				if sc.usage != "" {
					schema["usage"] = sc.usage
				}
				return schema
			}
		}
	}

	commands := make([]map[string]any, 0, len(subcommands))
	for _, sc := range subcommands {
		entry := map[string]any{"name": sc.name, "about": sc.about}
		if scope == "recursive" {
			if sc.usage != "" {
				entry["usage"] = sc.usage
			}
			if len(sc.arguments) > 0 {
				entry["arguments"] = argumentSchemas(sc.arguments)
			}
		}
		commands = append(commands, entry)
	}
	return map[string]any{
		"scope":        scope,
		"command_path": commandPath,
		"name":         "agent-cli",
		"about":        "Minimal agent-first CLI example",
		"usage":        "[OPTIONS] <COMMAND>",
		"arguments":    argumentSchemas(rootHelpArguments()),
		"subcommands":  commands,
	}
}

// bootstrapEmitter builds a finite emitter for errors raised before the main
// emitter exists (before --output is parsed): result → stdout, diagnostics and
// errors → stderr, routed by kind.
func bootstrapEmitter(format afdata.OutputFormat) *afdata.CliEmitter {
	return afdata.NewCliEmitterFinite(os.Stdout, os.Stderr, format)
}

// finishCliError builds a standard cli_error envelope with the shared error
// builder and finishes it on emitter, returning the process exit code. In finite
// mode the error routes to stderr; Finish collapses a broken pipe to a clean
// exit and any other write failure to code 4.
func finishCliError(emitter *afdata.CliEmitter, message, hint string, exitCode int) int {
	event, _ := afdata.BuildCLIError(message, hint)
	return emitter.Finish(event, exitCode)
}

func printHelp(command, output string, outputMissing bool, recursive bool) int {
	if outputMissing {
		return finishCliError(bootstrapEmitter(afdata.OutputFormatJson), "missing value for --output: expected plain, json, yaml, or markdown", "valid help output formats: plain, markdown, json, yaml", 2)
	}
	// Scope (one-level vs recursive) is set by --recursive; --output only picks
	// the format. A specific subcommand is leaf-level here, so its scope is the
	// same either way.
	scope := "one_level"
	if recursive {
		scope = "recursive"
	}
	if output == "plain" {
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
		return finishCliError(bootstrapEmitter(afdata.OutputFormatJson), err.Error(), "", 2)
	}
	event := afdata.NewJSONResult(map[string]any{
		"code": "help",
		"help": helpSchema(command, scope),
	}).Trace(map[string]any{}).Build()
	fmt.Println(afdata.Render(event.Value(), format, afdata.OutputOptions{}))
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
		"message":  "request",
		"category": "request",
		"command":  command,
	}).Trace(map[string]any{})
	event := builder.Build()
	return event.Value()
}

func buildStartupLog(args []string, command string, output string, filters afdata.LogFilters, verbose bool) map[string]any {
	if command == "" {
		command = "none"
	}
	builder := afdata.NewJSONLog(map[string]any{
		"level":    "info",
		"message":  "startup",
		"category": "startup",
		"event":    "startup",
		"argv":     redactArgv(args),
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
	}).Trace(map[string]any{})
	event := builder.Build()
	return event.Value()
}

// redactArgv redacts argv values whose long flag names are secret by AFDATA
// naming, covering both --name-secret=value and --name-secret value.
//
// RedactedValue is keyed by field name and cannot help here: a []string carries
// no keys, so it would pass argv through untouched.
func redactArgv(args []string) []string {
	out := make([]string, 0, len(args))
	redactNext := false
	for _, arg := range args {
		if redactNext {
			redactNext = false
			if !strings.HasPrefix(arg, "-") {
				out = append(out, "***")
				continue
			}
		}
		if strings.HasPrefix(arg, "--") {
			rest := strings.TrimPrefix(arg, "--")
			if name, _, found := strings.Cut(rest, "="); found {
				if isSecretFlagName(name) {
					out = append(out, "--"+name+"=***")
					continue
				}
			} else if isSecretFlagName(rest) {
				redactNext = true
			}
		}
		out = append(out, arg)
	}
	return out
}

func isSecretFlagName(name string) bool {
	normalized := strings.ReplaceAll(name, "-", "_")
	return strings.HasSuffix(normalized, "_secret") || strings.HasSuffix(normalized, "_SECRET")
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
	case "config", "service":
		return "command not implemented in Go example: " + command, "valid commands: echo, ping, cancel"
	default:
		return "unknown command: " + command, "valid commands: echo, ping, cancel"
	}
	return "", ""
}

func main() {
	// Make a write to a broken stdout/stderr pipe return an error the emitter
	// surfaces (so Finish can map it to a clean exit), instead of the default
	// SIGPIPE termination.
	installSigpipeHandler()

	output := "json"
	outputExplicit := false
	outputConflict := ""
	dryRun := false
	logArg := ""
	host := ""
	showHelp := false
	recursive := false
	verbose := false
	var positionals []string

	args := os.Args[1:]
	outputMissing := outputFlagMissing(args)

	// The example's own value-taking global long flags: the pre-parser consumes
	// each one's space value so it is never mistaken for the subcommand boundary
	// (afdata's own --output/--output-to are recognized without being listed).
	versionValueFlags := []string{"--log", "--host", "--api-key-secret"}
	if out, handled, err := afdata.CliHandleVersionOrContinue(args, versionValueFlags, "agent-cli", "Agent CLI Example", agentCliVersion, "", afdata.OutputFormatJson); handled {
		if err != nil {
			os.Exit(finishCliError(bootstrapEmitter(afdata.OutputFormatJson), err.Error(), "valid version output formats: json, yaml, plain", 2))
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
		case "--help":
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
		case "--verbose":
			verbose = true
		case "--api-key-secret":
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

	// --help inherits the normal JSON output default; --recursive expands the
	// tree and --output picks the format. A bare --recursive falls through.
	if showHelp {
		if outputConflict != "" {
			os.Exit(finishCliError(bootstrapEmitter(afdata.OutputFormatJson), outputConflict, "valid output formats: json, yaml, plain", 2))
		}
		os.Exit(printHelp(command, output, outputMissing, recursive))
	}

	// 1. Parse --output flag with structured error on failure.
	if outputMissing {
		os.Exit(finishCliError(bootstrapEmitter(afdata.OutputFormatJson), "missing value for --output: expected json, yaml, or plain", "valid output formats: json, yaml, plain", 2))
	}
	if outputConflict != "" {
		os.Exit(finishCliError(bootstrapEmitter(afdata.OutputFormatJson), outputConflict, "valid output formats: json, yaml, plain", 2))
	}
	format, err := afdata.CliParseOutput(output)
	if err != nil {
		os.Exit(finishCliError(bootstrapEmitter(afdata.OutputFormatJson), err.Error(), "", 2))
	}

	// One finite emitter for the command: result → stdout, error/log → stderr,
	// per the AFDATA output-stream contract, routed by kind.
	emitter := afdata.NewCliEmitterFinite(os.Stdout, os.Stderr, format)

	if message, hint := validateStrictArgs(args); message != "" {
		os.Exit(finishCliError(emitter, message, hint, 2))
	}

	// 2. Normalize --log filters: trim, lowercase, deduplicate.
	var filters afdata.LogFilters
	if logArg != "" {
		filters = afdata.CliParseLogFilters(strings.Split(logArg, ","))
	}
	if verbose {
		// --verbose is shorthand for --log all.
		allValues := filters.Values()
		allValues = append(allValues, "all")
		filters = afdata.CliParseLogFilters(allValues)
	}

	// Each diagnostic line self-tags with its `category`, so `--log all` reveals
	// the full set from real output. Diagnostics land on stderr.
	if logEnabled(filters, "request") {
		_ = emitter.EmitValidatedValue(buildRequestLog(command))
	}
	if logEnabled(filters, "startup") {
		_ = emitter.EmitValidatedValue(buildStartupLog(args, command, output, filters, verbose))
	}

	// 3. No subcommand → error with hint.
	if command == "" {
		os.Exit(finishCliError(emitter, "no subcommand provided", "try: agent-cli --help", 2))
	}

	switch command {
	case "echo":
		// 4. --dry-run → preview without executing. The preview carries a trace,
		// so build the event and Finish it (FinishResult builds a trace-less result).
		if dryRun {
			preview := afdata.NewJSONResult(map[string]any{
				"action": "echo",
				"log":    filters,
			}).Trace(map[string]any{"duration_ms": 0}).Build()
			os.Exit(emitter.Finish(preview, 0))
		}
		os.Exit(emitter.FinishResult(map[string]any{
			"action": "echo",
			"log":    filters,
		}))

	case "ping":
		// 5. Demonstrate a protocol v1 error with hint on failure.
		if host == "" {
			host = os.Getenv(pingHostEnv)
		}
		if host == "" {
			errVal, _ := afdata.NewJSONError("ping_target_not_configured", "ping target not configured").
				Hint("set PING_HOST or pass --host").
				Trace(map[string]any{"duration_ms": 0}).Build()
			os.Exit(emitter.Finish(errVal, 1))
		}

	case "cancel":
		errVal, _ := afdata.NewJSONError("cancelled", "operation cancelled").
			Hint("the operation was cancelled before completion").
			Trace(map[string]any{"duration_ms": 0}).Build()
		os.Exit(emitter.Finish(errVal, 1))

	default:
		os.Exit(finishCliError(emitter, "unknown command: "+command, "valid commands: echo, ping, cancel", 2))
	}
}
