package afdata

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
)

// ═══════════════════════════════════════════
// Public API: Reader (decode_protocol_event)
// ═══════════════════════════════════════════

// DecodedEvent is a base-validated protocol v1 event decoded from JSON text.
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
	Code    string
	Message string
	// Retryable is nil when the base-profile event omits it.
	Retryable *bool
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
	// Progress is the raw payload value (any JSON value).
	Progress any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

// DecodedLog is a decoded protocol v1 log event.
type DecodedLog struct {
	// Log is the raw payload value (any JSON value).
	Log any
	// Trace is the raw trace object, or nil when absent.
	Trace map[string]any
}

func (*DecodedResult) isDecodedEvent()   {}
func (*DecodedError) isDecodedEvent()    {}
func (*DecodedProgress) isDecodedEvent() {}
func (*DecodedLog) isDecodedEvent()      {}

// EventDecodeError reports why decode_protocol_event could not decode a line:
// either the text is not valid JSON, or it does not base-validate as a
// protocol v1 event.
type EventDecodeError struct {
	msg string
}

func (e *EventDecodeError) Error() string { return e.msg }

// DecodeProtocolEvent parses text as a single protocol v1 JSON line, base-
// validates it (see ValidateProtocolEvent with strict=false), and returns the
// typed decoded event. Returns *EventDecodeError on invalid JSON or a
// base-validation failure.
func DecodeProtocolEvent(text string) (DecodedEvent, error) {
	if err := checkProtocolLine(text); err != nil {
		return nil, &EventDecodeError{msg: fmt.Sprintf("invalid JSON: %v", err)}
	}
	var value any
	dec := json.NewDecoder(strings.NewReader(text))
	dec.UseNumber()
	if err := dec.Decode(&value); err != nil {
		return nil, &EventDecodeError{msg: fmt.Sprintf("invalid JSON: %v", err)}
	}
	var trailing any
	if err := dec.Decode(&trailing); err != io.EOF {
		if err == nil {
			return nil, &EventDecodeError{msg: "invalid JSON: trailing data after event"}
		}
		return nil, &EventDecodeError{msg: fmt.Sprintf("invalid JSON: trailing data: %v", err)}
	}

	if err := ValidateProtocolEvent(value, false); err != nil {
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
		var retryable *bool
		if value, ok := errorPayload["retryable"].(bool); ok {
			retryable = &value
		}
		return &DecodedError{
			Code:      errorPayload["code"].(string),
			Message:   errorPayload["message"].(string),
			Retryable: retryable,
			Hint:      hint,
			Fields:    fields,
			Trace:     trace,
		}, nil
	case "progress":
		return &DecodedProgress{
			Progress: obj["progress"],
			Trace:    trace,
		}, nil
	case "log":
		return &DecodedLog{
			Log:   obj["log"],
			Trace: trace,
		}, nil
	default:
		// Unreachable: ValidateProtocolEvent already rejected unknown kinds.
		return nil, &EventDecodeError{msg: fmt.Sprintf("unsupported event kind %q", obj["kind"])}
	}
}

// maxProtocolNesting is how many arrays and objects a protocol line may nest,
// counting the envelope object. Every SDK enforces the same bound.
const maxProtocolNesting = 127

// checkProtocolLine enforces the two decode rules encoding/json does not: a
// nesting bound, and no lone UTF-16 surrogate in a \u escape (which
// encoding/json would silently turn into U+FFFD). It only tracks strings and
// escapes; everything else about the grammar is left to the decoder.
func checkProtocolLine(text string) error {
	depth := 0
	inString := false
	for i := 0; i < len(text); i++ {
		c := text[i]
		if inString {
			switch c {
			case '"':
				inString = false
			case '\\':
				if i+1 < len(text) && text[i+1] == 'u' {
					unit, ok := hexEscape(text, i)
					if ok && unit >= 0xD800 && unit <= 0xDBFF {
						low, lowOK := hexEscape(text, i+6)
						if !lowOK || low < 0xDC00 || low > 0xDFFF {
							return fmt.Errorf("lone surrogate escape at byte %d", i)
						}
						i += 11
						continue
					}
					if ok && unit >= 0xDC00 && unit <= 0xDFFF {
						return fmt.Errorf("lone surrogate escape at byte %d", i)
					}
				}
				i++
			}
			continue
		}
		switch c {
		case '"':
			inString = true
		case '[', '{':
			depth++
			if depth > maxProtocolNesting {
				return fmt.Errorf("nesting deeper than %d", maxProtocolNesting)
			}
		case ']', '}':
			depth--
		}
	}
	return nil
}

// hexEscape reads the \uXXXX escape starting at text[at], if there is one.
func hexEscape(text string, at int) (uint16, bool) {
	if at+6 > len(text) || text[at] != '\\' || text[at+1] != 'u' {
		return 0, false
	}
	var unit uint16
	for _, c := range []byte(text[at+2 : at+6]) {
		var digit byte
		switch {
		case c >= '0' && c <= '9':
			digit = c - '0'
		case c >= 'a' && c <= 'f':
			digit = c - 'a' + 10
		case c >= 'A' && c <= 'F':
			digit = c - 'A' + 10
		default:
			return 0, false
		}
		unit = unit<<4 | uint16(digit)
	}
	return unit, true
}
