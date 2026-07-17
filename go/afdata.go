// Package afdata implements Agent-First Data (AFDATA) output formatting
// and protocol templates.
//
// Public APIs include protocol v1 builders + 2 value-copy redactors (redact
// _secret and _url fields) + 2 URL-string redactors (operate on one URL
// string; the value redactors apply these to _url fields) + the Render output
// entry point + 4 utilities + CLI helpers + OutputFormat + RedactionPolicy +
// Redactor + PlainStyle + OutputOptions + LogFilters.
package afdata

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"net/url"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf16"
)

const maxSafeInteger = uint64(9007199254740991)

// LogLevel represents the severity level for log events.
type LogLevel string

const (
	LogLevelDebug LogLevel = "debug"
	LogLevelInfo  LogLevel = "info"
	LogLevelWarn  LogLevel = "warn"
	LogLevelError LogLevel = "error"
)

// Event represents a protocol v1 event envelope with strict construction.
// Events are opaque and strict-valid by construction.
type Event struct {
	envelope map[string]any
}

// BuilderError represents an error encountered during builder construction,
// with structured details about what validation failed.
type BuilderError struct {
	msg string
	// Can be expanded with structured fields for errors.As() matching
}

func (e *BuilderError) Error() string {
	return e.msg
}

// ═══════════════════════════════════════════
// Public API: Protocol v1 Builders and Validation
// ═══════════════════════════════════════════

// MarshalJSON implements json.Marshaler, allowing Event to be serialized as JSON.
func (e Event) MarshalJSON() ([]byte, error) {
	return json.Marshal(e.envelope)
}

// Value returns the underlying envelope as a map for introspection.
// Callers should not modify the returned map.
func (e Event) Value() map[string]any {
	return e.envelope
}

// BuildCLIError builds a standard CLI error with code "cli_error".
// Pass empty string for hint to omit it. Returns a strict-ready Event.
func BuildCLIError(message string, hint string) (Event, error) {
	builder := NewJSONError("cli_error", message)
	if hint != "" {
		builder.Hint(hint)
	}
	return builder.Build()
}

// ═════════════════════════════════════════════
// JSON Result Builder
// ═════════════════════════════════════════════

// JSONResultBuilder constructs result events with optional trace.
type JSONResultBuilder struct {
	result any
	trace  any
}

// NewJSONResult creates a builder for a result event.
func NewJSONResult(result any) *JSONResultBuilder {
	return &JSONResultBuilder{result: result}
}

// Trace sets the trace object (must be or produce a JSON object).
func (b *JSONResultBuilder) Trace(trace any) *JSONResultBuilder {
	b.trace = trace
	return b
}

// Build constructs the Event. This builder cannot fail.
func (b *JSONResultBuilder) Build() Event {
	envelope := map[string]any{
		"kind":   "result",
		"result": b.result,
		"trace":  normalizeTrace(b.trace),
	}
	return Event{envelope: envelope}
}

// normalizeTrace coerces a builder trace into an object when it round-trips
// through JSON as one, and otherwise returns it unchanged. It never fails: an
// invalid (non-object) trace surfaces later at ValidateProtocolEvent, not at
// Build time. A nil trace becomes an empty object.
func normalizeTrace(trace any) any {
	if trace == nil {
		return map[string]any{}
	}
	if obj, ok := trace.(map[string]any); ok {
		return obj
	}
	data, err := json.Marshal(trace)
	if err != nil {
		return trace
	}
	var obj map[string]any
	if json.Unmarshal(data, &obj) == nil {
		return obj
	}
	return trace
}

// ═════════════════════════════════════════════
// JSON Error V1 Builder
// ═════════════════════════════════════════════

// JSONErrorBuilder constructs error events with optional hint, retryable, fields, and trace.
type JSONErrorBuilder struct {
	code      string
	message   string
	retryable bool
	hint      string
	fields    map[string]any
	trace     any
	errs      []error
	fieldErrs []string
}

// NewJSONError creates a builder for an error event.
// code and message must be non-empty strings.
func NewJSONError(code string, message string) *JSONErrorBuilder {
	b := &JSONErrorBuilder{
		code:      code,
		message:   message,
		retryable: false,
		fields:    make(map[string]any),
	}
	if code == "" {
		b.errs = append(b.errs, &BuilderError{msg: "error code must be non-empty"})
	}
	if message == "" {
		b.errs = append(b.errs, &BuilderError{msg: "error message must be non-empty"})
	}
	return b
}

// Retryable marks the error as retryable (true).
func (b *JSONErrorBuilder) Retryable() *JSONErrorBuilder {
	b.retryable = true
	return b
}

// RetryableIf marks the error as retryable based on a boolean condition.
func (b *JSONErrorBuilder) RetryableIf(flag bool) *JSONErrorBuilder {
	b.retryable = flag
	return b
}

// Hint sets the optional hint string.
func (b *JSONErrorBuilder) Hint(text string) *JSONErrorBuilder {
	b.hint = text
	return b
}

// Field adds a single extension field.
// Reserved field names (code, message, hint, retryable) are recorded as errors,
// returned at Build() time.
func (b *JSONErrorBuilder) Field(name string, value any) *JSONErrorBuilder {
	if isReservedErrorField(name) {
		b.fieldErrs = append(b.fieldErrs, fmt.Sprintf("field %q is reserved", name))
		return b
	}
	b.fields[name] = value
	return b
}

// Fields merges extension fields from a map.
// Reserved field names are skipped with an error recorded.
func (b *JSONErrorBuilder) Fields(obj map[string]any) *JSONErrorBuilder {
	if obj == nil {
		return b
	}
	for k, v := range obj {
		b.Field(k, v)
	}
	return b
}

// Extend merges fields from a struct (serialized via json tags) or map.
// Reserved field names are rejected with an error recorded.
func (b *JSONErrorBuilder) Extend(value any) *JSONErrorBuilder {
	if value == nil {
		return b
	}
	data, err := json.Marshal(value)
	if err != nil {
		b.errs = append(b.errs, &BuilderError{msg: "extend: failed to marshal value: " + err.Error()})
		return b
	}
	var obj map[string]any
	if err := json.Unmarshal(data, &obj); err != nil {
		b.errs = append(b.errs, &BuilderError{msg: "extend: value must serialize to a JSON object"})
		return b
	}
	b.Fields(obj)
	return b
}

// Trace sets the trace object.
func (b *JSONErrorBuilder) Trace(trace any) *JSONErrorBuilder {
	b.trace = trace
	return b
}

// Build constructs the Event, performing strict validation.
func (b *JSONErrorBuilder) Build() (Event, error) {
	if len(b.errs) > 0 {
		return Event{}, b.errs[0]
	}
	if len(b.fieldErrs) > 0 {
		return Event{}, &BuilderError{msg: b.fieldErrs[0]}
	}

	errorPayload := make(map[string]any)
	for k, v := range b.fields {
		errorPayload[k] = v
	}
	errorPayload["code"] = b.code
	errorPayload["message"] = b.message
	errorPayload["retryable"] = b.retryable
	if b.hint != "" {
		errorPayload["hint"] = b.hint
	}

	traceObj := map[string]any{}
	if b.trace != nil {
		if obj, ok := b.trace.(map[string]any); ok {
			traceObj = obj
		} else {
			data, err := json.Marshal(b.trace)
			if err != nil {
				return Event{}, &BuilderError{msg: "trace must be serializable: " + err.Error()}
			}
			if err := json.Unmarshal(data, &traceObj); err != nil {
				return Event{}, &BuilderError{msg: "trace must be a JSON object"}
			}
		}
	}

	envelope := map[string]any{
		"kind":  "error",
		"error": errorPayload,
		"trace": traceObj,
	}
	return Event{envelope: envelope}, nil
}

func isReservedErrorField(name string) bool {
	return name == "code" || name == "message" || name == "hint" || name == "retryable"
}

// ═════════════════════════════════════════════
// JSON Progress Builder
// ═════════════════════════════════════════════

// JSONProgressBuilder constructs progress events from a tool-defined payload and optional trace.
type JSONProgressBuilder struct {
	payload any
	trace   any
}

// NewJSONProgress creates a builder for a progress event.
func NewJSONProgress(payload any) *JSONProgressBuilder {
	return &JSONProgressBuilder{payload: payload}
}

// Trace sets the trace object.
func (b *JSONProgressBuilder) Trace(trace any) *JSONProgressBuilder {
	b.trace = trace
	return b
}

// Build constructs the Event. This builder cannot fail.
func (b *JSONProgressBuilder) Build() Event {
	envelope := map[string]any{
		"kind":     "progress",
		"progress": b.payload,
		"trace":    normalizeTrace(b.trace),
	}
	return Event{envelope: envelope}
}

// ═════════════════════════════════════════════
// JSON Log Builder
// ═════════════════════════════════════════════

// JSONLogBuilder constructs log events from a tool-defined payload and optional trace.
type JSONLogBuilder struct {
	payload any
	trace   any
}

// NewJSONLog creates a builder for a log event.
func NewJSONLog(payload any) *JSONLogBuilder {
	return &JSONLogBuilder{payload: payload}
}

// Trace sets the trace object.
func (b *JSONLogBuilder) Trace(trace any) *JSONLogBuilder {
	b.trace = trace
	return b
}

// Build constructs the Event. This builder cannot fail.
func (b *JSONLogBuilder) Build() Event {
	envelope := map[string]any{
		"kind":  "log",
		"log":   b.payload,
		"trace": normalizeTrace(b.trace),
	}
	return Event{envelope: envelope}
}

// ValidateProtocolEvent validates one protocol v1 event envelope. When strict
// is true, it additionally applies the recommended strict profile: a required
// trace object and event.error.retryable required and boolean.
func ValidateProtocolEvent(event any, strict bool) error {
	obj, ok := event.(map[string]any)
	if !ok {
		return fmt.Errorf("event must be a JSON object")
	}
	kind, ok := obj["kind"].(string)
	if !ok {
		return fmt.Errorf("event.kind must be one of result, error, progress, log")
	}
	switch kind {
	case "result", "error", "progress", "log":
	default:
		return fmt.Errorf("unsupported event kind %q", kind)
	}
	if _, ok := obj[kind]; !ok {
		return fmt.Errorf("event payload field %q is required", kind)
	}
	for key := range obj {
		if key != "kind" && key != kind && key != "trace" {
			return fmt.Errorf("unexpected top-level field %q", key)
		}
	}
	trace, hasTrace := obj["trace"]
	if hasTrace {
		if _, ok := trace.(map[string]any); !ok {
			return fmt.Errorf("event.trace must be a JSON object when present")
		}
	}
	if strict && !hasTrace {
		return fmt.Errorf("event.trace is required by the strict profile")
	}
	if kind == "error" {
		if err := validateErrorPayload(obj["error"]); err != nil {
			return err
		}
	}
	if !strict {
		return nil
	}
	switch kind {
	case "error":
		return validateStrictErrorPayload(obj["error"])
	default:
		return nil
	}
}

func validateErrorPayload(value any) error {
	errorPayload, ok := value.(map[string]any)
	if !ok {
		return fmt.Errorf("event.error must be a JSON object")
	}
	code, ok := errorPayload["code"].(string)
	if !ok || code == "" {
		return fmt.Errorf("event.error.code must be a non-empty string")
	}
	message, ok := errorPayload["message"].(string)
	if !ok || message == "" {
		return fmt.Errorf("event.error.message must be a non-empty string")
	}
	if hint, ok := errorPayload["hint"]; ok {
		if _, ok := hint.(string); !ok {
			return fmt.Errorf("event.error.hint must be a string when present")
		}
	}
	return nil
}

// ValidateProtocolStream validates a finite structured CLI event stream:
// (log | progress)* -> exactly one (result | error) -> end. When strict is
// true, every event is additionally validated against the strict profile
// (see ValidateProtocolEvent).
func ValidateProtocolStream(events []any, strict bool) error {
	terminalSeen := false
	for idx, event := range events {
		if err := ValidateProtocolEvent(event, strict); err != nil {
			return fmt.Errorf("event %d: %w", idx, err)
		}
		kind := event.(map[string]any)["kind"].(string)
		switch kind {
		case "log", "progress":
			if terminalSeen {
				return fmt.Errorf("event %d: non-terminal event after terminal", idx)
			}
		case "result", "error":
			if terminalSeen {
				return fmt.Errorf("event %d: duplicate terminal event", idx)
			}
			terminalSeen = true
		default:
			return fmt.Errorf("event %d: unsupported event kind %q", idx, kind)
		}
	}
	if !terminalSeen {
		return fmt.Errorf("event stream must contain exactly one terminal result or error")
	}
	return nil
}

func validateStrictErrorPayload(value any) error {
	errorPayload, ok := value.(map[string]any)
	if !ok {
		return fmt.Errorf("event.error must be a JSON object")
	}
	// Validate retryable is present and boolean
	retryable, ok := errorPayload["retryable"]
	if !ok {
		return fmt.Errorf("event.error.retryable is required by the strict profile")
	}
	if _, ok := retryable.(bool); !ok {
		return fmt.Errorf("event.error.retryable must be a boolean")
	}
	return nil
}

func isNonNegativeInteger(value any) bool {
	v := reflect.ValueOf(value)
	if !v.IsValid() {
		return false
	}
	switch v.Kind() {
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return v.Int() >= 0
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return true
	case reflect.Float32, reflect.Float64:
		f := v.Float()
		return f >= 0 && math.Trunc(f) == f
	default:
		return false
	}
}

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

// RedactionPolicy controls scoped redaction behavior for Render.
type RedactionPolicy string

const (
	// RedactionAll redacts every secret field anywhere in the value (the default;
	// also the zero value "" of RedactionPolicy behaves as All).
	RedactionAll RedactionPolicy = "All"
	// RedactionTraceOnly redacts only inside the top-level trace object.
	RedactionTraceOnly RedactionPolicy = "TraceOnly"
	// RedactionOff redacts nothing.
	RedactionOff RedactionPolicy = "Off"
)

// Redactor configures scoped redaction and extra secret field names.
// The zero value applies default full redaction with no extra names.
type Redactor struct {
	// SecretNames are field names to redact in addition to _secret suffixes.
	// Matching is exact field-name equality at any nesting level. The same list
	// also matches URL query-parameter names inside _url fields (see URL).
	SecretNames []string
	// Policy controls where redaction is applied. Empty means default full redaction.
	Policy RedactionPolicy
}

// Value returns a JSON-safe copy of v with the redactor's redaction applied.
func (r Redactor) Value(v any) any {
	sanitized := sanitizeForJSON(v)
	context := newRedactionContext(r)
	return applyRedactionPolicyWithContext(sanitized, r.Policy, context)
}

// URL redacts secret components of a single URL string.
//
// A query parameter is redacted iff its (form-decoded) name ends in
// _secret/_SECRET or matches an exact entry in SecretNames. The userinfo
// password (scheme://user:pass@host) is always redacted as a structural rule.
// Only the secret spans are replaced with "***"; every other byte is preserved.
// A string that is not a single, whitespace-free, scheme-prefixed URL (including
// a URL embedded in surrounding prose) is returned unchanged.
func (r Redactor) URL(rawURL string) string {
	context := newRedactionContext(r)
	if redacted, ok := redactURLInStr(rawURL, context); ok {
		return redacted
	}
	return rawURL
}

// PlainStyle controls plain (logfmt) rendering style. It affects plain output
// ONLY; JSON and YAML are structure-preserving and ignore it.
type PlainStyle string

const (
	// PlainStyleReadable strips AFDATA suffixes and formats values.
	PlainStyleReadable PlainStyle = "Readable"
	// PlainStyleRaw preserves keys and values after redaction.
	PlainStyleRaw PlainStyle = "Raw"
)

// OutputOptions combines redaction and rendering style.
type OutputOptions struct {
	Redaction Redactor
	Style     PlainStyle
}

// OutputOptionsForPolicy returns OutputOptions with the given redaction policy
// and the default (Readable) style.
func OutputOptionsForPolicy(p RedactionPolicy) OutputOptions {
	return OutputOptions{Redaction: Redactor{Policy: p}}
}

// renderJSON formats value as single-line JSON with the given output options.
// JSON ignores PlainStyle and preserves original keys and values after redaction.
func renderJSON(value any, options OutputOptions) string {
	return marshalOutputJSON(options.Redaction.Value(value))
}

func marshalOutputJSON(value any) string {
	out, err := json.Marshal(value)
	if err != nil {
		// Last-resort fallback: preserve JSONL contract even for pathological inputs.
		fallback, _ := json.Marshal(map[string]any{
			"error":  "output_json_failed",
			"detail": err.Error(),
		})
		return string(fallback)
	}
	return string(out)
}

// renderYaml formats value as multi-line YAML with the given output options.
// Structure-preserving: like JSON, original keys, scalar types, and numeric
// semantics are kept after redaction; secrets are still redacted. YAML output
// ignores PlainStyle.
func renderYaml(value any, options OutputOptions) string {
	lines := []string{"---"}
	v := options.Redaction.Value(value)
	renderYamlRaw(v, 0, &lines)
	return strings.Join(lines, "\n")
}

// renderPlain formats value as single-line logfmt with the given output
// options. Keys are stripped and values formatted unless PlainStyleRaw is
// set; secrets are always redacted.
func renderPlain(value any, options OutputOptions) string {
	var pairs [][2]string
	v := options.Redaction.Value(value)
	if options.Style == PlainStyleRaw {
		collectPlainPairsRaw(v, "", &pairs)
	} else {
		collectPlainPairs(v, "", &pairs)
	}
	sort.Slice(pairs, func(i, j int) bool {
		return jcsLess(pairs[i][0], pairs[j][0])
	})
	parts := make([]string, len(pairs))
	for i, p := range pairs {
		parts[i] = fmt.Sprintf("%s=%s", quoteLogfmtKey(p[0]), quoteLogfmtValue(p[1]))
	}
	return strings.Join(parts, " ")
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

// RedactedValue returns a JSON-safe copy with default _secret redaction applied.
// For scoped redaction or extra secret names, use Redactor.Value.
func RedactedValue(value any) any {
	return Redactor{}.Value(value)
}

// RedactURLSecrets redacts secret components of a single URL string, using
// default options. Returns url with its userinfo password and any
// _secret-suffixed query parameter values replaced by "***".
// For extra secret names, use Redactor.URL.
func RedactURLSecrets(rawURL string) string {
	return Redactor{}.URL(rawURL)
}

// NormalizeUTCOffset normalizes a fixed UTC offset string to AFDATA canonical form.
// It returns "UTC" for zero offset, or ±HH:MM for non-zero offsets. This helper
// handles fixed offsets only; IANA timezone names and DST rules are out of scope.
func NormalizeUTCOffset(s string) (string, bool) {
	s = strings.TrimSpace(s)
	if strings.EqualFold(s, "UTC") || strings.EqualFold(s, "Z") {
		return "UTC", true
	}
	if s == "" || (s[0] != '+' && s[0] != '-') {
		return "", false
	}
	sign := s[0]
	hours, minutes, ok := parseUTCOffsetBody(s[1:])
	if !ok || hours > 23 || minutes > 59 {
		return "", false
	}
	if hours == 0 && minutes == 0 {
		return "UTC", true
	}
	return fmt.Sprintf("%c%02d:%02d", sign, hours, minutes), true
}

// IsValidRFC3339Date reports whether s is an RFC 3339 full-date (YYYY-MM-DD).
func IsValidRFC3339Date(s string) bool {
	if len(s) != 10 || s[4] != '-' || s[7] != '-' {
		return false
	}
	year, ok := parseASCIIInt(s[0:4])
	if !ok {
		return false
	}
	month, ok := parseASCIIInt(s[5:7])
	if !ok {
		return false
	}
	day, ok := parseASCIIInt(s[8:10])
	if !ok {
		return false
	}
	return month >= 1 && month <= 12 && day >= 1 && day <= daysInMonth(year, month)
}

// IsValidRFC3339Time reports whether s is an RFC 3339 partial-time (HH:MM:SS[.fraction]).
func IsValidRFC3339Time(s string) bool {
	if len(s) < 8 || s[2] != ':' || s[5] != ':' {
		return false
	}
	hour, ok := parseASCIIInt(s[0:2])
	if !ok {
		return false
	}
	minute, ok := parseASCIIInt(s[3:5])
	if !ok {
		return false
	}
	second, ok := parseASCIIInt(s[6:8])
	if !ok {
		return false
	}
	if hour > 23 || minute > 59 || second > 59 {
		return false
	}
	if len(s) == 8 {
		return true
	}
	if s[8] != '.' || len(s) == 9 {
		return false
	}
	return isASCIIDigits(s[9:])
}

// IsValidRFC3339 reports whether s is a complete RFC 3339 date-time, such as
// 2026-02-14T10:30:00Z or 2026-02-14T10:30:00.5+08:00. Composed from
// IsValidRFC3339Date and IsValidRFC3339Time: a full-date, a T/t separator, a
// partial-time (with optional fractional seconds), and a mandatory time-offset
// (Z/z or ±HH:MM with HH in 00..23 and MM in 00..59). The offset is required, so
// a bare 2026-02-14T10:30:00 is rejected; a space separator is rejected; and a
// leap second (:60) is rejected, matching IsValidRFC3339Time. Non-ASCII input is
// rejected.
func IsValidRFC3339(s string) bool {
	if len(s) < 20 || !isASCIIString(s) {
		return false
	}
	if !IsValidRFC3339Date(s[0:10]) {
		return false
	}
	if s[10] != 'T' && s[10] != 't' {
		return false
	}
	rest := s[11:]
	var partial string
	if last := rest[len(rest)-1]; last == 'Z' || last == 'z' {
		partial = rest[:len(rest)-1]
	} else {
		if len(rest) < 6 || !isRFC3339NumOffset(rest[len(rest)-6:]) {
			return false
		}
		partial = rest[:len(rest)-6]
	}
	return IsValidRFC3339Time(partial)
}

func isRFC3339NumOffset(o string) bool {
	if len(o) != 6 || (o[0] != '+' && o[0] != '-') || o[3] != ':' {
		return false
	}
	hours, ok := parseASCIIInt(o[1:3])
	if !ok {
		return false
	}
	minutes, ok := parseASCIIInt(o[4:6])
	if !ok {
		return false
	}
	return hours <= 23 && minutes <= 59
}

func isASCIIString(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] >= 0x80 {
			return false
		}
	}
	return true
}

// IsValidBCP47 reports whether s is a structurally well-formed BCP 47 (RFC 5646)
// language tag. This is a grammar-level check, not a registry lookup: it accepts
// hyphen-separated ASCII-alphanumeric subtags (each 1-8 characters) whose primary
// subtag is a 2-3 letter language code, or the x/i privateuse/grandfathered lead.
// It rejects the POSIX underscore form (zh_CN), empty or misplaced hyphens, non-ASCII,
// and out-of-range primaries such as chinese. It does not verify that subtags are
// registered with IANA.
func IsValidBCP47(s string) bool {
	if s == "" {
		return false
	}
	for index, subtag := range strings.Split(s, "-") {
		if len(subtag) < 1 || len(subtag) > 8 || !isASCIIAlnumString(subtag) {
			return false
		}
		if index == 0 {
			isLanguage := len(subtag) >= 2 && len(subtag) <= 3 && isASCIIAlphaString(subtag)
			isSpecial := subtag == "x" || subtag == "i"
			if !isLanguage && !isSpecial {
				return false
			}
		}
	}
	return true
}

func isASCIIAlnumString(s string) bool {
	for i := 0; i < len(s); i++ {
		if !isASCIIAlphanumeric(s[i]) {
			return false
		}
	}
	return true
}

func isASCIIAlphaString(s string) bool {
	for i := 0; i < len(s); i++ {
		if !isASCIIAlpha(s[i]) {
			return false
		}
	}
	return true
}

func parseUTCOffsetBody(body string) (int, int, bool) {
	if body == "" {
		return 0, 0, false
	}
	if strings.Contains(body, ":") {
		parts := strings.Split(body, ":")
		if len(parts) != 2 || parts[0] == "" || len(parts[0]) > 2 || len(parts[1]) != 2 {
			return 0, 0, false
		}
		hours, ok := parseASCIIInt(parts[0])
		if !ok {
			return 0, 0, false
		}
		minutes, ok := parseASCIIInt(parts[1])
		return hours, minutes, ok
	}
	if !isASCIIDigits(body) {
		return 0, 0, false
	}
	switch len(body) {
	case 1, 2:
		hours, ok := parseASCIIInt(body)
		return hours, 0, ok
	case 4:
		hours, ok := parseASCIIInt(body[:2])
		if !ok {
			return 0, 0, false
		}
		minutes, ok := parseASCIIInt(body[2:])
		return hours, minutes, ok
	default:
		return 0, 0, false
	}
}

func parseASCIIInt(s string) (int, bool) {
	if !isASCIIDigits(s) {
		return 0, false
	}
	n, err := strconv.Atoi(s)
	if err != nil {
		return 0, false
	}
	return n, true
}

func isASCIIDigits(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

func daysInMonth(year int, month int) int {
	switch month {
	case 1, 3, 5, 7, 8, 10, 12:
		return 31
	case 4, 6, 9, 11:
		return 30
	case 2:
		if isLeapYear(year) {
			return 29
		}
		return 28
	default:
		return 0
	}
}

func isLeapYear(year int) bool {
	return year%4 == 0 && (year%100 != 0 || year%400 == 0)
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

type redactionContext struct {
	secretNames map[string]struct{}
}

func newRedactionContext(r Redactor) redactionContext {
	names := make(map[string]struct{}, len(r.SecretNames))
	for _, name := range r.SecretNames {
		names[name] = struct{}{}
	}
	return redactionContext{secretNames: names}
}

func (c redactionContext) isSecretKey(key string) bool {
	if keyHasSecretSuffix(key) {
		return true
	}
	if len(c.secretNames) == 0 {
		return false
	}
	_, ok := c.secretNames[key]
	return ok
}

func keyHasSecretSuffix(key string) bool {
	return strings.HasSuffix(key, "_secret") || strings.HasSuffix(key, "_SECRET")
}

func keyHasURLSuffix(key string) bool {
	return strings.HasSuffix(key, "_url") || strings.HasSuffix(key, "_URL")
}

// redactSecretsWithContext walks value applying default-policy redaction and
// returns the (possibly replaced) value. Containers are mutated in place. A
// _secret field becomes "***"; a _url field has its URL secrets scrubbed in
// place. No other string is scanned.
const maxDepth = 256
const maxDepthMarker = "<afdata:max-depth>"

func redactSecretsWithContext(value any, context redactionContext) any {
	return redactSecretsWithContextDepth(value, context, 0)
}

func redactSecretsWithContextDepth(value any, context redactionContext, depth int) any {
	if depth >= maxDepth {
		return maxDepthMarker
	}
	switch v := value.(type) {
	case map[string]any:
		for k := range v {
			switch {
			case context.isSecretKey(k):
				v[k] = "***"
			case keyHasURLSuffix(k):
				if s, ok := v[k].(string); ok {
					v[k] = redactURLFieldValue(s, context)
				} else {
					v[k] = redactSecretsWithContextDepth(v[k], context, depth+1)
				}
			default:
				v[k] = redactSecretsWithContextDepth(v[k], context, depth+1)
			}
		}
		return v
	case []any:
		for i, item := range v {
			v[i] = redactSecretsWithContextDepth(item, context, depth+1)
		}
		return v
	default:
		return value
	}
}

func applyRedactionPolicyWithContext(value any, redactionPolicy RedactionPolicy, context redactionContext) any {
	switch redactionPolicy {
	case RedactionTraceOnly:
		if obj, ok := value.(map[string]any); ok {
			if trace, exists := obj["trace"]; exists {
				obj["trace"] = redactSecretsWithContext(trace, context)
			}
		}
		return value
	case RedactionOff:
		// Explicitly disabled.
		return value
	default:
		// Zero value "" or RedactionAll falls back to full redaction.
		return redactSecretsWithContext(value, context)
	}
}

// ═══════════════════════════════════════════
// URL Secret Redaction
// ═══════════════════════════════════════════

func redactURLFieldValue(s string, context redactionContext) string {
	if redacted, ok := redactURLInStr(s, context); ok {
		return redacted
	}
	trimmed := strings.TrimSpace(s)
	if trimmed != s {
		if redacted, ok := redactURLInStr(trimmed, context); ok {
			return redacted
		}
	}
	// Fail closed: a _url value we could not parse as a clean scheme-prefixed
	// URL, yet which carries a credential sigil ('@' userinfo) or internal
	// whitespace, is redacted wholesale rather than passed through. A schemeless
	// connection string like user:pass@host/db has no scheme anchor for the
	// surgical span logic above, so blanket redaction is the safe default.
	hasWhitespace := strings.IndexFunc(s, func(r rune) bool {
		return r == ' ' || r == '\t' || r == '\n' || r == '\r' || r == '\f' || r == '\v'
	}) >= 0
	if hasWhitespace || strings.Contains(s, "@") {
		return "***"
	}
	return s
}

// redactURLInStr redacts secret components of a single URL string, returning
// (redacted, true) when s is a processable URL, or ("", false) when it is not
// (so callers keep the original). Only secret spans change; all other bytes are
// preserved.
func redactURLInStr(s string, context redactionContext) (string, bool) {
	// Precondition (spec): a single, whitespace-free, scheme-prefixed URL.
	// The gate is scheme + no-whitespace only — NOT "parses via net/url". Span
	// location below is purely byte-wise (we never re-serialize the URL), so a
	// url.Parse gate would only diverge across languages — net/url accepts
	// inputs (ports > 65535, empty host) that other languages' URL libraries
	// reject, and rejecting here would silently leak secrets in those values.
	if !strings.Contains(s, "://") || !isSingleURL(s) {
		return "", false
	}
	schemeSep := strings.Index(s, "://")
	if schemeSep < 0 {
		return "", false
	}
	scheme := s[:schemeSep]
	rest := s[schemeSep+3:]

	// Authority runs from after "://" to the first '/', '?', or '#'.
	authEnd := strings.IndexAny(rest, "/?#")
	if authEnd < 0 {
		authEnd = len(rest)
	}
	authority := rest[:authEnd]
	remainder := rest[authEnd:]

	newAuthority := redactUserinfoPassword(authority)

	// Query runs from the first '?' to the first '#' (or end).
	newRemainder := remainder
	if q := strings.Index(remainder, "?"); q >= 0 {
		path := remainder[:q]
		queryBody := remainder[q+1:]
		query := queryBody
		fragment := ""
		if h := strings.Index(queryBody, "#"); h >= 0 {
			query = queryBody[:h]
			fragment = queryBody[h:]
		}
		newRemainder = path + "?" + redactQuery(query, context) + fragment
	}

	return scheme + "://" + newAuthority + newRemainder, true
}

// redactUserinfoPassword replaces the userinfo password (user:pass@) with "***",
// preserving the username. Authority without '@', or userinfo without ':', is
// unchanged.
func redactUserinfoPassword(authority string) string {
	at := strings.LastIndex(authority, "@")
	if at < 0 {
		return authority
	}
	userinfo := authority[:at]
	colon := strings.Index(userinfo, ":")
	if colon < 0 {
		return authority
	}
	return authority[:colon] + ":***" + authority[at:]
}

// redactQuery redacts the values of secret-named query parameters, preserving
// raw bytes of every other segment (keys, benign values, encoding, ordering,
// separators).
func redactQuery(query string, context redactionContext) string {
	segments := strings.Split(query, "&")
	for i, segment := range segments {
		eq := strings.Index(segment, "=")
		if eq < 0 {
			continue
		}
		rawKey := segment[:eq]
		// Form-decode the name (`+` → space, percent-decode) for the check.
		name := formDecodeName(segment)
		if context.isSecretKey(name) {
			segments[i] = rawKey + "=***"
		}
	}
	return strings.Join(segments, "&")
}

// formDecodeName form-decodes the parameter name (the bytes before the first
// '='), matching application/x-www-form-urlencoded: '+' → space then
// percent-decode. Falls back to the raw key bytes on a decode error.
func formDecodeName(segment string) string {
	eq := strings.Index(segment, "=")
	rawKey := segment
	if eq >= 0 {
		rawKey = segment[:eq]
	}
	decoded, err := url.QueryUnescape(rawKey)
	if err != nil {
		return rawKey
	}
	return decoded
}

// isSingleURL reports whether s begins with a URL scheme
// (ALPHA *(ALPHA / DIGIT / "+" / "-" / ".") "://") and contains no ASCII
// whitespace — i.e. a single bare URL, not a URL embedded in prose.
func isSingleURL(s string) bool {
	for i := 0; i < len(s); i++ {
		if isASCIIWhitespace(s[i]) {
			return false
		}
	}
	if len(s) == 0 || !isASCIIAlpha(s[0]) {
		return false
	}
	i := 1
	for i < len(s) {
		c := s[i]
		if isASCIIAlphanumeric(c) || c == '+' || c == '-' || c == '.' {
			i++
		} else {
			break
		}
	}
	return strings.HasPrefix(s[i:], "://")
}

func isASCIIWhitespace(b byte) bool {
	switch b {
	case '\t', '\n', '\v', '\f', '\r', ' ':
		return true
	}
	return false
}

func isASCIIAlpha(b byte) bool {
	return (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z')
}

func isASCIIAlphanumeric(b byte) bool {
	return isASCIIAlpha(b) || (b >= '0' && b <= '9')
}

// ═══════════════════════════════════════════
// Suffix Processing
// ═══════════════════════════════════════════

// stripSuffixCI strips a suffix matching exact lowercase or exact uppercase only.
func stripSuffixCI(key, suffixLower string) (string, bool) {
	if strings.HasSuffix(key, suffixLower) {
		return key[:len(key)-len(suffixLower)], true
	}
	suffixUpper := strings.ToUpper(suffixLower)
	if strings.HasSuffix(key, suffixUpper) {
		return key[:len(key)-len(suffixUpper)], true
	}
	return "", false
}

// tryStripGenericCents extracts currency code from _{code}_cents / _{CODE}_CENTS.
func tryStripGenericCents(key string) (stripped, code string, ok bool) {
	code = extractCurrencyCode(key)
	if code == "" {
		return "", "", false
	}
	suffixLen := len(code) + len("_cents") + 1 // _{code}_cents
	stripped = key[:len(key)-suffixLen]
	if stripped == "" {
		return "", "", false
	}
	return stripped, code, true
}

// tryStripGenericMicro extracts currency code from _{code}_micro / _{CODE}_MICRO.
func tryStripGenericMicro(key string) (stripped, code string, ok bool) {
	code = extractCurrencyCodeMicro(key)
	if code == "" {
		return "", "", false
	}
	suffixLen := len(code) + len("_micro") + 1 // _{code}_micro
	stripped = key[:len(key)-suffixLen]
	if stripped == "" {
		return "", "", false
	}
	return stripped, code, true
}

type processedField struct {
	key         string
	value       any
	formatted   string
	isFormatted bool
}

// tryProcessField tries suffix-driven processing.
// Returns (stripped_key, formatted_value, true) or ("", "", false).
func tryProcessField(key string, value any) (string, string, bool) {
	// Group 1: compound timestamp suffixes
	if stripped, ok := stripSuffixCI(key, "_epoch_ms"); ok {
		if n, ok := asInt64(value); ok {
			if formatted, ok := formatRFC3339Ms(n); ok {
				return stripped, formatted, true
			}
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_epoch_s"); ok {
		if n, ok := asInt64(value); ok {
			if n > math.MaxInt64/1000 || n < math.MinInt64/1000 {
				return "", "", false // *1000 would overflow; fall through to raw
			}
			if formatted, ok := formatRFC3339Ms(n * 1000); ok {
				return stripped, formatted, true
			}
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_epoch_ns"); ok {
		if n, ok := asDecimalInt64(value); ok {
			ms := n / 1_000_000
			if n%1_000_000 < 0 {
				ms--
			}
			if formatted, ok := formatRFC3339Ms(ms); ok {
				return stripped, formatted, true
			}
		}
		return "", "", false
	}

	// Group 2: compound currency suffixes
	if stripped, ok := stripSuffixCI(key, "_usd_cents"); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, fmt.Sprintf("$%d.%02d", n/100, n%100), true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_eur_cents"); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, fmt.Sprintf("\u20ac%d.%02d", n/100, n%100), true
		}
		return "", "", false
	}
	if stripped, code, ok := tryStripGenericCents(key); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, fmt.Sprintf("%d.%02d %s", n/100, n%100, strings.ToUpper(code)), true
		}
		return "", "", false
	}
	if stripped, code, ok := tryStripGenericMicro(key); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, fmt.Sprintf("%d.%06d %s", n/1_000_000, n%1_000_000, strings.ToUpper(code)), true
		}
		return "", "", false
	}

	// Group 3: multi-char suffixes
	if stripped, ok := stripSuffixCI(key, "_rfc3339"); ok {
		if s, ok := value.(string); ok {
			return stripped, s, true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_minutes"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + " minutes", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_hours"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + " hours", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_days"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + " days", true
		}
		return "", "", false
	}

	// Group 4: single-unit suffixes
	if stripped, ok := stripSuffixCI(key, "_msats"); ok {
		if text, ok := decimalIntText(value); ok {
			return stripped, text + "msats", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_sats"); ok {
		if text, ok := decimalIntText(value); ok {
			return stripped, text + "sats", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_bytes"); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, formatBytesHuman(n), true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_percent"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "%", true
		}
		return "", "", false
	}
	// Group 5: short suffixes (last to avoid false positives)
	if stripped, ok := stripSuffixCI(key, "_jpy"); ok {
		if n, ok := asNonNegInt64(value); ok {
			return stripped, fmt.Sprintf("\u00a5%s", formatWithCommas(uint64(n))), true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_ns"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "ns", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_us"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "\u03bcs", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_ms"); ok {
		if formatted, ok := formatMsValue(value); ok {
			return stripped, formatted, true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_s"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "s", true
		}
		return "", "", false
	}

	return "", "", false
}

// processObjectFields processes fields: strip keys, format values, detect collisions.
func processObjectFields(m map[string]any) []processedField {
	type entry struct {
		stripped    string
		original    string
		value       any
		formatted   string
		isFormatted bool
	}

	entries := make([]entry, 0, len(m))
	for k, v := range m {
		if stripped, ok := stripSuffixCI(k, "_secret"); ok {
			entries = append(entries, entry{stripped, k, v, "", false})
			continue
		}
		if stripped, formatted, ok := tryProcessField(k, v); ok {
			entries = append(entries, entry{stripped, k, v, formatted, true})
		} else {
			entries = append(entries, entry{k, k, v, "", false})
		}
	}

	// Detect collisions
	counts := make(map[string]int)
	for _, e := range entries {
		counts[e.stripped]++
	}

	// Resolve collisions: revert both key and formatted value
	result := make([]processedField, len(entries))
	for i, e := range entries {
		displayKey := e.stripped
		isFormatted := e.isFormatted
		formatted := e.formatted
		if counts[e.stripped] > 1 && e.original != e.stripped {
			displayKey = e.original
			isFormatted = false
			formatted = ""
		}
		result[i] = processedField{displayKey, e.value, formatted, isFormatted}
	}

	// Sort by display key (JCS order)
	sort.Slice(result, func(i, j int) bool {
		return jcsLess(result[i].key, result[j].key)
	})
	return result
}

// ═══════════════════════════════════════════
// Formatting Helpers
// ═══════════════════════════════════════════

// formatMsAsSeconds formats ms as seconds: 3 decimal places, trim trailing zeros, min 1 decimal.
func formatMsAsSeconds(ms float64) string {
	formatted := fmt.Sprintf("%.3f", ms/1000)
	trimmed := strings.TrimRight(formatted, "0")
	if strings.HasSuffix(trimmed, ".") {
		return trimmed + "0s"
	}
	return trimmed + "s"
}

// formatMsValue formats _ms value: < 1000 → {n}ms, ≥ 1000 → seconds.
func formatMsValue(value any) (string, bool) {
	n, ok := asFloat64(value)
	if !ok {
		return "", false
	}
	if math.Abs(n) >= 1000 {
		return formatMsAsSeconds(n), true
	}
	return plainScalar(value) + "ms", true
}

const minRFC3339Ms int64 = -62135596800000
const maxRFC3339Ms int64 = 253402300799999

func formatRFC3339Ms(ms int64) (string, bool) {
	if ms < minRFC3339Ms || ms > maxRFC3339Ms {
		return "", false
	}
	sec := ms / 1000
	rem := ms % 1000
	if rem < 0 {
		sec--
		rem += 1000
	}
	nsec := rem * 1_000_000
	t := time.Unix(sec, nsec).UTC()
	return t.Format("2006-01-02T15:04:05.000Z"), true
}

func formatBytesHuman(bytes int64) string {
	const KiB = 1024.0
	const MiB = KiB * 1024
	const GiB = MiB * 1024
	const TiB = GiB * 1024

	b := float64(bytes)
	switch {
	case b >= TiB:
		return fmt.Sprintf("%.1fTiB", b/TiB)
	case b >= GiB:
		return fmt.Sprintf("%.1fGiB", b/GiB)
	case b >= MiB:
		return fmt.Sprintf("%.1fMiB", b/MiB)
	case b >= KiB:
		return fmt.Sprintf("%.1fKiB", b/KiB)
	default:
		return fmt.Sprintf("%dB", bytes)
	}
}

func formatWithCommas(n uint64) string {
	s := fmt.Sprintf("%d", n)
	if len(s) <= 3 {
		return s
	}
	var result strings.Builder
	for i, c := range s {
		if i > 0 && (len(s)-i)%3 == 0 {
			result.WriteByte(',')
		}
		result.WriteRune(c)
	}
	return result.String()
}

// extractCurrencyCode extracts code from _{code}_cents / _{CODE}_CENTS suffix.
func extractCurrencyCode(key string) string {
	var withoutCents string
	if strings.HasSuffix(key, "_cents") {
		withoutCents = key[:len(key)-6]
	} else if strings.HasSuffix(key, "_CENTS") {
		withoutCents = key[:len(key)-6]
	} else {
		return ""
	}
	return extractCurrencyCodeFromStem(withoutCents)
}

// extractCurrencyCodeMicro extracts code from _{code}_micro / _{CODE}_MICRO suffix.
func extractCurrencyCodeMicro(key string) string {
	var withoutMicro string
	if strings.HasSuffix(key, "_micro") {
		withoutMicro = key[:len(key)-6]
	} else if strings.HasSuffix(key, "_MICRO") {
		withoutMicro = key[:len(key)-6]
	} else {
		return ""
	}
	return extractCurrencyCodeFromStem(withoutMicro)
}

func extractCurrencyCodeFromStem(stem string) string {
	idx := strings.LastIndex(stem, "_")
	if idx < 0 {
		return ""
	}
	code := stem[idx+1:]
	if code == "" || len(code) < 3 || len(code) > 4 {
		return ""
	}
	for i := 0; i < len(code); i++ {
		c := code[i]
		if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) {
			return ""
		}
	}
	return code
}

// ═══════════════════════════════════════════
// YAML Rendering (structure-preserving)
// ═══════════════════════════════════════════

func renderYamlRaw(value any, indent int, lines *[]string) {
	prefix := strings.Repeat("  ", indent)
	switch v := value.(type) {
	case map[string]any:
		for _, key := range sortedObjectKeys(v) {
			renderYamlFieldRaw(prefix, key, v[key], indent, lines)
		}
	case []any:
		renderYamlArrayRaw(v, indent, lines)
	default:
		*lines = append(*lines, fmt.Sprintf("%s%s", prefix, yamlScalar(value)))
	}
}

func renderYamlFieldRaw(prefix, key string, value any, indent int, lines *[]string) {
	switch v := value.(type) {
	case map[string]any:
		if len(v) > 0 {
			*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, yamlKey(key)))
			renderYamlRaw(v, indent+1, lines)
		} else {
			*lines = append(*lines, fmt.Sprintf("%s%s: {}", prefix, yamlKey(key)))
		}
	case []any:
		if len(v) == 0 {
			*lines = append(*lines, fmt.Sprintf("%s%s: []", prefix, yamlKey(key)))
		} else {
			*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, yamlKey(key)))
			renderYamlArrayRaw(v, indent+1, lines)
		}
	default:
		*lines = append(*lines, fmt.Sprintf("%s%s: %s", prefix, yamlKey(key), yamlScalar(value)))
	}
}

func renderYamlArrayRaw(arr []any, indent int, lines *[]string) {
	prefix := strings.Repeat("  ", indent)
	for _, item := range arr {
		switch v := item.(type) {
		case map[string]any:
			if len(v) > 0 {
				*lines = append(*lines, fmt.Sprintf("%s-", prefix))
				renderYamlRaw(v, indent+1, lines)
			} else {
				*lines = append(*lines, fmt.Sprintf("%s- {}", prefix))
			}
		case []any:
			if len(v) > 0 {
				*lines = append(*lines, fmt.Sprintf("%s-", prefix))
				renderYamlArrayRaw(v, indent+1, lines)
			} else {
				*lines = append(*lines, fmt.Sprintf("%s- []", prefix))
			}
		default:
			*lines = append(*lines, fmt.Sprintf("%s- %s", prefix, yamlScalar(item)))
		}
	}
}

func escapeYamlStr(s string) string {
	s = strings.ReplaceAll(s, `\`, `\\`)
	s = strings.ReplaceAll(s, `"`, `\"`)
	s = strings.ReplaceAll(s, "\n", `\n`)
	s = strings.ReplaceAll(s, "\r", `\r`)
	s = strings.ReplaceAll(s, "\t", `\t`)
	s = strings.ReplaceAll(s, "\f", `\f`)
	s = strings.ReplaceAll(s, "\v", `\v`)
	return s
}

func yamlKey(key string) string {
	if isSafeKey(key) {
		return key
	}
	return `"` + escapeYamlStr(key) + `"`
}

func quoteLogfmtKey(key string) string {
	if isSafeKey(key) {
		return key
	}
	return quoteLogfmtValue(key)
}

func isSafeKey(key string) bool {
	if key == "" {
		return false
	}
	for i := 0; i < len(key); i++ {
		c := key[i]
		if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-' || c == '.') {
			return false
		}
	}
	return true
}

func yamlScalar(value any) string {
	switch v := value.(type) {
	case string:
		return fmt.Sprintf(`"%s"`, escapeYamlStr(v))
	case nil:
		return "null"
	case bool:
		if v {
			return "true"
		}
		return "false"
	case int:
		return strconv.Itoa(v)
	case int64:
		return strconv.FormatInt(v, 10)
	case float64:
		// 'f', -1 yields the shortest round-trip form and drops the trailing
		// ".0" from integral floats (3.0 -> "3"), matching Rust/TS/Python.
		return formatFloatCanonical(v)
	case json.Number:
		return v.String()
	case map[string]any, []any:
		return fmt.Sprintf(`"%s"`, escapeYamlStr(canonicalJSON(value)))
	default:
		return fmt.Sprintf(`"<unsupported:%T>"`, value)
	}
}

// ═══════════════════════════════════════════
// Plain Rendering (logfmt)
// ═══════════════════════════════════════════

func collectPlainPairs(value any, prefix string, pairs *[][2]string) {
	m, ok := value.(map[string]any)
	if !ok {
		return
	}
	for _, pf := range processObjectFields(m) {
		fullKey := pf.key
		if prefix != "" {
			fullKey = prefix + "." + pf.key
		}
		if pf.isFormatted {
			*pairs = append(*pairs, [2]string{fullKey, pf.formatted})
		} else {
			switch v := pf.value.(type) {
			case map[string]any:
				collectPlainPairs(v, fullKey, pairs)
			case []any:
				parts := make([]string, len(v))
				for i, item := range v {
					parts[i] = plainScalar(item)
				}
				*pairs = append(*pairs, [2]string{fullKey, strings.Join(parts, ",")})
			case nil:
				*pairs = append(*pairs, [2]string{fullKey, ""})
			default:
				*pairs = append(*pairs, [2]string{fullKey, plainScalar(pf.value)})
			}
		}
	}
}

func collectPlainPairsRaw(value any, prefix string, pairs *[][2]string) {
	m, ok := value.(map[string]any)
	if !ok {
		return
	}
	for _, key := range sortedObjectKeys(m) {
		fullKey := key
		if prefix != "" {
			fullKey = prefix + "." + key
		}
		switch v := m[key].(type) {
		case map[string]any:
			collectPlainPairsRaw(v, fullKey, pairs)
		case []any:
			parts := make([]string, len(v))
			for i, item := range v {
				parts[i] = plainScalarRaw(item)
			}
			*pairs = append(*pairs, [2]string{fullKey, strings.Join(parts, ",")})
		case nil:
			*pairs = append(*pairs, [2]string{fullKey, ""})
		default:
			*pairs = append(*pairs, [2]string{fullKey, plainScalar(v)})
		}
	}
}

func plainScalar(value any) string {
	switch v := value.(type) {
	case string:
		return v
	case nil:
		return "null"
	case bool:
		if v {
			return "true"
		}
		return "false"
	case int:
		return strconv.Itoa(v)
	case int64:
		return strconv.FormatInt(v, 10)
	case float64:
		// 'f', -1 yields the shortest round-trip form and drops the trailing
		// ".0" from integral floats (3.0 -> "3"), matching Rust/TS/Python.
		return formatFloatCanonical(v)
	case json.Number:
		return v.String()
	case map[string]any, []any:
		return canonicalJSON(value)
	default:
		return fmt.Sprintf("<unsupported:%T>", value)
	}
}

func plainScalarRaw(value any) string {
	switch value.(type) {
	case map[string]any, []any:
		return canonicalJSON(value)
	}
	return plainScalar(value)
}

func formatFloatCanonical(v float64) string {
	if math.IsInf(v, 0) || math.IsNaN(v) {
		return strconv.FormatFloat(v, 'g', -1, 64)
	}
	if math.Trunc(v) == v && math.Abs(v) < 1e21 {
		return strconv.FormatFloat(v, 'f', 0, 64)
	}
	return normalizeExponent(strconv.FormatFloat(v, 'g', -1, 64))
}

func normalizeExponent(s string) string {
	e := strings.IndexAny(s, "eE")
	if e < 0 {
		return s
	}
	mantissa := s[:e]
	exp := s[e+1:]
	sign := ""
	if strings.HasPrefix(exp, "+") || strings.HasPrefix(exp, "-") {
		sign = exp[:1]
		exp = exp[1:]
	}
	exp = strings.TrimLeft(exp, "0")
	if exp == "" {
		exp = "0"
	}
	return mantissa + "e" + sign + exp
}

func quoteLogfmtValue(value string) string {
	if value == "" {
		return ""
	}
	needsQuote := false
	for _, r := range value {
		if r == '=' || r == '"' || r == '\\' || r == '\u00a0' || r == '\v' || r == '\f' || r == '\n' || r == '\r' || r == '\t' || r == ' ' {
			needsQuote = true
			break
		}
	}
	if !needsQuote {
		return value
	}
	escaped := strings.ReplaceAll(value, `\`, `\\`)
	escaped = strings.ReplaceAll(escaped, `"`, `\"`)
	escaped = strings.ReplaceAll(escaped, "\n", `\n`)
	escaped = strings.ReplaceAll(escaped, "\r", `\r`)
	escaped = strings.ReplaceAll(escaped, "\t", `\t`)
	escaped = strings.ReplaceAll(escaped, "\f", `\f`)
	escaped = strings.ReplaceAll(escaped, "\v", `\v`)
	return `"` + escaped + `"`
}

func canonicalJSON(value any) string {
	b, err := json.Marshal(sortJSONValue(value))
	if err != nil {
		return fmt.Sprintf("<unsupported:%T>", value)
	}
	return string(b)
}

func sortJSONValue(value any) any {
	switch v := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(v))
		for _, key := range sortedObjectKeys(v) {
			out[key] = sortJSONValue(v[key])
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			out[i] = sortJSONValue(item)
		}
		return out
	default:
		return value
	}
}

// ═══════════════════════════════════════════
// Utilities
// ═══════════════════════════════════════════

func asInt64(value any) (int64, bool) {
	switch v := value.(type) {
	case int:
		return int64(v), true
	case int64:
		return v, true
	case float64:
		if v == math.Trunc(v) && !math.IsInf(v, 0) {
			return int64(v), true
		}
	case json.Number:
		if n, err := v.Int64(); err == nil {
			return n, true
		}
	}
	return 0, false
}

func asNonNegInt64(value any) (int64, bool) {
	n, ok := asInt64(value)
	if ok && n >= 0 {
		return n, true
	}
	return 0, false
}

func asDecimalInt64(value any) (int64, bool) {
	if s, ok := value.(string); ok && isDecimalIntegerString(s) {
		n, err := strconv.ParseInt(s, 10, 64)
		return n, err == nil
	}
	return asInt64(value)
}

func decimalIntText(value any) (string, bool) {
	if s, ok := value.(string); ok && isDecimalIntegerString(s) {
		return s, true
	}
	if n, ok := asInt64(value); ok {
		return strconv.FormatInt(n, 10), true
	}
	return "", false
}

func isDecimalIntegerString(s string) bool {
	if strings.HasPrefix(s, "-") {
		s = strings.TrimPrefix(s, "-")
	}
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func asFloat64(value any) (float64, bool) {
	switch v := value.(type) {
	case int:
		return float64(v), true
	case int64:
		return float64(v), true
	case float64:
		return v, true
	case json.Number:
		if n, err := v.Float64(); err == nil {
			return n, true
		}
	}
	return 0, false
}

// normalize converts a Go value through JSON round-trip to get map[string]any.
func normalize(value any) any {
	switch value.(type) {
	case map[string]any, []any, string, float64, bool, nil, json.Number:
		return value
	}
	b, err := json.Marshal(value)
	if err != nil {
		return value
	}
	// UseNumber so large integers (>2^53) inside structs/uint64 survive the
	// round-trip instead of collapsing to a lossy float64.
	dec := json.NewDecoder(bytes.NewReader(b))
	dec.UseNumber()
	var result any
	if err := dec.Decode(&result); err != nil {
		return value
	}
	return result
}

// sanitizeForJSON converts values into JSON-safe data while preserving map/array structure.
func sanitizeForJSON(value any) any {
	return sanitizeForJSONWithVisited(value, map[visitKey]struct{}{}, 0)
}

type visitKey struct {
	kind reflect.Kind
	ptr  uintptr
}

func sanitizeForJSONWithVisited(value any, visited map[visitKey]struct{}, depth int) any {
	if depth >= maxDepth {
		return maxDepthMarker
	}
	switch v := value.(type) {
	case map[string]any:
		rv := reflect.ValueOf(v)
		key := visitKey{kind: rv.Kind(), ptr: rv.Pointer()}
		if key.ptr != 0 {
			if _, seen := visited[key]; seen {
				return "<unsupported:circular>"
			}
			visited[key] = struct{}{}
			defer delete(visited, key)
		}

		out := make(map[string]any, len(v))
		for k, item := range v {
			out[k] = sanitizeForJSONWithVisited(item, visited, depth+1)
		}
		return out
	case []any:
		if len(v) > 0 {
			rv := reflect.ValueOf(v)
			key := visitKey{kind: rv.Kind(), ptr: rv.Pointer()}
			if _, seen := visited[key]; seen {
				return "<unsupported:circular>"
			}
			visited[key] = struct{}{}
			defer delete(visited, key)
		}

		out := make([]any, len(v))
		for i, item := range v {
			out[i] = sanitizeForJSONWithVisited(item, visited, depth+1)
		}
		return out
	}

	normalized := normalize(value)
	if _, err := json.Marshal(normalized); err == nil {
		return normalized
	}
	// Never stringify raw value content here; it may contain secrets.
	return fmt.Sprintf("<unsupported:%T>", value)
}

// jcsLess compares two strings by UTF-16 code unit order per RFC 8785.
func jcsLess(a, b string) bool {
	ua := utf16.Encode([]rune(a))
	ub := utf16.Encode([]rune(b))
	for i := 0; i < len(ua) && i < len(ub); i++ {
		if ua[i] != ub[i] {
			return ua[i] < ub[i]
		}
	}
	return len(ua) < len(ub)
}

func sortedObjectKeys(m map[string]any) []string {
	keys := make([]string, 0, len(m))
	for key := range m {
		keys = append(keys, key)
	}
	sort.Slice(keys, func(i, j int) bool {
		return jcsLess(keys[i], keys[j])
	})
	return keys
}
