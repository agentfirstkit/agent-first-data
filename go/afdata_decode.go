package afdata

import (
	"encoding/json"
	"fmt"
	"strings"
)

// ═══════════════════════════════════════════
// Public API: Reader (decode_protocol_event)
// ═══════════════════════════════════════════

// DecodedEvent is a strict-validated protocol v1 event decoded from JSON text.
// It is a sealed interface: the only implementations are *DecodedResult,
// *DecodedError, *DecodedProgress, and *DecodedLog. Callers type-switch on the
// concrete type to read fields.
type DecodedEvent interface {
	isDecodedEvent()
}

// DecodedResult is a decoded protocol v1 result event.
type DecodedResult struct {
	// Result is the raw payload value (any JSON value).
	Result any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

// DecodedError is a decoded protocol v1 error event.
type DecodedError struct {
	Code      string
	Message   string
	Retryable bool
	// Hint is the optional hint string, or "" when absent.
	Hint string
	// Fields holds extension fields: error payload keys beyond code, message,
	// retryable, and hint.
	Fields map[string]any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

// DecodedProgress is a decoded protocol v1 progress event.
type DecodedProgress struct {
	Message string
	// Fields holds extension fields: progress payload keys beyond message.
	Fields map[string]any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

// DecodedLog is a decoded protocol v1 log event.
type DecodedLog struct {
	Level   LogLevel
	Message string
	// Fields holds extension fields: log payload keys beyond level and message.
	Fields map[string]any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

func (*DecodedResult) isDecodedEvent()   {}
func (*DecodedError) isDecodedEvent()    {}
func (*DecodedProgress) isDecodedEvent() {}
func (*DecodedLog) isDecodedEvent()      {}

// EventDecodeError reports why decode_protocol_event could not decode a line:
// either the text is not valid JSON, or it does not strict-validate as a
// protocol v1 event.
type EventDecodeError struct {
	msg string
}

func (e *EventDecodeError) Error() string { return e.msg }

// DecodeProtocolEvent parses text as a single protocol v1 JSON line, strict-
// validates it (see ValidateProtocolEvent with strict=true), and returns the
// typed decoded event. Returns *EventDecodeError on invalid JSON or a
// strict-validation failure.
func DecodeProtocolEvent(text string) (DecodedEvent, error) {
	var value any
	dec := json.NewDecoder(strings.NewReader(text))
	dec.UseNumber()
	if err := dec.Decode(&value); err != nil {
		return nil, &EventDecodeError{msg: fmt.Sprintf("invalid JSON: %v", err)}
	}

	if err := ValidateProtocolEvent(value, true); err != nil {
		return nil, &EventDecodeError{msg: err.Error()}
	}

	obj := value.(map[string]any)
	trace, _ := obj["trace"].(map[string]any)
	switch obj["kind"].(string) {
	case "result":
		return &DecodedResult{Result: obj["result"], Trace: trace}, nil
	case "error":
		errorPayload := obj["error"].(map[string]any)
		hint, _ := errorPayload["hint"].(string)
		fields := make(map[string]any, len(errorPayload))
		for k, v := range errorPayload {
			if k != "code" && k != "message" && k != "retryable" && k != "hint" {
				fields[k] = v
			}
		}
		return &DecodedError{
			Code:      errorPayload["code"].(string),
			Message:   errorPayload["message"].(string),
			Retryable: errorPayload["retryable"].(bool),
			Hint:      hint,
			Fields:    fields,
			Trace:     trace,
		}, nil
	case "progress":
		progressPayload := obj["progress"].(map[string]any)
		fields := make(map[string]any, len(progressPayload))
		for k, v := range progressPayload {
			if k != "message" {
				fields[k] = v
			}
		}
		return &DecodedProgress{
			Message: progressPayload["message"].(string),
			Fields:  fields,
			Trace:   trace,
		}, nil
	case "log":
		logPayload := obj["log"].(map[string]any)
		fields := make(map[string]any, len(logPayload))
		for k, v := range logPayload {
			if k != "level" && k != "message" {
				fields[k] = v
			}
		}
		return &DecodedLog{
			Level:   LogLevel(logPayload["level"].(string)),
			Message: logPayload["message"].(string),
			Fields:  fields,
			Trace:   trace,
		}, nil
	default:
		// Unreachable: ValidateProtocolEvent already rejected unknown kinds.
		return nil, &EventDecodeError{msg: fmt.Sprintf("unsupported event kind %q", obj["kind"])}
	}
}
