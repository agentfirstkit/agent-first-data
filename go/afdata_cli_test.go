package afdata

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"syscall"
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
	s := Render(v, OutputFormatJson, OutputOptions{})
	if s == "" {
		t.Error("OutputJson returned empty string")
	}
	if !contains(s, "error") {
		t.Errorf("json %q missing 'error'", s)
	}
}

// ═══════════════════════════════════════════
// Render
// ═══════════════════════════════════════════

func TestCliOutput_DispatchesJson(t *testing.T) {
	event := NewJSONResult(map[string]any{"size_bytes": int64(1024)}).Build()
	v := event.Value()
	out := Render(v, OutputFormatJson, OutputOptions{})
	if !contains(out, "size_bytes") {
		t.Errorf("json output should preserve raw keys, got: %s", out)
	}
	if contains(out, "\n") {
		t.Error("json output should be single line")
	}
}

func TestCliOutput_DispatchesYaml(t *testing.T) {
	event := NewJSONResult(map[string]any{"size_bytes": int64(1024)}).Build()
	v := event.Value()
	out := Render(v, OutputFormatYaml, OutputOptions{})
	if !contains(out, "---") {
		t.Errorf("yaml output should start with ---, got: %s", out)
	}
	if !contains(out, "size_bytes: 1024") {
		t.Errorf("yaml output should be structure-preserving (raw key, raw value), got: %s", out)
	}
}

func TestCliOutput_DispatchesPlain(t *testing.T) {
	event := NewJSONResult(map[string]any{"ok": true}).Build()
	v := event.Value()
	out := Render(v, OutputFormatPlain, OutputOptions{})
	if !contains(out, "kind=result") {
		t.Errorf("plain output should be logfmt, got: %s", out)
	}
}

func TestCliOutputWithOptions_DispatchesRawYaml(t *testing.T) {
	v := map[string]any{"size_bytes": int64(1024)}
	out := Render(v, OutputFormatYaml, OutputOptions{Style: PlainStyleRaw})
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
	logEvent := NewJSONLog(map[string]any{"level": "info", "message": "startup"}).Build()
	if err := emitter.Emit(logEvent); err != nil {
		t.Fatalf("log emit: %v", err)
	}
	resultEvent := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(resultEvent); err != nil {
		t.Fatalf("result emit: %v", err)
	}
	out := buf.String()
	if !contains(out, "\"kind\":\"log\"") || !contains(out, "\"kind\":\"result\"") {
		t.Fatalf("unexpected output: %s", out)
	}
}

func TestCliEmitterFramingAllFormats(t *testing.T) {
	logEvent := NewJSONLog(map[string]any{"level": "info", "message": "startup"}).Build()
	resultEvent := NewJSONResult(map[string]any{"rows": 2}).Build()
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
	resultEvent := NewJSONResult(map[string]any{"rows": 2}).Build()
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
	resultEvent := NewJSONResult(map[string]any{"rows": 2}).Build()
	if err := emitter.Emit(resultEvent); err != nil {
		t.Fatalf("result emit: %v", err)
	}
	progressEvent := NewJSONProgress(map[string]any{"message": "working"}).Build()
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
	event := NewJSONResult(map[string]any{"rows": 2}).Build()
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
	event := NewJSONResult(map[string]any{"rows": 2}).Build()
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
// OutputTo parsing
// ═══════════════════════════════════════════

func TestParseOutputTo_AllVariants(t *testing.T) {
	cases := []struct {
		in   string
		want OutputTo
	}{
		{"split", OutputToSplit},
		{"stdout", OutputToStdout},
		{"stderr", OutputToStderr},
	}
	for _, c := range cases {
		got, err := ParseOutputTo(c.in)
		if err != nil {
			t.Errorf("ParseOutputTo(%q): unexpected error: %v", c.in, err)
		}
		if got != c.want {
			t.Errorf("ParseOutputTo(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestParseOutputTo_RejectsUnknown(t *testing.T) {
	for _, s := range []string{"both", "SPLIT", "", "file"} {
		_, err := ParseOutputTo(s)
		if err == nil {
			t.Errorf("ParseOutputTo(%q): expected error, got nil", s)
		}
	}
	_, err := ParseOutputTo("both")
	if err == nil {
		t.Fatal("expected error")
	}
	if !contains(err.Error(), "both") || !contains(err.Error(), "split") {
		t.Errorf("error %q should name the bad value and the accepted set", err.Error())
	}
}

// ═══════════════════════════════════════════
// CliEmitter two-mode routing
// ═══════════════════════════════════════════

// Finite one-shot mode splits by kind: result → the primary (stdout) sink,
// while error/progress/log → the diagnostic (stderr) sink.
func TestCliEmitterFiniteSplitsResultToPrimary(t *testing.T) {
	var out, diag bytes.Buffer
	emitter := NewCliEmitterFinite(&out, &diag, OutputFormatJson)

	if err := emitter.EmitProgress("working"); err != nil {
		t.Fatalf("progress emit: %v", err)
	}
	if err := emitter.EmitLog(LogLevelInfo, "step"); err != nil {
		t.Fatalf("log emit: %v", err)
	}
	if err := emitter.EmitResult(map[string]any{"rows": 2}); err != nil {
		t.Fatalf("result emit: %v", err)
	}

	if !contains(out.String(), "\"kind\":\"result\"") {
		t.Fatalf("result must go to the primary sink, got: %q", out.String())
	}
	if contains(out.String(), "\"kind\":\"progress\"") || contains(out.String(), "\"kind\":\"log\"") {
		t.Fatalf("diagnostics must not reach the primary sink, got: %q", out.String())
	}
	if !contains(diag.String(), "\"kind\":\"progress\"") || !contains(diag.String(), "\"kind\":\"log\"") {
		t.Fatalf("progress/log must go to the diagnostic sink, got: %q", diag.String())
	}
	if contains(diag.String(), "\"kind\":\"result\"") {
		t.Fatalf("result must not reach the diagnostic sink, got: %q", diag.String())
	}
}

// In finite mode an error is a diagnostic (routing follows kind, not exit code),
// so it goes to the diagnostic sink, keeping the primary (stdout) sink free of
// anything a pipe could mistake for a successful payload.
func TestCliEmitterFiniteRoutesErrorToDiagnostic(t *testing.T) {
	var out, diag bytes.Buffer
	emitter := NewCliEmitterFinite(&out, &diag, OutputFormatJson)

	if err := emitter.EmitError("boom", "it failed"); err != nil {
		t.Fatalf("error emit: %v", err)
	}
	if out.Len() != 0 {
		t.Fatalf("primary sink must stay empty on error, got: %q", out.String())
	}
	if !contains(diag.String(), "\"kind\":\"error\"") {
		t.Fatalf("error must go to the diagnostic sink, got: %q", diag.String())
	}
}

// Event-stream (unified) mode collapses every event, including error, onto the
// single writer so a consumer reading one ordered stream sees them all.
func TestCliEmitterStreamUnifiesAllEvents(t *testing.T) {
	var buf bytes.Buffer
	emitter := NewCliEmitter(&buf, OutputFormatJson)

	if err := emitter.EmitProgress("working"); err != nil {
		t.Fatalf("progress emit: %v", err)
	}
	if err := emitter.EmitLog(LogLevelInfo, "step"); err != nil {
		t.Fatalf("log emit: %v", err)
	}
	if err := emitter.EmitError("boom", "it failed"); err != nil {
		t.Fatalf("error emit: %v", err)
	}

	out := buf.String()
	for _, want := range []string{"\"kind\":\"progress\"", "\"kind\":\"log\"", "\"kind\":\"error\""} {
		if !contains(out, want) {
			t.Fatalf("stream mode must keep %s on the single writer, got: %q", want, out)
		}
	}
	if lines := strings.Count(strings.TrimSpace(out), "\n") + 1; lines != 3 {
		t.Fatalf("expected three framed events, got %d: %q", lines, out)
	}
}

// The from-selector constructor picks the shape: split → finite (a diagnostic
// sink is wired), stdout/stderr → stream (a single writer, no diagnostic sink).
func TestNewCliEmitterFromOutputToShape(t *testing.T) {
	if e := NewCliEmitterFromOutputTo(OutputToSplit, OutputFormatJson); e.diagnostic == nil {
		t.Error("OutputToSplit must produce a finite emitter with a diagnostic sink")
	}
	if e := NewCliEmitterFromOutputTo(OutputToStdout, OutputFormatJson); e.diagnostic != nil {
		t.Error("OutputToStdout must produce a stream emitter with no diagnostic sink")
	}
	if e := NewCliEmitterFromOutputTo(OutputToStderr, OutputFormatJson); e.diagnostic != nil {
		t.Error("OutputToStderr must produce a stream emitter with no diagnostic sink")
	}
}

// ═══════════════════════════════════════════
// CliEmitter Finish / exit-code helpers
// ═══════════════════════════════════════════

func TestCliEmitterFinishSuccessReturnsSuccessCode(t *testing.T) {
	var buf bytes.Buffer
	emitter := NewCliEmitter(&buf, OutputFormatJson)
	errEvent, _ := NewJSONError("cancelled", "operation cancelled").Build()
	if code := emitter.Finish(errEvent, 1); code != 1 {
		t.Fatalf("Finish success code = %d, want 1", code)
	}
	if !contains(buf.String(), "\"kind\":\"error\"") {
		t.Fatalf("Finish must write the event: %q", buf.String())
	}
}

func TestCliEmitterFinishResultWritesResultAndReturnsZero(t *testing.T) {
	var out, diag bytes.Buffer
	emitter := NewCliEmitterFinite(&out, &diag, OutputFormatJson)
	if code := emitter.FinishResult(map[string]any{"rows": 3}); code != 0 {
		t.Fatalf("FinishResult code = %d, want 0", code)
	}
	if !contains(out.String(), "\"kind\":\"result\"") || !contains(out.String(), "rows") {
		t.Fatalf("FinishResult must write the result to the primary sink: %q", out.String())
	}
	if diag.Len() != 0 {
		t.Fatalf("FinishResult must not write to the diagnostic sink: %q", diag.String())
	}
}

// A rich error routed through the builder + Finish lands on the diagnostic sink
// with its hint intact, and Finish returns the caller's exit code.
func TestCliEmitterFinishRichErrorToDiagnostic(t *testing.T) {
	var out, diag bytes.Buffer
	emitter := NewCliEmitterFinite(&out, &diag, OutputFormatJson)
	event, err := NewJSONError("cancelled", "operation cancelled").
		Hint("retry later").
		Trace(map[string]any{"duration_ms": 0}).
		Build()
	if err != nil {
		t.Fatalf("build error event: %v", err)
	}
	if code := emitter.Finish(event, 1); code != 1 {
		t.Fatalf("Finish code = %d, want 1", code)
	}
	if out.Len() != 0 {
		t.Fatalf("error must not reach the primary sink: %q", out.String())
	}
	got := diag.String()
	if !contains(got, "\"kind\":\"error\"") || !contains(got, "cancelled") || !contains(got, "retry later") {
		t.Fatalf("error (incl. hint) must reach the diagnostic sink: %q", got)
	}
}

type epipeWriter struct{}

func (epipeWriter) Write(_ []byte) (int, error) { return 0, syscall.EPIPE }

func TestCliEmitterFinishBrokenPipeReturnsZero(t *testing.T) {
	// A broken pipe (reader hung up) collapses any success code to 0.
	if code := NewCliEmitter(epipeWriter{}, OutputFormatJson).FinishResult(map[string]any{"ok": true}); code != 0 {
		t.Fatalf("broken pipe FinishResult code = %d, want 0", code)
	}
	errEvent, _ := NewJSONError("boom", "it failed").Build()
	if code := NewCliEmitter(epipeWriter{}, OutputFormatJson).Finish(errEvent, 7); code != 0 {
		t.Fatalf("broken pipe Finish code = %d, want 0", code)
	}
}

func TestCliEmitterFinishOtherWriteFailureReturnsFour(t *testing.T) {
	// failingWriter returns a non-EPIPE error, so Finish maps it to 4.
	if code := NewCliEmitter(failingWriter{}, OutputFormatJson).FinishResult(map[string]any{"ok": true}); code != 4 {
		t.Fatalf("other write failure code = %d, want 4", code)
	}
}

// ═══════════════════════════════════════════
// Version helpers
// ═══════════════════════════════════════════

// versionValueFlags mirrors the example's own value-taking global flags, so the
// pre-parser recognizes their space-separated values.
var versionValueFlags = []string{"--log", "--stdout-file", "--stderr-file"}

func TestBuildCliVersion_StandardShape(t *testing.T) {
	v := BuildCliVersion("agent-cli", "Agent CLI Example", "1.2.3", "abc1234")
	if v["kind"] != "result" {
		t.Errorf("kind = %v", v["kind"])
	}
	result := v["result"].(map[string]any)
	if result["code"] != "version" {
		t.Errorf("code = %v", result["code"])
	}
	if result["name"] != "agent-cli" {
		t.Errorf("name = %v", result["name"])
	}
	if result["display_name"] != "Agent CLI Example" {
		t.Errorf("display_name = %v", result["display_name"])
	}
	if result["version"] != "1.2.3" {
		t.Errorf("version = %v", result["version"])
	}
	if result["build"] != "abc1234" {
		t.Errorf("build = %v", result["build"])
	}
	if trace, ok := v["trace"].(map[string]any); !ok || len(trace) != 0 {
		t.Errorf("trace = %v, want empty object", v["trace"])
	}
}

func TestBuildCliVersion_OmitsAbsentDisplayNameAndBuild(t *testing.T) {
	v := BuildCliVersion("agent-cli", "", "1.2.3", "")
	result := v["result"].(map[string]any)
	if result["name"] != "agent-cli" {
		t.Errorf("name = %v", result["name"])
	}
	if result["version"] != "1.2.3" {
		t.Errorf("version = %v", result["version"])
	}
	if _, ok := result["display_name"]; ok {
		t.Errorf("expected no display_name key, got %v", result["display_name"])
	}
	if _, ok := result["build"]; ok {
		t.Errorf("expected no build key, got %v", result["build"])
	}
}

func TestCliRenderVersion_Json(t *testing.T) {
	out := CliRenderVersion("agent-cli", "Agent CLI Example", "1.2.3", "abc1234", OutputFormatJson)
	var parsed map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &parsed); err != nil {
		t.Fatalf("version json must parse: %v (%q)", err, out)
	}
	result := parsed["result"].(map[string]any)
	if parsed["kind"] != "result" || result["code"] != "version" {
		t.Errorf("json version wrong shape: %s", out)
	}
	if result["name"] != "agent-cli" || result["version"] != "1.2.3" {
		t.Errorf("json version missing name/version: %s", out)
	}
	if result["display_name"] != "Agent CLI Example" || result["build"] != "abc1234" {
		t.Errorf("json version missing display_name/build: %s", out)
	}
}

func TestCliHandleVersionOrContinue_HonorsOutputFlag(t *testing.T) {
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--version", "--output", "plain"},
		versionValueFlags,
		"agent-cli", "Agent CLI Example", "1.2.3", "",
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
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
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
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
	)
	if !handled {
		t.Fatal("expected handled")
	}
	if err == nil || !contains(err.Error(), "conflicting output formats") {
		t.Fatalf("expected conflict error, got %v", err)
	}
}

func TestCliHandleVersionOrContinue_BareDefaultsToJson(t *testing.T) {
	// The one blessed behavior: bare --version always answers with a protocol-v1
	// event, JSON by default — no conventional bare-text special case.
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--version"},
		versionValueFlags,
		"agent-cli", "Agent CLI Example", "1.2.3", "",
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	var parsed map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &parsed); err != nil {
		t.Fatalf("bare --version must render json: %v (%q)", err, out)
	}
	result := parsed["result"].(map[string]any)
	if parsed["kind"] != "result" || result["code"] != "version" {
		t.Errorf("bare version wrong shape: %s", out)
	}
	if result["name"] != "agent-cli" || result["version"] != "1.2.3" {
		t.Errorf("bare version missing name/version: %s", out)
	}
	if result["display_name"] != "Agent CLI Example" {
		t.Errorf("bare version missing display_name: %s", out)
	}
}

func TestCliHandleVersionOrContinue_ReturnsNoneWithoutVersion(t *testing.T) {
	_, handled, err := CliHandleVersionOrContinue(
		[]string{"ping"},
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
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
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
	)
	if !handled {
		t.Fatal("expected handled")
	}
	if err == nil || !contains(err.Error(), "xml") {
		t.Fatalf("expected xml error, got %v", err)
	}
}

func TestCliHandleVersionOrContinue_IgnoresVersionFlagAfterSubcommand(t *testing.T) {
	// A subcommand that takes its own --version <value> must not be hijacked
	// by the top-level pre-parser.
	for _, args := range [][]string{
		{"hatch", "--version", "1.3.0"},
		{"hatch", "-V", "1.3.0"},
	} {
		_, handled, err := CliHandleVersionOrContinue(args, versionValueFlags, "agent-cli", "", "1.2.3", "")
		if err != nil {
			t.Fatalf("unexpected error for %v: %v", args, err)
		}
		if handled {
			t.Fatalf("expected handled=false for %v", args)
		}
	}
}

func TestCliHandleVersionOrContinue_HonorsOutputFlagBeforeTopLevelVersion(t *testing.T) {
	// A known output flag consumes its value, so a trailing top-level
	// --version is still recognized.
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--output", "json", "--version"},
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	if !contains(out, `"version":"1.2.3"`) {
		t.Fatalf("expected version json, got %q", out)
	}
}

func TestCliHandleVersionOrContinue_SkipsCallerDefinedValueFlag(t *testing.T) {
	// A caller's own value-taking global flag (here a comma-list --log) must have
	// its space-separated value recognized through valueFlags, not a hardcoded
	// list. Without that, "request,startup" would be mistaken for the subcommand
	// boundary and the trailing --version would be dropped.
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--log", "request,startup", "--version"},
		[]string{"--log"},
		"hypha", "", "1.2.3", "",
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	var parsed map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &parsed); err != nil {
		t.Fatalf("version json must parse: %v (%q)", err, out)
	}
	result := parsed["result"].(map[string]any)
	if result["name"] != "hypha" || result["version"] != "1.2.3" {
		t.Errorf("version missing name/version: %s", out)
	}
}

func TestCliHandleVersionOrContinue_SkipsOutputToSpaceValue(t *testing.T) {
	// A preceding --output-to <value> (space form) must not be mistaken for the
	// subcommand boundary; the trailing --version must still be recognized.
	out, handled, err := CliHandleVersionOrContinue(
		[]string{"--output-to", "stdout", "--version"},
		versionValueFlags,
		"agent-cli", "", "1.2.3", "",
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !handled {
		t.Fatal("expected handled")
	}
	if !contains(out, `"version":"1.2.3"`) {
		t.Fatalf("expected version json, got %q", out)
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
