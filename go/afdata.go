// Package afdata implements Agent-First Data (AFDATA) output formatting
// and protocol templates.
//
// 29 public APIs and 5 types: 3 protocol builders + 3 value-copy redactors +
// 7 output formatters + 2 in-place value redactors (redact _secret and _url
// fields) + 2 URL-string redactors (operate on one URL string; the value
// redactors apply these to _url fields) + 4 utilities + 8 CLI helpers +
// OutputFormat + RedactionPolicy + RedactionOptions + OutputStyle + OutputOptions.
package afdata

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"math/bits"
	"net/url"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf16"
)

const maxSafeInteger = uint64(9007199254740991)

// ═══════════════════════════════════════════
// Public API: Protocol Builders
// ═══════════════════════════════════════════

// BuildJsonOk builds {code: "ok", result, trace?}.
func BuildJsonOk(result any, trace any) map[string]any {
	m := map[string]any{"code": "ok", "result": result}
	if trace != nil {
		m["trace"] = trace
	}
	return m
}

// BuildJsonError builds {code: "error", error: message, hint?, trace?}.
// Pass empty string for hint to omit it.
func BuildJsonError(message string, hint string, trace any) map[string]any {
	m := map[string]any{"code": "error", "error": message}
	if hint != "" {
		m["hint"] = hint
	}
	if trace != nil {
		m["trace"] = trace
	}
	return m
}

// BuildJson builds {code: "<custom>", ...fields, trace?}.
func BuildJson(code string, fields any, trace any) map[string]any {
	result := make(map[string]any)
	if m, ok := fields.(map[string]any); ok {
		for k, v := range m {
			result[k] = v
		}
	}
	result["code"] = code
	if trace != nil {
		result["trace"] = trace
	}
	return result
}

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

// RedactionPolicy controls scoped redaction behavior for OutputJsonWith.
type RedactionPolicy string

const (
	RedactionTraceOnly RedactionPolicy = "RedactionTraceOnly"
	RedactionNone      RedactionPolicy = "RedactionNone"
)

// RedactionOptions controls scoped redaction and legacy secret field names.
type RedactionOptions struct {
	// Policy controls where redaction is applied. Empty means default full redaction.
	Policy RedactionPolicy
	// SecretNames are field names to redact in addition to _secret suffixes.
	// Matching is exact field-name equality at any nesting level. The same list
	// also matches URL query-parameter names inside _url fields (see
	// RedactURLSecrets).
	SecretNames []string
}

// OutputStyle controls YAML/plain rendering style.
type OutputStyle string

const (
	// OutputStyleReadable strips AFDATA suffixes and formats values.
	OutputStyleReadable OutputStyle = "Readable"
	// OutputStyleRaw preserves keys and values after redaction.
	OutputStyleRaw OutputStyle = "Raw"
)

// OutputOptions combines redaction and rendering style.
type OutputOptions struct {
	Redaction RedactionOptions
	Style     OutputStyle
}

// OutputJson formats as single-line JSON. Secrets redacted, original keys, raw values.
func OutputJson(value any) string {
	return marshalOutputJSON(RedactedValue(value))
}

// OutputJsonWith formats as single-line JSON with explicit redaction policy.
func OutputJsonWith(value any, redactionPolicy RedactionPolicy) string {
	return marshalOutputJSON(RedactedValueWith(value, redactionPolicy))
}

// OutputJsonWithOptions formats as single-line JSON with explicit output options.
// JSON ignores OutputStyle and preserves original keys and values after redaction.
func OutputJsonWithOptions(value any, outputOptions OutputOptions) string {
	return marshalOutputJSON(RedactedValueWithOptions(value, outputOptions.Redaction))
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

// OutputYaml formats as multi-line YAML. Keys stripped, values formatted, secrets redacted.
func OutputYaml(value any) string {
	return OutputYamlWithOptions(value, OutputOptions{Redaction: RedactionOptions{}})
}

// OutputYamlWithOptions formats as multi-line YAML with explicit output options.
func OutputYamlWithOptions(value any, outputOptions OutputOptions) string {
	lines := []string{"---"}
	v := RedactedValueWithOptions(value, outputOptions.Redaction)
	if outputOptions.Style == OutputStyleRaw {
		renderYamlRaw(v, 0, &lines)
	} else {
		renderYamlProcessed(v, 0, &lines)
	}
	return strings.Join(lines, "\n")
}

// OutputPlain formats as single-line logfmt. Keys stripped, values formatted, secrets redacted.
func OutputPlain(value any) string {
	return OutputPlainWithOptions(value, OutputOptions{Redaction: RedactionOptions{}})
}

// OutputPlainWithOptions formats as single-line logfmt with explicit output options.
func OutputPlainWithOptions(value any, outputOptions OutputOptions) string {
	var pairs [][2]string
	v := RedactedValueWithOptions(value, outputOptions.Redaction)
	if outputOptions.Style == OutputStyleRaw {
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

// RedactSecretsInPlace redacts _secret fields in-place. Container roots
// (objects, arrays) are mutated in place; a bare string root cannot be replaced
// through this API — use RedactedValue or RedactURLSecrets for that.
func RedactSecretsInPlace(value any) {
	redactSecretsWithContext(value, redactionContext{})
}

// RedactSecretsInPlaceWithOptions redacts secret fields in-place using explicit options.
func RedactSecretsInPlaceWithOptions(value any, redactionOptions RedactionOptions) {
	context := newRedactionContext(redactionOptions)
	switch redactionOptions.Policy {
	case RedactionTraceOnly:
		if obj, ok := value.(map[string]any); ok {
			if trace, exists := obj["trace"]; exists {
				obj["trace"] = redactSecretsWithContext(trace, context)
			}
		}
	case RedactionNone:
		// Explicitly disabled.
	default:
		redactSecretsWithContext(value, context)
	}
}

// RedactedValue returns a JSON-safe copy with default _secret redaction applied.
func RedactedValue(value any) any {
	return RedactedValueWithOptions(value, RedactionOptions{})
}

// RedactedValueWith returns a JSON-safe copy with an explicit redaction policy applied.
func RedactedValueWith(value any, redactionPolicy RedactionPolicy) any {
	v := sanitizeForJSON(value)
	return applyRedactionPolicyWithContext(v, redactionPolicy, redactionContext{})
}

// RedactedValueWithOptions returns a JSON-safe copy with explicit redaction options applied.
func RedactedValueWithOptions(value any, redactionOptions RedactionOptions) any {
	v := sanitizeForJSON(value)
	return applyRedactionOptions(v, redactionOptions)
}

// RedactURLSecrets redacts secret components of a single URL string, using
// default options. Returns url with its userinfo password and any
// _secret-suffixed query parameter values replaced by "***".
// See RedactURLSecretsWithOptions.
func RedactURLSecrets(rawURL string) string {
	return RedactURLSecretsWithOptions(rawURL, RedactionOptions{})
}

// RedactURLSecretsWithOptions redacts secret components of a single URL string.
//
// A query parameter is redacted iff its (form-decoded) name ends in
// _secret/_SECRET or matches an exact entry in SecretNames. The userinfo
// password (scheme://user:pass@host) is always redacted as a structural rule.
// Only the secret spans are replaced with "***"; every other byte is preserved.
// A string that is not a single, whitespace-free, scheme-prefixed URL (including
// a URL embedded in surrounding prose) is returned unchanged.
func RedactURLSecretsWithOptions(rawURL string, redactionOptions RedactionOptions) string {
	context := newRedactionContext(redactionOptions)
	if redacted, ok := redactURLInStr(rawURL, context); ok {
		return redacted
	}
	return rawURL
}

// ParseSize parses a human-readable size string into bytes.
// Accepts bare numbers or numbers followed by a unit letter (B/K/M/G/T).
// Case-insensitive. Trims whitespace. Returns (0, false) for invalid input.
func ParseSize(s string) (uint64, bool) {
	s = strings.TrimSpace(s)
	if s == "" {
		return 0, false
	}
	last := s[len(s)-1]
	var numStr string
	var mult uint64
	switch {
	case last == 'B' || last == 'b':
		numStr, mult = s[:len(s)-1], 1
	case last == 'K' || last == 'k':
		numStr, mult = s[:len(s)-1], 1024
	case last == 'M' || last == 'm':
		numStr, mult = s[:len(s)-1], 1024*1024
	case last == 'G' || last == 'g':
		numStr, mult = s[:len(s)-1], 1024*1024*1024
	case last == 'T' || last == 't':
		numStr, mult = s[:len(s)-1], 1024*1024*1024*1024
	case (last >= '0' && last <= '9') || last == '.':
		numStr, mult = s, 1
	default:
		return 0, false
	}
	if numStr == "" || !isDecimalNumber(numStr) {
		return 0, false
	}
	if n, err := strconv.ParseUint(numStr, 10, 64); err == nil {
		hi, lo := bits.Mul64(n, mult)
		if hi != 0 || lo > maxSafeInteger {
			return 0, false
		}
		return lo, true
	}
	// Integer overflow must not silently fall back to float parsing.
	if !strings.ContainsAny(numStr, ".eE") {
		return 0, false
	}
	f, err := strconv.ParseFloat(numStr, 64)
	if err != nil || f < 0 || math.IsNaN(f) || math.IsInf(f, 0) {
		return 0, false
	}
	result := f * float64(mult)
	if result > float64(maxSafeInteger) {
		return 0, false
	}
	return uint64(result), true
}

func isDecimalNumber(s string) bool {
	i := 0
	digitsBefore := 0
	for i < len(s) && s[i] >= '0' && s[i] <= '9' {
		i++
		digitsBefore++
	}
	digitsAfter := 0
	if i < len(s) && s[i] == '.' {
		i++
		for i < len(s) && s[i] >= '0' && s[i] <= '9' {
			i++
			digitsAfter++
		}
	}
	if digitsBefore+digitsAfter == 0 {
		return false
	}
	if i < len(s) && (s[i] == 'e' || s[i] == 'E') {
		i++
		if i < len(s) && (s[i] == '+' || s[i] == '-') {
			i++
		}
		expDigits := 0
		for i < len(s) && s[i] >= '0' && s[i] <= '9' {
			i++
			expDigits++
		}
		if expDigits == 0 {
			return false
		}
	}
	return i == len(s)
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

func newRedactionContext(redactionOptions RedactionOptions) redactionContext {
	names := make(map[string]struct{}, len(redactionOptions.SecretNames))
	for _, name := range redactionOptions.SecretNames {
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

func redactSecretsWithContext(value any, context redactionContext) any {
	return redactSecretsWithContextDepth(value, context, 0)
}

func redactSecretsWithContextDepth(value any, context redactionContext, depth int) any {
	if depth >= maxDepth {
		return "***"
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

func applyRedactionOptions(value any, redactionOptions RedactionOptions) any {
	context := newRedactionContext(redactionOptions)
	return applyRedactionPolicyWithContext(value, redactionOptions.Policy, context)
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
	case RedactionNone:
		// Explicitly disabled.
		return value
	default:
		// Empty/unknown policy falls back to default full redaction.
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
		if n, ok := asInt64(value); ok {
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
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "msats", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_sats"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + "sats", true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_bytes"); ok {
		if n, ok := asInt64(value); ok {
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
	if stripped, ok := stripSuffixCI(key, "_btc"); ok {
		if _, ok := asFloat64(value); ok {
			return stripped, plainScalar(value) + " BTC", true
		}
		return "", "", false
	}
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
	const KB = 1024.0
	const MB = KB * 1024
	const GB = MB * 1024
	const TB = GB * 1024

	sign := ""
	b := float64(bytes)
	if b < 0 {
		sign = "-"
		b = -b
	}
	switch {
	case b >= TB:
		return fmt.Sprintf("%s%.1fTB", sign, b/TB)
	case b >= GB:
		return fmt.Sprintf("%s%.1fGB", sign, b/GB)
	case b >= MB:
		return fmt.Sprintf("%s%.1fMB", sign, b/MB)
	case b >= KB:
		return fmt.Sprintf("%s%.1fKB", sign, b/KB)
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
	idx := strings.LastIndex(withoutCents, "_")
	if idx < 0 {
		return ""
	}
	code := withoutCents[idx+1:]
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
// YAML Rendering
// ═══════════════════════════════════════════

func renderYamlProcessed(value any, indent int, lines *[]string) {
	prefix := strings.Repeat("  ", indent)
	m, ok := value.(map[string]any)
	if !ok {
		*lines = append(*lines, fmt.Sprintf("%s%s", prefix, yamlScalar(value)))
		return
	}

	for _, pf := range processObjectFields(m) {
		if pf.isFormatted {
			*lines = append(*lines, fmt.Sprintf("%s%s: \"%s\"", prefix, yamlKey(pf.key), escapeYamlStr(pf.formatted)))
		} else {
			switch v := pf.value.(type) {
			case map[string]any:
				if len(v) > 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, yamlKey(pf.key)))
					renderYamlProcessed(v, indent+1, lines)
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s: {}", prefix, yamlKey(pf.key)))
				}
			case []any:
				if len(v) == 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s: []", prefix, yamlKey(pf.key)))
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, yamlKey(pf.key)))
					for _, item := range v {
						if _, ok := item.(map[string]any); ok {
							*lines = append(*lines, fmt.Sprintf("%s  -", prefix))
							renderYamlProcessed(item, indent+2, lines)
						} else {
							*lines = append(*lines, fmt.Sprintf("%s  - %s", prefix, yamlScalar(item)))
						}
					}
				}
			default:
				*lines = append(*lines, fmt.Sprintf("%s%s: %s", prefix, yamlKey(pf.key), yamlScalar(pf.value)))
			}
		}
	}
}

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
		return "***"
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
