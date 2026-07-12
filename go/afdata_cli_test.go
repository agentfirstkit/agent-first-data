package afdata

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
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
	if !sliceEq(got.Values(), want) {
		t.Errorf("got %v, want %v", got.Values(), want)
	}
}

func TestCliParseLogFilters_Deduplicates(t *testing.T) {
	got := CliParseLogFilters([]string{"query", "error", "Query", "query"})
	want := []string{"query", "error"}
	if !sliceEq(got.Values(), want) {
		t.Errorf("got %v, want %v", got.Values(), want)
	}
}

func TestCliParseLogFilters_RemovesEmpty(t *testing.T) {
	got := CliParseLogFilters([]string{"", "query", "  "})
	want := []string{"query"}
	if !sliceEq(got.Values(), want) {
		t.Errorf("got %v, want %v", got.Values(), want)
	}
}

func TestCliParseLogFilters_EmptySlice(t *testing.T) {
	got := CliParseLogFilters([]string{})
	if !got.IsEmpty() {
		t.Errorf("expected empty, got %v", got.Values())
	}
}

func TestCliParseLogFilters_PreservesOrder(t *testing.T) {
	got := CliParseLogFilters([]string{"startup", "request", "retry"})
	want := []string{"startup", "request", "retry"}
	if !sliceEq(got.Values(), want) {
		t.Errorf("got %v, want %v", got.Values(), want)
	}
}

// ═══════════════════════════════════════════
// BuildCLIError
// ═══════════════════════════════════════════

func TestBuildCLIError_RequiredFields(t *testing.T) {
	event, err := BuildCLIError("missing --sql", "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	v := event.Value()
	if v["kind"] != "error" {
		t.Errorf("kind = %v", v["kind"])
	}
	errPayload := v["error"].(map[string]any)
	if errPayload["code"] != "cli_error" {
		t.Errorf("error.code = %v", errPayload["code"])
	}
	if errPayload["message"] != "missing --sql" {
		t.Errorf("error.message = %v", errPayload["message"])
	}
	if errPayload["retryable"] != false {
		t.Errorf("error.retryable = %v", errPayload["retryable"])
	}
	if _, ok := v["error_code"]; ok {
		t.Errorf("unexpected error_code = %v", v["error_code"])
	}
	if _, ok := v["retryable"]; ok {
		t.Errorf("unexpected retryable = %v", v["retryable"])
	}
	if trace, ok := v["trace"].(map[string]any); !ok || len(trace) != 0 {
		t.Errorf("trace = %v, want empty object", v["trace"])
	}
}

func TestBuildCLIError_WithHint(t *testing.T) {
	event, err := BuildCLIError("bad flag", "try --help")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	v := event.Value()
	errPayload := v["error"].(map[string]any)
	if errPayload["hint"] != "try --help" {
		t.Errorf("hint = %v, want 'try --help'", errPayload["hint"])
	}
}

func TestBuildCLIError_WithoutHintHasNoHintKey(t *testing.T) {
	event, err := BuildCLIError("oops", "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	v := event.Value()
	errPayload := v["error"].(map[string]any)
	if _, ok := errPayload["hint"]; ok {
		t.Errorf("expected no hint key, got %v", errPayload["hint"])
	}
}

func TestBuildCLIError_IsValidJson(t *testing.T) {
	event, err := BuildCLIError("oops", "")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	v := event.Value()
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
	event, _ := NewJSONResult(map[string]any{"size_bytes": int64(1024)}).Build()
	v := event.Value()
	out := CliOutput(v, OutputFormatJson)
	if !contains(out, "size_bytes") {
		t.Errorf("json output should preserve raw keys, got: %s", out)
	}
	if contains(out, "\n") {
		t.Error("json output should be single line")
	}
}

func TestCliOutput_DispatchesYaml(t *testing.T) {
	event, _ := NewJSONResult(map[string]any{"size_bytes": int64(1024)}).Build()
	v := event.Value()
	out := CliOutput(v, OutputFormatYaml)
	if !contains(out, "---") {
		t.Errorf("yaml output should start with ---, got: %s", out)
	}
	if !contains(out, "size:") {
		t.Errorf("yaml output should strip suffix, got: %s", out)
	}
}

func TestCliOutput_DispatchesPlain(t *testing.T) {
	event, _ := NewJSONResult(map[string]any{"ok": true}).Build()
	v := event.Value()
	out := CliOutput(v, OutputFormatPlain)
	if !contains(out, "kind=result") {
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
// CliEmitter
// ═══════════════════════════════════════════

func TestCliEmitterWritesEventsAndTracksTerminal(t *testing.T) {
	var buf bytes.Buffer
	emitter := NewCliEmitter(&buf, OutputFormatJson)
	logEvent, _ := NewJSONLog(LogLevelInfo, "startup").Build()
	if err := emitter.Emit(logEvent); err != nil {
		t.Fatalf("log emit: %v", err)
	}
	resultEvent, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(resultEvent); err != nil {
		t.Fatalf("result emit: %v", err)
	}
	out := buf.String()
	if !contains(out, "\"kind\":\"log\"") || !contains(out, "\"kind\":\"result\"") {
		t.Fatalf("unexpected output: %s", out)
	}
}

func TestCliEmitterFramingAllFormats(t *testing.T) {
	logEvent, _ := NewJSONLog(LogLevelInfo, "startup").Build()
	resultEvent, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	cases := []struct {
		name   string
		format OutputFormat
		check  func(t *testing.T, out string)
	}{
		{
			name:   "json",
			format: OutputFormatJson,
			check: func(t *testing.T, out string) {
				lines := strings.Split(strings.TrimSuffix(out, "\n"), "\n")
				if len(lines) != 2 {
					t.Fatalf("json should frame one event per line: %q", out)
				}
				for _, line := range lines {
					var parsed map[string]any
					if err := json.Unmarshal([]byte(line), &parsed); err != nil {
						t.Fatalf("json frame is not valid json: %q: %v", line, err)
					}
				}
			},
		},
		{
			name:   "plain",
			format: OutputFormatPlain,
			check: func(t *testing.T, out string) {
				lines := strings.Split(strings.TrimSuffix(out, "\n"), "\n")
				if len(lines) != 2 || !strings.HasPrefix(lines[0], "kind=log") || !strings.HasPrefix(lines[1], "kind=result") {
					t.Fatalf("plain should frame one display event per line: %q", out)
				}
			},
		},
		{
			name:   "yaml",
			format: OutputFormatYaml,
			check: func(t *testing.T, out string) {
				if strings.Count(out, "---") != 2 {
					t.Fatalf("yaml should frame every event with a document boundary: %q", out)
				}
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var buf bytes.Buffer
			emitter := NewCliEmitter(&buf, tc.format)
			if err := emitter.Emit(logEvent); err != nil {
				t.Fatalf("emit: %v", err)
			}
			if err := emitter.Emit(resultEvent); err != nil {
				t.Fatalf("emit: %v", err)
			}
			tc.check(t, buf.String())
		})
	}
}

func TestCliEmitterRejectsDuplicateTerminal(t *testing.T) {
	var buf bytes.Buffer
	emitter := NewCliEmitter(&buf, OutputFormatJson)
	resultEvent, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(resultEvent); err != nil {
		t.Fatalf("result emit: %v", err)
	}
	errorEvent, _ := NewJSONError("late_error", "too late").Build()
	err := emitter.Emit(errorEvent)
	if err == nil || !contains(err.Error(), "duplicate terminal") {
		t.Fatalf("expected duplicate terminal error, got %v", err)
	}
}

func TestCliEmitterRejectsNonTerminalAfterTerminal(t *testing.T) {
	var buf bytes.Buffer
	emitter := NewCliEmitter(&buf, OutputFormatJson)
	resultEvent, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(resultEvent); err != nil {
		t.Fatalf("result emit: %v", err)
	}
	progressEvent, _ := NewJSONProgress("working").Build()
	err := emitter.Emit(progressEvent)
	if err == nil || !contains(err.Error(), "after terminal") {
		t.Fatalf("expected after terminal error, got %v", err)
	}
}

type failingWriter struct{}

func (failingWriter) Write(_ []byte) (int, error) {
	return 0, errors.New("closed")
}

func TestCliEmitterReturnsWriterErrors(t *testing.T) {
	emitter := NewCliEmitter(failingWriter{}, OutputFormatJson)
	event, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	err := emitter.Emit(event)
	if err == nil || !contains(err.Error(), "failed to write") {
		t.Fatalf("expected writer error, got %v", err)
	}
}

type failOnceWriter struct {
	failed bool
	buf    bytes.Buffer
}

func (w *failOnceWriter) Write(value []byte) (int, error) {
	if !w.failed {
		w.failed = true
		return 0, errors.New("retry")
	}
	return w.buf.Write(value)
}

func TestCliEmitterDoesNotCommitTerminalStateWhenWriteFails(t *testing.T) {
	writer := &failOnceWriter{}
	emitter := NewCliEmitter(writer, OutputFormatJson)
	event, _ := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(event); err == nil {
		t.Fatal("first write must fail")
	}
	if err := emitter.Emit(event); err != nil {
		t.Fatalf("terminal event should remain retryable: %v", err)
	}
	if lines := strings.Count(strings.TrimSpace(writer.buf.String()), "\n") + 1; lines != 1 {
		t.Fatalf("expected one event line, got %d", lines)
	}
}

// ═══════════════════════════════════════════
// Version helpers
// ═══════════════════════════════════════════

func TestBuildCliVersion_StandardShape(t *testing.T) {
	v := BuildCliVersion("1.2.3")
	if v["kind"] != "result" {
		t.Errorf("kind = %v", v["kind"])
	}
	result := v["result"].(map[string]any)
	if result["version"] != "1.2.3" {
		t.Errorf("version = %v", result["version"])
	}
	if _, ok := v["trace"]; ok {
		t.Errorf("unexpected trace = %v", v["trace"])
	}
}

func TestCliRenderVersion_DefaultJson(t *testing.T) {
	out := CliRenderVersion("agent-cli", "1.2.3", OutputFormatJson)
	if !contains(out, "\"kind\":\"result\"") {
		t.Errorf("json version missing kind: %s", out)
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
	if !contains(out, "kind=result") || !contains(out, "result.version=1.2.3") {
		t.Errorf("plain version output = %s", out)
	}
}

func TestCliHandleVersionOrContinue_JsonAlias(t *testing.T) {
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--version", "--json"},
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
	if !contains(out, "\"kind\":\"result\"") || !contains(out, "\"version\":\"1.2.3\"") {
		t.Errorf("json alias version output = %s", out)
	}
}

func TestCliHandleVersionOrContinue_JsonAliasConflict(t *testing.T) {
	_, handled, err := CliHandleVersionOrContinue(
		[]string{"--version", "--json", "--output", "yaml"},
		"agent-cli",
		"1.2.3",
		"",
	)
	if !handled {
		t.Fatal("expected handled")
	}
	if err == nil || !contains(err.Error(), "conflicting output formats") {
		t.Fatalf("expected conflict error, got %v", err)
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
