package afdata

import (
	"encoding/json"
	"testing"
)

func TestDecodeProtocolEventResult(t *testing.T) {
	event, _ := NewJSONResult(map[string]any{"hash": "abc"}).Trace(map[string]any{"duration_ms": 12}).Build()
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
	event, _ := NewJSONProgress("working").Field("percent", float64(50)).Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	progress, ok := decoded.(*DecodedProgress)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedProgress", decoded)
	}
	if progress.Message != "working" {
		t.Errorf("message = %v", progress.Message)
	}
	if progress.Fields["percent"] == nil {
		t.Errorf("fields missing percent: %v", progress.Fields)
	}
	if _, ok := progress.Fields["message"]; ok {
		t.Errorf("fields should not contain reserved key 'message'")
	}
}

func TestDecodeProtocolEventLog(t *testing.T) {
	event, _ := NewJSONLog(LogLevelWarn, "disk low").Field("free_bytes", float64(1024)).Build()
	line, _ := json.Marshal(event)

	decoded, err := DecodeProtocolEvent(string(line))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	log, ok := decoded.(*DecodedLog)
	if !ok {
		t.Fatalf("decoded type = %T, want *DecodedLog", decoded)
	}
	if log.Level != LogLevelWarn {
		t.Errorf("level = %v", log.Level)
	}
	if log.Message != "disk low" {
		t.Errorf("message = %v", log.Message)
	}
	if log.Fields["free_bytes"] == nil {
		t.Errorf("fields missing free_bytes: %v", log.Fields)
	}
	if _, ok := log.Fields["level"]; ok {
		t.Errorf("fields should not contain reserved key 'level'")
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
