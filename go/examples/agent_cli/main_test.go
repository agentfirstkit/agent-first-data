package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

func TestRootHelpIsOneLevel(t *testing.T) {
	help := formatRootHelp(true)
	for _, want := range []string{"echo", "ping", "--output", "--help"} {
		if !containsStr(help, want) {
			t.Errorf("root --help missing %q", want)
		}
	}
	if !containsStr(help, "AFDATA: "+afdata.Version) {
		t.Errorf("root --help missing AFDATA version")
	}
	for _, notWant := range []string{"--help-all", "--dry-run", "--host", "--stream", "--result-only"} {
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

// The about lives in the markdown heading only; it must never be repeated as
// the first line of the fenced help block.
func TestMarkdownAboutAppearsOnce(t *testing.T) {
	root := formatMarkdownHelp("", false)
	if n := strings.Count(root, "Minimal agent-first CLI example"); n != 1 {
		t.Errorf("root about must appear once (heading only), got %d:\n%s", n, root)
	}
	echo := formatMarkdownHelp("echo", false)
	if n := strings.Count(echo, "Echo back the input as structured output"); n != 1 {
		t.Errorf("subcommand about must appear once (heading only), got %d:\n%s", n, echo)
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
	versions, ok := schema["versions"].(map[string]any)
	if !ok || versions["afdata"] != afdata.Version {
		t.Fatalf("help schema must include only the AFDATA version: %v", schema["versions"])
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
	echoHelp := formatSubcommandHelp("echo", true, true)
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
	target := formatSubcommandHelp("echo", true, true)
	for _, want := range []string{"--output", "markdown"} {
		if !containsStr(target, want) {
			t.Errorf("leaf --help target missing %q:\n%s", want, target)
		}
	}
	descendant := formatSubcommandHelp("echo", false, false)
	if containsStr(descendant, "Global options") {
		t.Errorf("descendant rendering must not repeat global options:\n%s", descendant)
	}
}

// Invariant: every --help output, in every format, documents the help formats.
func TestHelpAlwaysDocumentsFormats(t *testing.T) {
	// Structured (json/yaml) schema, root and leaf targets.
	root := afdata.Render(helpSchema("", "one_level"), afdata.OutputFormatJson, afdata.OutputOptions{})
	for _, want := range []string{"--output", "markdown", "--recursive"} {
		if !containsStr(root, want) {
			t.Errorf("root help schema missing %q:\n%s", want, root)
		}
	}
	leaf := afdata.Render(helpSchema("echo", "one_level"), afdata.OutputFormatJson, afdata.OutputOptions{})
	if !containsStr(leaf, "--output") || !containsStr(leaf, "markdown") {
		t.Errorf("leaf help schema must document --output formats:\n%s", leaf)
	}
	// Plain and markdown root help.
	if !containsStr(formatRootHelp(true), "markdown") {
		t.Error("root plain help must mention the markdown format")
	}
}

func TestHelpRedactsSecretDefaults(t *testing.T) {
	secretDefault, redactionMarker := securityHelpDefaultCase(t)
	if secretDefault != helpDefaultAPIKeySecret {
		t.Fatalf("fixture default must match example default: %q", secretDefault)
	}
	if redactionMarker != "***" {
		t.Fatalf("fixture expected marker must match help redaction: %q", redactionMarker)
	}
	rendered := []string{
		formatRootHelp(true),
		formatMarkdownHelp("", false),
		afdata.Render(helpSchema("", "one_level"), afdata.OutputFormatJson, afdata.OutputOptions{}),
		afdata.Render(helpSchema("", "one_level"), afdata.OutputFormatYaml, afdata.OutputOptions{}),
	}
	for _, text := range rendered {
		if !containsStr(text, redactionMarker) {
			t.Errorf("help output must contain redaction marker:\n%s", text)
		}
		if containsStr(text, secretDefault) {
			t.Errorf("help output leaked secret default:\n%s", text)
		}
	}
}

func securityHelpDefaultCase(t *testing.T) (string, string) {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("locate test file")
	}
	path := filepath.Join(filepath.Dir(file), "..", "..", "..", "spec", "fixtures", "security.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read security fixture: %v", err)
	}
	var fixture struct {
		HelpDefaultCases []struct {
			Default  string `json:"default"`
			Expected string `json:"expected"`
		} `json:"help_default_cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatalf("parse security fixture: %v", err)
	}
	if len(fixture.HelpDefaultCases) == 0 {
		t.Fatal("security fixture has no help_default_cases")
	}
	helpCase := fixture.HelpDefaultCases[0]
	return helpCase.Default, helpCase.Expected
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

func TestOutputFlagMissing(t *testing.T) {
	for _, args := range [][]string{
		{"--output"},
		{"--output", "--json"},
		{"--output="},
	} {
		if !outputFlagMissing(args) {
			t.Fatalf("outputFlagMissing(%v) = false", args)
		}
	}
	for _, args := range [][]string{
		{"--output", "json"},
		{"--output=json"},
		{"--json"},
	} {
		if outputFlagMissing(args) {
			t.Fatalf("outputFlagMissing(%v) = true", args)
		}
	}
}

func TestValidateStrictArgs(t *testing.T) {
	valid := [][]string{
		{"echo"},
		{"echo", "--dry-run"},
		{"ping", "--host", "example.com"},
	}
	for _, args := range valid {
		if message, _ := validateStrictArgs(args); message != "" {
			t.Fatalf("validateStrictArgs(%v) unexpected error: %s", args, message)
		}
	}
	invalid := [][]string{
		{"--bogus", "echo"},
		{"--log"},
		{"echo", "--host", "example.com"},
		{"echo", "extra"},
		{"ping", "extra"},
	}
	for _, args := range invalid {
		if message, _ := validateStrictArgs(args); message == "" {
			t.Fatalf("validateStrictArgs(%v) unexpectedly passed", args)
		}
	}
}

func TestLogEnabledWildcards(t *testing.T) {
	emptyFilters := afdata.CliParseLogFilters([]string{})
	if logEnabled(emptyFilters, "startup") {
		t.Error("empty filters must not enable startup")
	}
	startupFilters := afdata.CliParseLogFilters([]string{"startup"})
	if !logEnabled(startupFilters, "startup") {
		t.Error("explicit startup must be enabled")
	}
	if logEnabled(startupFilters, "request") {
		t.Error("startup must not enable request")
	}
	// "all" is the single wildcard word; it enables every category.
	allFilters := afdata.CliParseLogFilters([]string{"all"})
	if !logEnabled(allFilters, "startup") || !logEnabled(allFilters, "request") {
		t.Error(`"all" must enable every category`)
	}
	// "*" is not special — it is a literal prefix, so it enables nothing
	// unless an event name actually starts with it.
	starFilters := afdata.CliParseLogFilters([]string{"*"})
	if logEnabled(starFilters, "request") {
		t.Error(`"*" must not be a wildcard`)
	}
}

func TestLogLinesAreCategoryTagged(t *testing.T) {
	req := buildRequestLog("")
	reqPayload := req["log"].(map[string]any)
	if req["kind"] != "log" || reqPayload["category"] != "request" || reqPayload["command"] != "none" {
		t.Errorf("request log not tagged correctly: %v", req)
	}
	raw := []string{"--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"}
	startupFilters := afdata.CliParseLogFilters([]string{"startup"})
	start := buildStartupLog(raw, "ping", "yaml", startupFilters, false)
	startPayload := start["log"].(map[string]any)
	if start["kind"] != "log" || startPayload["category"] != "startup" {
		t.Errorf("startup log not tagged correctly: %v", start)
	}
	// argv is now a []interface{} after RedactedValue conversion
	// Note: RedactArgv was deleted in 0.16; argv redaction is no longer automatic
	argvRaw, ok := startPayload["argv"].([]interface{})
	if !ok {
		t.Fatalf("argv not []interface{}: %#v", startPayload["argv"])
	}
	// Arguments are passed through as-is, not redacted (RedactArgv removed in 0.16)
	argvExpected := []interface{}{"--output", "yaml", "--log", "startup", "--api-key-secret", "sk-test", "ping"}
	if len(argvRaw) != len(argvExpected) {
		t.Fatalf("argv length = %d, want %d: %v", len(argvRaw), len(argvExpected), argvRaw)
	}
	for i, expected := range argvExpected {
		if argvRaw[i] != expected {
			t.Errorf("argv[%d] = %q, want %q", i, argvRaw[i], expected)
		}
	}
	parsed, ok := startPayload["parsed"].(map[string]any)
	if !ok {
		t.Fatalf("parsed missing: %v", startPayload["parsed"])
	}
	if parsed["command"] != "ping" || parsed["output"] != "yaml" || parsed["verbose"] != false {
		t.Fatalf("unexpected parsed config: %v", parsed)
	}
	assertStringSlice(t, parsed["log"], []string{"startup"})
	effective, ok := startPayload["effective_config"].(map[string]any)
	if !ok {
		t.Fatalf("effective_config missing: %v", startPayload["effective_config"])
	}
	if effective["output"] != "yaml" {
		t.Fatalf("unexpected effective_config: %v", effective)
	}
	assertStringSlice(t, effective["log"], []string{"startup"})
	env, ok := startPayload["env"].([]map[string]any)
	if !ok || len(env) != 1 {
		t.Fatalf("unexpected env snapshot: %v", startPayload["env"])
	}
	if env[0]["key"] != pingHostEnv {
		t.Fatalf("unexpected env key: %v", env[0])
	}
}

func assertStringSlice(t *testing.T, got any, want []string) {
	t.Helper()
	// Handle both []string and []interface{} (from JSON unmarshaling)
	var values []string
	switch v := got.(type) {
	case []string:
		values = v
	case []interface{}:
		values = make([]string, len(v))
		for i, item := range v {
			s, ok := item.(string)
			if !ok {
				t.Fatalf("slice[%d] is not a string: %#v", i, item)
			}
			values[i] = s
		}
	default:
		t.Fatalf("expected []string or []interface{}, got %T: %#v", got, got)
	}
	if len(values) != len(want) {
		t.Fatalf("slice length = %d, want %d: %v", len(values), len(want), values)
	}
	for i := range values {
		if values[i] != want[i] {
			t.Fatalf("slice[%d] = %q, want %q: %v", i, values[i], want[i], values)
		}
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
	if len(got.Values()) != 2 {
		t.Errorf("unexpected number of filters: %v", got.Values())
	}
	if !got.Enabled("query") || !got.Enabled("error") {
		t.Errorf("filters not working: %v", got.Values())
	}
}

func TestBuildCliErrorStructure(t *testing.T) {
	event, _ := afdata.BuildCLIError("bad flag", "")
	v := event.Value()
	if v["kind"] != "error" {
		t.Errorf("kind = %v", v["kind"])
	}
	errPayload := v["error"].(map[string]any)
	if errPayload["code"] != "cli_error" {
		t.Errorf("error.code = %v", errPayload["code"])
	}
	if _, ok := v["retryable"]; ok {
		t.Errorf("unexpected retryable = %v", v["retryable"])
	}
}

func TestBuildCliErrorWithHint(t *testing.T) {
	event, _ := afdata.BuildCLIError("unknown action: foo", "valid actions: echo, ping")
	v := event.Value()
	if v["kind"] != "error" {
		t.Errorf("kind = %v", v["kind"])
	}
	errPayload := v["error"].(map[string]any)
	if errPayload["code"] != "cli_error" {
		t.Errorf("error.code = %v", errPayload["code"])
	}
	if errPayload["hint"] != "valid actions: echo, ping" {
		t.Errorf("hint = %v", errPayload["hint"])
	}
}

func TestBuildJsonErrorWithHint(t *testing.T) {
	event, _ := afdata.NewJSONError("not_configured", "not configured").Hint("set PING_HOST").Build()
	v := event.Value()
	if v["kind"] != "error" {
		t.Errorf("kind = %v", v["kind"])
	}
	errPayload := v["error"].(map[string]any)
	if errPayload["code"] != "not_configured" {
		t.Errorf("error.code = %v", errPayload["code"])
	}
	if errPayload["message"] != "not configured" {
		t.Errorf("error.message = %v", errPayload["message"])
	}
	if errPayload["hint"] != "set PING_HOST" {
		t.Errorf("hint = %v", errPayload["hint"])
	}
}

func TestBuildJsonErrorWithoutHint(t *testing.T) {
	event, _ := afdata.NewJSONError("failed", "something failed").Build()
	v := event.Value()
	errPayload := v["error"].(map[string]any)
	if _, ok := errPayload["hint"]; ok {
		t.Errorf("hint should not be present, got %v", errPayload["hint"])
	}
}

func TestCliOutputAllFormats(t *testing.T) {
	event := afdata.NewJSONResult(map[string]any{"ok": true}).Build()
	v := event.Value()
	for _, f := range []afdata.OutputFormat{afdata.OutputFormatJson, afdata.OutputFormatYaml, afdata.OutputFormatPlain} {
		out := afdata.Render(v, f, afdata.OutputOptions{})
		if out == "" {
			t.Errorf("Render(%v) returned empty string", f)
		}
	}
}

func TestErrorRoundTripIsValidJsonl(t *testing.T) {
	event, _ := afdata.BuildCLIError("oops", "")
	v := event.Value()
	s := afdata.Render(v, afdata.OutputFormatJson, afdata.OutputOptions{})
	if len(s) == 0 {
		t.Error("empty json")
	}
	for _, c := range s {
		if c == '\n' {
			t.Error("json contains newline")
		}
	}
}
