package afdata

import (
	"testing"
)

// ═══════════════════════════════════════════
// CliParseOutput
// ═══════════════════════════════════════════

func TestCliParseOutput_AllFormats(t *testing.T) {
	cases := []struct {
		in   string
		want OutputFormat
	}{
		{"json", OutputFormatJson},
		{"yaml", OutputFormatYaml},
		{"plain", OutputFormatPlain},
	}
	for _, c := range cases {
		got, err := CliParseOutput(c.in)
		if err != nil {
			t.Errorf("CliParseOutput(%q): unexpected error: %v", c.in, err)
		}
		if got != c.want {
			t.Errorf("CliParseOutput(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestCliParseOutput_RejectsUnknown(t *testing.T) {
	for _, s := range []string{"xml", "JSON", "YAML", ""} {
		_, err := CliParseOutput(s)
		if err == nil {
			t.Errorf("CliParseOutput(%q): expected error, got nil", s)
		}
	}
}

func TestCliParseOutput_ErrorContainsValue(t *testing.T) {
	_, err := CliParseOutput("toml")
	if err == nil {
		t.Fatal("expected error")
	}
	msg := err.Error()
	if !contains(msg, "toml") {
		t.Errorf("error %q does not contain input value", msg)
	}
	if !contains(msg, "json") {
		t.Errorf("error %q does not mention expected values", msg)
	}
}

// ═══════════════════════════════════════════
// CliParseLogFilters
// ═══════════════════════════════════════════

func TestCliParseLogFilters_TrimsAndLowercases(t *testing.T) {
	got := CliParseLogFilters([]string{"  Query  ", "ERROR"})
	want := []string{"query", "error"}
	if !sliceEq(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

func TestCliParseLogFilters_Deduplicates(t *testing.T) {
	got := CliParseLogFilters([]string{"query", "error", "Query", "query"})
	want := []string{"query", "error"}
	if !sliceEq(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

func TestCliParseLogFilters_RemovesEmpty(t *testing.T) {
	got := CliParseLogFilters([]string{"", "query", "  "})
	want := []string{"query"}
	if !sliceEq(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

func TestCliParseLogFilters_EmptySlice(t *testing.T) {
	got := CliParseLogFilters([]string{})
	if len(got) != 0 {
		t.Errorf("expected empty, got %v", got)
	}
}

func TestCliParseLogFilters_PreservesOrder(t *testing.T) {
	got := CliParseLogFilters([]string{"startup", "request", "retry"})
	want := []string{"startup", "request", "retry"}
	if !sliceEq(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

// ═══════════════════════════════════════════
// BuildCliError
// ═══════════════════════════════════════════

func TestBuildCliError_RequiredFields(t *testing.T) {
	v := BuildCliError("missing --sql", "")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["error"] != "missing --sql" {
		t.Errorf("error = %v", v["error"])
	}
	if _, ok := v["error_code"]; ok {
		t.Errorf("unexpected error_code = %v", v["error_code"])
	}
	if _, ok := v["retryable"]; ok {
		t.Errorf("unexpected retryable = %v", v["retryable"])
	}
	if _, ok := v["trace"]; ok {
		t.Errorf("unexpected trace = %v", v["trace"])
	}
}

func TestBuildCliError_WithHint(t *testing.T) {
	v := BuildCliError("bad flag", "try --help")
	if v["hint"] != "try --help" {
		t.Errorf("hint = %v, want 'try --help'", v["hint"])
	}
}

func TestBuildCliError_WithoutHintHasNoHintKey(t *testing.T) {
	v := BuildCliError("oops", "")
	if _, ok := v["hint"]; ok {
		t.Errorf("expected no hint key, got %v", v["hint"])
	}
}

func TestBuildCliError_IsValidJson(t *testing.T) {
	v := BuildCliError("oops", "")
	s := OutputJson(v)
	if s == "" {
		t.Error("OutputJson returned empty string")
	}
	if !contains(s, "error") {
		t.Errorf("json %q missing 'error'", s)
	}
}

// ═══════════════════════════════════════════
// CliOutput
// ═══════════════════════════════════════════

func TestCliOutput_DispatchesJson(t *testing.T) {
	v := map[string]any{"code": "ok", "size_bytes": int64(1024)}
	out := CliOutput(v, OutputFormatJson)
	if !contains(out, "size_bytes") {
		t.Errorf("json output should preserve raw keys, got: %s", out)
	}
	if contains(out, "\n") {
		t.Error("json output should be single line")
	}
}

func TestCliOutput_DispatchesYaml(t *testing.T) {
	v := map[string]any{"code": "ok", "size_bytes": int64(1024)}
	out := CliOutput(v, OutputFormatYaml)
	if !contains(out, "---") {
		t.Errorf("yaml output should start with ---, got: %s", out)
	}
	if !contains(out, "size:") {
		t.Errorf("yaml output should strip suffix, got: %s", out)
	}
}

func TestCliOutput_DispatchesPlain(t *testing.T) {
	v := map[string]any{"code": "ok"}
	out := CliOutput(v, OutputFormatPlain)
	if !contains(out, "code=ok") {
		t.Errorf("plain output should be logfmt, got: %s", out)
	}
}

func TestCliOutputWithOptions_DispatchesRawYaml(t *testing.T) {
	v := map[string]any{"size_bytes": int64(1024)}
	out := CliOutputWithOptions(
		v,
		OutputFormatYaml,
		OutputOptions{Style: OutputStyleRaw},
	)
	if !contains(out, "size_bytes: 1024") {
		t.Errorf("raw yaml output should preserve suffix key, got: %s", out)
	}
	if contains(out, "size:") {
		t.Errorf("raw yaml output should not strip suffix, got: %s", out)
	}
}

// ═══════════════════════════════════════════
// Version helpers
// ═══════════════════════════════════════════

func TestBuildCliVersion_StandardShape(t *testing.T) {
	v := BuildCliVersion("1.2.3")
	if v["code"] != "version" {
		t.Errorf("code = %v", v["code"])
	}
	if v["version"] != "1.2.3" {
		t.Errorf("version = %v", v["version"])
	}
	if _, ok := v["trace"]; ok {
		t.Errorf("unexpected trace = %v", v["trace"])
	}
}

func TestCliRenderVersion_DefaultJson(t *testing.T) {
	out := CliRenderVersion("agent-cli", "1.2.3", OutputFormatJson)
	if !contains(out, "\"code\":\"version\"") {
		t.Errorf("json version missing code: %s", out)
	}
	if !contains(out, "\"version\":\"1.2.3\"") {
		t.Errorf("json version missing version: %s", out)
	}
}

func TestCliRenderVersion_ConventionalText(t *testing.T) {
	got := CliRenderVersion("agent-cli", "1.2.3", "")
	if got != "agent-cli 1.2.3\n" {
		t.Errorf("got %q", got)
	}
}

func TestCliHandleVersionOrContinue_HonorsOutputFlag(t *testing.T) {
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--version", "--output", "plain"},
		"agent-cli",
		"1.2.3",
		OutputFormatJson,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	if !contains(out, "code=version") || !contains(out, "version=1.2.3") {
		t.Errorf("plain version output = %s", out)
	}
}

func TestCliHandleVersionOrContinue_ConventionalDefault(t *testing.T) {
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--version"},
		"agent-cli",
		"1.2.3",
		"",
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	if out != "agent-cli 1.2.3\n" {
		t.Errorf("out = %q", out)
	}
}

func TestCliHandleVersionOrContinue_ReturnsNoneWithoutVersion(t *testing.T) {
	_, handled, err := CliHandleVersionOrContinue(
		[]string{"ping"},
		"agent-cli",
		"1.2.3",
		OutputFormatJson,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if handled {
		t.Fatal("expected handled=false")
	}
}

func TestCliHandleVersionOrContinue_RejectsInvalidOutput(t *testing.T) {
	_, handled, err := CliHandleVersionOrContinue(
		[]string{"--version", "--output", "xml"},
		"agent-cli",
		"1.2.3",
		OutputFormatJson,
	)
	if !handled {
		t.Fatal("expected handled")
	}
	if err == nil || !contains(err.Error(), "xml") {
		t.Fatalf("expected xml error, got %v", err)
	}
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

func sliceEq(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (s == sub || len(sub) == 0 ||
		func() bool {
			for i := 0; i <= len(s)-len(sub); i++ {
				if s[i:i+len(sub)] == sub {
					return true
				}
			}
			return false
		}())
}
