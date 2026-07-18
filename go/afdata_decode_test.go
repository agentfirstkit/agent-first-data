package afdata

import (
	"encoding/json"
	"testing"
)

func TestDecodeProtocolEventResult(t *testing.T) {
	event := NewJSONResult(map[string]any{"hash": "abc"}).Trace(map[string]any{"duration_ms": 12}).Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	result, ok := decoded.(*DecodedResult)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedResult", decoded)
	}
	resultMap, ok := result.Result.(map[string]any)
	if !ok || resultMap["hash"] != "abc" {
		t.Errorf("result = %v", result.Result)
	}
	if result.Trace["duration_ms"] == nil {
		t.Errorf("trace missing duration_ms: %v", result.Trace)
	}
}

func TestDecodeProtocolEventError(t *testing.T) {
	event, _ := NewJSONError("not_found", "not found").
		Hint("try again").
		Retryable().
		Field("field", "email").
		Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	errEvent, ok := decoded.(*DecodedError)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedError", decoded)
	}
	if errEvent.Code != "not_found" {
		t.Errorf("code = %v", errEvent.Code)
	}
	if errEvent.Message != "not found" {
		t.Errorf("message = %v", errEvent.Message)
	}
	if !errEvent.Retryable {
		t.Errorf("retryable = %v, want true", errEvent.Retryable)
	}
	if errEvent.Hint != "try again" {
		t.Errorf("hint = %v", errEvent.Hint)
	}
	if errEvent.Fields["field"] != "email" {
		t.Errorf("fields[field] = %v, want email", errEvent.Fields["field"])
	}
	if _, ok := errEvent.Fields["code"]; ok {
		t.Errorf("fields should not contain reserved key 'code'")
	}
}

func TestDecodeProtocolEventErrorWithoutHint(t *testing.T) {
	event, _ := NewJSONError("failed", "oops").Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	errEvent := decoded.(*DecodedError)
	if errEvent.Hint != "" {
		t.Errorf("hint = %q, want empty", errEvent.Hint)
	}
	if errEvent.Retryable {
		t.Errorf("retryable = true, want false")
	}
}

func TestDecodeProtocolEventProgress(t *testing.T) {
	event := NewJSONProgress(map[string]any{"message": "working", "percent": float64(50)}).Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	progress, ok := decoded.(*DecodedProgress)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedProgress", decoded)
	}
	payload := progress.Progress.(map[string]any)
	if payload["message"] != "working" || payload["percent"] == nil {
		t.Errorf("unexpected progress payload: %v", payload)
	}
}

func TestDecodeProtocolEventLog(t *testing.T) {
	event := NewJSONLog(map[string]any{"level": "warn", "message": "disk low", "free_bytes": float64(1024)}).Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	log, ok := decoded.(*DecodedLog)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedLog", decoded)
	}
	payload := log.Log.(map[string]any)
	if payload["level"] != "warn" || payload["message"] != "disk low" || payload["free_bytes"] == nil {
		t.Errorf("unexpected log payload: %v", payload)
	}
}

func TestDecodeProtocolEventInvalidJSON(t *testing.T) {
	_, err := DecodeProtocolEvent("not json")
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
	var decodeErr *EventDecodeError
	if !asEventDecodeError(err, &decodeErr) {
		t.Fatalf("expected *EventDecodeError, got %T", err)
	}
}

func TestDecodeProtocolEventFailsStrictValidation(t *testing.T) {
	// Missing trace fails the strict profile even though it is a well-formed result envelope.
	_, err := DecodeProtocolEvent(`{"kind":"result","result":{}}`)
	if err == nil {
		t.Fatal("expected strict validation error for missing trace")
	}
}

func TestDecodeProtocolEventRejectsUnknownKind(t *testing.T) {
	_, err := DecodeProtocolEvent(`{"kind":"bogus","bogus":{},"trace":{}}`)
	if err == nil {
		t.Fatal("expected error for unknown kind")
	}
}

func asEventDecodeError(err error, target **EventDecodeError) bool {
	if e, ok := err.(*EventDecodeError); ok {
		*target = e
		return true
	}
	return false
}

// TestNumberFidelityFixtures drives spec/fixtures/number_fidelity.json
// (shared across all four SDKs). DecodeProtocolEvent already used
// dec.UseNumber() before this phase; this suite is the regression guard for
// that plus the marshalToObject fix in afdata.go (builder Trace/Extend/error
// Build no longer round-trip a caller-supplied struct through
// json.Unmarshal into map[string]any without UseNumber, which silently
// collapsed >2^53 struct fields to float64).
func TestNumberFidelityFixtures(t *testing.T) {
	for _, tc := range loadFixture("number_fidelity.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			inputLine := tc["input_line"].(string)
			expectedJSON := tc["expected_json"].(string)

			decoded, err := DecodeProtocolEvent(inputLine)
			if err != nil {
				t.Fatalf("decode failed: %v", err)
			}
			result, ok := decoded.(*DecodedResult)
			if !ok {
				t.Fatalf("decoded type = %T, want *DecodedResult", decoded)
			}

			gotJSON := Render(result.Result, OutputFormatJson, OutputOptions{})
			if gotJSON != expectedJSON {
				t.Errorf("json got %q, want %q", gotJSON, expectedJSON)
			}

			if expectedYAML, ok := tc["expected_yaml"].(string); ok {
				gotYAML := Render(result.Result, OutputFormatYaml, OutputOptions{})
				if gotYAML != expectedYAML {
					t.Errorf("yaml got %q, want %q", gotYAML, expectedYAML)
				}
			}
		})
	}
}

// TestNumberFidelityDoesNotRegressOrdinaryDecodedNumbersInPlainOutput guards
// the pre-existing (not newly introduced) json.Number handling in
// asInt64/asFloat64/yamlScalar/plainScalar: DecodeProtocolEvent wraps every
// decoded number, including small ordinary ones, in json.Number, so Plain's
// suffix arithmetic must keep working for a routine decoded event.
func TestNumberFidelityDoesNotRegressOrdinaryDecodedNumbersInPlainOutput(t *testing.T) {
	line := `{"kind":"result","result":{"duration_ms":42,"size_bytes":5242880,"cpu_percent":85.5},"trace":{}}`
	decoded, err := DecodeProtocolEvent(line)
	if err != nil {
		t.Fatalf("decode failed: %v", err)
	}
	result := decoded.(*DecodedResult).Result
	got := Render(result, OutputFormatPlain, OutputOptions{})
	want := "cpu=85.5% duration=42ms size=5.0MiB"
	if got != want {
		t.Errorf("plain got %q, want %q", got, want)
	}
}
