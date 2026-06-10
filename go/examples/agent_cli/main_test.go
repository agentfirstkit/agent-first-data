package main

import (
	"strings"
	"testing"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

func TestRootHelpIsOneLevel(t *testing.T) {
	help := formatRootHelp()
	for _, want := range []string{"echo", "ping", "--output", "--help"} {
		if !containsStr(help, want) {
			t.Errorf("root --help missing %q", want)
		}
	}
	for _, notWant := range []string{"--help-all", "--dry-run", "--host"} {
		if containsStr(help, notWant) {
			t.Errorf("root --help should not include %q", notWant)
		}
	}
}

func TestRecursiveMarkdownExportContainsSubcommandDetails(t *testing.T) {
	help := formatMarkdownHelp("", true)
	for _, want := range []string{"# agent-cli", "echo", "ping", "--dry-run", "--host"} {
		if !containsStr(help, want) {
			t.Errorf("recursive markdown help missing %q", want)
		}
	}
}

func TestOneLevelMarkdownOmitsDescendantDetails(t *testing.T) {
	help := formatMarkdownHelp("", false)
	if !containsStr(help, "# agent-cli") {
		t.Error("one-level markdown missing root heading")
	}
	for _, notWant := range []string{"--dry-run", "--host"} {
		if containsStr(help, notWant) {
			t.Errorf("one-level markdown should not expand %q", notWant)
		}
	}
}

func TestRecursivePlainContainsSubcommandDetails(t *testing.T) {
	help := formatCompleteHelp()
	for _, want := range []string{"echo", "ping", "--output", "--dry-run", "--host"} {
		if !containsStr(help, want) {
			t.Errorf("recursive help missing %q", want)
		}
	}
}

func TestHelpSchemaIsRecursiveExport(t *testing.T) {
	schema := helpSchema("", "recursive")
	if schema["code"] != "help" || schema["scope"] != "recursive" {
		t.Fatalf("unexpected help schema header: %v", schema)
	}
	commands, ok := schema["commands"].([]map[string]any)
	if !ok || len(commands) == 0 {
		t.Fatalf("commands missing from schema: %v", schema["commands"])
	}
	if _, ok := commands[0]["flags"]; !ok {
		t.Fatalf("recursive schema should include child flags: %v", commands[0])
	}
}

func TestHelpSchemaOneLevelOmitsChildFlags(t *testing.T) {
	schema := helpSchema("", "one_level")
	if schema["scope"] != "one_level" {
		t.Fatalf("unexpected scope: %v", schema["scope"])
	}
	commands, ok := schema["commands"].([]map[string]any)
	if !ok || len(commands) == 0 {
		t.Fatalf("commands missing from schema: %v", schema["commands"])
	}
	if _, ok := commands[0]["flags"]; ok {
		t.Fatalf("one-level schema must not include child flags: %v", commands[0])
	}
}

func TestSubcommandHelpScoped(t *testing.T) {
	echoHelp := formatSubcommandHelp("echo", true)
	if !containsStr(echoHelp, "--dry-run") {
		t.Error("echo --help missing --dry-run")
	}
	if containsStr(echoHelp, "--host") {
		t.Error("echo --help should NOT contain --host")
	}
}

// A leaf --help target must still advertise the --output formats; a descendant
// rendering (withGlobals=false) must not, to keep recursive dumps lean.
func TestLeafHelpTargetDocumentsFormats(t *testing.T) {
	target := formatSubcommandHelp("echo", true)
	for _, want := range []string{"--output", "markdown"} {
		if !containsStr(target, want) {
			t.Errorf("leaf --help target missing %q:\n%s", want, target)
		}
	}
	descendant := formatSubcommandHelp("echo", false)
	if containsStr(descendant, "Global options") {
		t.Errorf("descendant rendering must not repeat global options:\n%s", descendant)
	}
}

// Invariant: every --help output, in every format, documents the help formats.
func TestHelpAlwaysDocumentsFormats(t *testing.T) {
	// Structured (json/yaml) schema, root and leaf targets.
	root := afdata.OutputJson(helpSchema("", "one_level"))
	for _, want := range []string{"--output", "markdown", "--recursive"} {
		if !containsStr(root, want) {
			t.Errorf("root help schema missing %q:\n%s", want, root)
		}
	}
	leaf := afdata.OutputJson(helpSchema("echo", "one_level"))
	if !containsStr(leaf, "--output") || !containsStr(leaf, "markdown") {
		t.Errorf("leaf help schema must document --output formats:\n%s", leaf)
	}
	// Plain and markdown root help.
	if !containsStr(formatRootHelp(), "markdown") {
		t.Error("root plain help must mention the markdown format")
	}
}

// Token economy: a recursive dump documents the modifiers once (at the root),
// never repeating the leaf "Global options" note per descendant.
func TestRecursiveDumpsDoNotRepeatGlobalOptions(t *testing.T) {
	if strings.Count(formatCompleteHelp(), "Global options") != 0 {
		t.Error("recursive plain must not repeat the leaf Global options note")
	}
	if strings.Count(formatMarkdownHelp("", true), "Global options") != 0 {
		t.Error("recursive markdown must not repeat the leaf Global options note")
	}
}

func containsStr(s, sub string) bool {
	return len(s) > 0 && len(sub) > 0 && strings.Contains(s, sub)
}

func TestLogEnabledWildcards(t *testing.T) {
	if logEnabled(nil, "startup") {
		t.Error("empty filters must not enable startup")
	}
	if !logEnabled([]string{"startup"}, "startup") {
		t.Error("explicit startup must be enabled")
	}
	if logEnabled([]string{"startup"}, "request") {
		t.Error("startup must not enable request")
	}
	for _, all := range []string{"all", "*"} {
		if !logEnabled([]string{all}, "startup") || !logEnabled([]string{all}, "request") {
			t.Errorf("%q must enable every category", all)
		}
	}
}

func TestLogLinesAreCategoryTagged(t *testing.T) {
	req := buildRequestLog("")
	if req["code"] != "log" || req["category"] != "request" || req["command"] != "none" {
		t.Errorf("request log not tagged correctly: %v", req)
	}
	start := buildStartupLog()
	if start["code"] != "log" || start["category"] != "startup" {
		t.Errorf("startup log not tagged correctly: %v", start)
	}
}

func TestParseOutputAllVariants(t *testing.T) {
	for _, s := range []string{"json", "yaml", "plain"} {
		if _, err := afdata.CliParseOutput(s); err != nil {
			t.Errorf("CliParseOutput(%q): %v", s, err)
		}
	}
}

func TestParseLogNormalizes(t *testing.T) {
	got := afdata.CliParseLogFilters([]string{"Query", " ERROR ", "query"})
	if len(got) != 2 || got[0] != "query" || got[1] != "error" {
		t.Errorf("unexpected: %v", got)
	}
}

func TestBuildCliErrorStructure(t *testing.T) {
	v := afdata.BuildCliError("bad flag", "")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if _, ok := v["retryable"]; ok {
		t.Errorf("unexpected retryable = %v", v["retryable"])
	}
}

func TestBuildCliErrorWithHint(t *testing.T) {
	v := afdata.BuildCliError("unknown action: foo", "valid actions: echo, ping")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["hint"] != "valid actions: echo, ping" {
		t.Errorf("hint = %v", v["hint"])
	}
}

func TestBuildJsonErrorWithHint(t *testing.T) {
	v := afdata.BuildJsonError("not configured", "set PING_HOST", nil)
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["error"] != "not configured" {
		t.Errorf("error = %v", v["error"])
	}
	if v["hint"] != "set PING_HOST" {
		t.Errorf("hint = %v", v["hint"])
	}
}

func TestBuildJsonErrorWithoutHint(t *testing.T) {
	v := afdata.BuildJsonError("something failed", "", nil)
	if _, ok := v["hint"]; ok {
		t.Errorf("hint should not be present, got %v", v["hint"])
	}
}

func TestCliOutputAllFormats(t *testing.T) {
	v := map[string]any{"code": "ok"}
	for _, f := range []afdata.OutputFormat{afdata.OutputFormatJson, afdata.OutputFormatYaml, afdata.OutputFormatPlain} {
		out := afdata.CliOutput(v, f)
		if out == "" {
			t.Errorf("CliOutput(%v) returned empty string", f)
		}
	}
}

func TestErrorRoundTripIsValidJsonl(t *testing.T) {
	v := afdata.BuildCliError("oops", "")
	s := afdata.OutputJson(v)
	if len(s) == 0 {
		t.Error("empty json")
	}
	for _, c := range s {
		if c == '\n' {
			t.Error("json contains newline")
		}
	}
}
