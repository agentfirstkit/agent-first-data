package afdata

import (
	"encoding/json"
	"testing"
)

// ═══════════════════════════════════════════
// Fixture-driven builder tests
// ═══════════════════════════════════════════

func TestBuildJSONResult(t *testing.T) {
	result := map[string]any{"hash": "abc"}
	event, err := NewJSONResult(result).Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	if envelope["kind"] != "result" {
		t.Errorf("kind = %v, want 'result'", envelope["kind"])
	}
	if !deepEqual(envelope["result"], result) {
		t.Errorf("result mismatch: %v", envelope["result"])
	}
	if trace, ok := envelope["trace"].(map[string]any); !ok || len(trace) != 0 {
		t.Errorf("trace = %v, want empty object", envelope["trace"])
	}
}

func TestBuildJSONResultWithTrace(t *testing.T) {
	result := map[string]any{"hash": "abc"}
	traceData := map[string]any{"duration_ms": float64(12)}
	builder := NewJSONResult(result).Trace(traceData)
	event, err := builder.Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	if !deepEqual(envelope["trace"], traceData) {
		t.Errorf("trace mismatch: %v", envelope["trace"])
	}
}

func TestBuildJSONError(t *testing.T) {
	event, err := NewJSONError("not_found", "not found").Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	if envelope["kind"] != "error" {
		t.Errorf("kind = %v, want 'error'", envelope["kind"])
	}
	errPayload := envelope["error"].(map[string]any)
	if errPayload["code"] != "not_found" {
		t.Errorf("error.code = %v", errPayload["code"])
	}
	if errPayload["message"] != "not found" {
		t.Errorf("error.message = %v", errPayload["message"])
	}
	if errPayload["retryable"] != false {
		t.Errorf("error.retryable = %v, want false", errPayload["retryable"])
	}
}

func TestBuildJSONErrorWithHint(t *testing.T) {
	builder := NewJSONError("timeout", "connection timeout").Hint("increase --timeout-s")
	event, err := builder.Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	errPayload := envelope["error"].(map[string]any)
	if errPayload["hint"] != "increase --timeout-s" {
		t.Errorf("hint = %v", errPayload["hint"])
	}
}

func TestBuildJSONErrorRetryable(t *testing.T) {
	builder := NewJSONError("network_error", "connection failed").Retryable()
	event, err := builder.Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	errPayload := envelope["error"].(map[string]any)
	if errPayload["retryable"] != true {
		t.Errorf("retryable = %v, want true", errPayload["retryable"])
	}
}

func TestBuildJSONErrorRetryableIf(t *testing.T) {
	cases := []bool{true, false}
	for _, shouldRetry := range cases {
		builder := NewJSONError("error", "msg").RetryableIf(shouldRetry)
		event, err := builder.Build()
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		envelope := event.Value()
		errPayload := envelope["error"].(map[string]any)
		if errPayload["retryable"] != shouldRetry {
			t.Errorf("retryable = %v, want %v", errPayload["retryable"], shouldRetry)
		}
	}
}

func TestBuildJSONErrorWithFields(t *testing.T) {
	builder := NewJSONError("validation_failed", "invalid input").
		Field("field", "email").
		Field("rule_id", "format")
	event, err := builder.Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	errPayload := envelope["error"].(map[string]any)
	if errPayload["field"] != "email" || errPayload["rule_id"] != "format" {
		t.Errorf("fields not merged correctly: %v", errPayload)
	}
}

func TestBuildJSONErrorRejectsReservedField(t *testing.T) {
	builder := NewJSONError("error", "msg").Field("code", "override")
	_, err := builder.Build()
	if err == nil || !contains(err.Error(), "reserved") {
		t.Fatalf("expected reserved field error, got %v", err)
	}
}

func TestBuildJSONProgress(t *testing.T) {
	event, err := NewJSONProgress("working").Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	if envelope["kind"] != "progress" {
		t.Errorf("kind = %v, want 'progress'", envelope["kind"])
	}
	progressPayload := envelope["progress"].(map[string]any)
	if progressPayload["message"] != "working" {
		t.Errorf("message = %v", progressPayload["message"])
	}
}

func TestBuildJSONProgressEmptyMessage(t *testing.T) {
	_, err := NewJSONProgress("").Build()
	if err == nil || !contains(err.Error(), "must be non-empty") {
		t.Fatalf("expected non-empty message error, got %v", err)
	}
}

func TestBuildJSONLog(t *testing.T) {
	event, err := NewJSONLog(LogLevelInfo, "request started").Build()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	if envelope["kind"] != "log" {
		t.Errorf("kind = %v, want 'log'", envelope["kind"])
	}
	logPayload := envelope["log"].(map[string]any)
	if logPayload["level"] != "info" {
		t.Errorf("level = %v", logPayload["level"])
	}
	if logPayload["message"] != "request started" {
		t.Errorf("message = %v", logPayload["message"])
	}
	// 0.16: log payload must NOT contain 'code'
	if _, hasCode := logPayload["code"]; hasCode {
		t.Errorf("log payload should not contain 'code' field")
	}
}

func TestBuildJSONLogAllLevels(t *testing.T) {
	levels := []LogLevel{LogLevelDebug, LogLevelInfo, LogLevelWarn, LogLevelError}
	for _, level := range levels {
		event, err := NewJSONLog(level, "msg").Build()
		if err != nil {
			t.Fatalf("level %v: unexpected error: %v", level, err)
		}
		envelope := event.Value()
		logPayload := envelope["log"].(map[string]any)
		if logPayload["level"] != string(level) {
			t.Errorf("level = %v, want %v", logPayload["level"], level)
		}
	}
}

func TestBuildJSONLogInvalidLevel(t *testing.T) {
	_, err := NewJSONLog(LogLevel("invalid"), "msg").Build()
	if err == nil || !contains(err.Error(), "invalid") {
		t.Fatalf("expected invalid level error, got %v", err)
	}
}

func TestBuildCLIError(t *testing.T) {
	event, err := BuildCLIError("missing flag", "try --help")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	envelope := event.Value()
	errPayload := envelope["error"].(map[string]any)
	if errPayload["code"] != "cli_error" {
		t.Errorf("code = %v, want cli_error", errPayload["code"])
	}
	if errPayload["message"] != "missing flag" {
		t.Errorf("message = %v", errPayload["message"])
	}
	if errPayload["hint"] != "try --help" {
		t.Errorf("hint = %v", errPayload["hint"])
	}
}

func TestEventMarshalJSON(t *testing.T) {
	event, _ := NewJSONError("test_error", "test message").Build()
	data, err := json.Marshal(event)
	if err != nil {
		t.Fatalf("MarshalJSON failed: %v", err)
	}
	var envelope map[string]any
	if err := json.Unmarshal(data, &envelope); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if envelope["kind"] != "error" {
		t.Errorf("kind = %v", envelope["kind"])
	}
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

func deepEqual(a, b any) bool {
	// Use JSON marshaling for deep comparison
	aJSON, _ := json.Marshal(a)
	bJSON, _ := json.Marshal(b)
	return string(aJSON) == string(bJSON)
}
