// Package afdata implements Agent-First Data (AFDATA) output formatting
// and protocol templates.
//
// 12 public APIs and 1 type: 3 protocol builders + 3 output formatters +
// 1 redaction + 1 utility + 4 CLI helpers + OutputFormat.
package afdata

import (
	"encoding/json"
	"fmt"
	"math"
	"math/bits"
	"sort"
	"strconv"
	"strings"
	"time"
	"unicode/utf16"
)

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

// BuildJsonError builds {code: "error", error: message, trace?}.
func BuildJsonError(message string, trace any) map[string]any {
	m := map[string]any{"code": "error", "error": message}
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

// OutputJson formats as single-line JSON. Secrets redacted, original keys, raw values.
func OutputJson(value any) string {
	// Deep copy via JSON round-trip
	b, _ := json.Marshal(value)
	var v any
	json.Unmarshal(b, &v)
	redactSecrets(v)
	out, _ := json.Marshal(v)
	return string(out)
}

// OutputYaml formats as multi-line YAML. Keys stripped, values formatted, secrets redacted.
func OutputYaml(value any) string {
	lines := []string{"---"}
	renderYamlProcessed(normalize(value), 0, &lines)
	return strings.Join(lines, "\n")
}

// OutputPlain formats as single-line logfmt. Keys stripped, values formatted, secrets redacted.
func OutputPlain(value any) string {
	var pairs [][2]string
	collectPlainPairs(normalize(value), "", &pairs)
	sort.Slice(pairs, func(i, j int) bool {
		return jcsLess(pairs[i][0], pairs[j][0])
	})
	parts := make([]string, len(pairs))
	for i, p := range pairs {
		if strings.Contains(p[1], " ") {
			parts[i] = fmt.Sprintf("%s=\"%s\"", p[0], p[1])
		} else {
			parts[i] = fmt.Sprintf("%s=%s", p[0], p[1])
		}
	}
	return strings.Join(parts, " ")
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

// InternalRedactSecrets redacts _secret fields in-place.
func InternalRedactSecrets(value any) {
	redactSecrets(value)
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
	if numStr == "" {
		return 0, false
	}
	if n, err := strconv.ParseUint(numStr, 10, 64); err == nil {
		hi, lo := bits.Mul64(n, mult)
		if hi != 0 {
			return 0, false
		}
		return lo, true
	}
	f, err := strconv.ParseFloat(numStr, 64)
	if err != nil || f < 0 || math.IsNaN(f) || math.IsInf(f, 0) {
		return 0, false
	}
	result := f * float64(mult)
	if result > float64(math.MaxUint64) {
		return 0, false
	}
	return uint64(result), true
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

func redactSecrets(value any) {
	switch v := value.(type) {
	case map[string]any:
		for k := range v {
			if strings.HasSuffix(k, "_secret") || strings.HasSuffix(k, "_SECRET") {
				switch v[k].(type) {
				case map[string]any, []any:
					// Traverse containers, don't replace
					redactSecrets(v[k])
				default:
					v[k] = "***"
				}
			} else {
				redactSecrets(v[k])
			}
		}
	case []any:
		for _, item := range v {
			redactSecrets(item)
		}
	}
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
			return stripped, formatRFC3339Ms(n), true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_epoch_s"); ok {
		if n, ok := asInt64(value); ok {
			return stripped, formatRFC3339Ms(n * 1000), true
		}
		return "", "", false
	}
	if stripped, ok := stripSuffixCI(key, "_epoch_ns"); ok {
		if n, ok := asInt64(value); ok {
			ms := n / 1_000_000
			if n%1_000_000 < 0 {
				ms--
			}
			return stripped, formatRFC3339Ms(ms), true
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
	if stripped, ok := stripSuffixCI(key, "_secret"); ok {
		return stripped, "***", true
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

func formatRFC3339Ms(ms int64) string {
	sec := ms / 1000
	rem := ms % 1000
	if rem < 0 {
		sec--
		rem += 1000
	}
	nsec := rem * 1_000_000
	t := time.Unix(sec, nsec).UTC()
	return t.Format("2006-01-02T15:04:05.000Z")
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
	if code == "" {
		return ""
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
			*lines = append(*lines, fmt.Sprintf("%s%s: \"%s\"", prefix, pf.key, escapeYamlStr(pf.formatted)))
		} else {
			switch v := pf.value.(type) {
			case map[string]any:
				if len(v) > 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, pf.key))
					renderYamlProcessed(v, indent+1, lines)
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s: {}", prefix, pf.key))
				}
			case []any:
				if len(v) == 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s: []", prefix, pf.key))
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, pf.key))
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
				*lines = append(*lines, fmt.Sprintf("%s%s: %s", prefix, pf.key, yamlScalar(pf.value)))
			}
		}
	}
}

func escapeYamlStr(s string) string {
	s = strings.ReplaceAll(s, `\`, `\\`)
	s = strings.ReplaceAll(s, `"`, `\"`)
	s = strings.ReplaceAll(s, "\n", `\n`)
	s = strings.ReplaceAll(s, "\r", `\r`)
	s = strings.ReplaceAll(s, "\t", `\t`)
	return s
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
		if v == math.Trunc(v) && !math.IsInf(v, 0) && math.Abs(v) < 1e15 {
			return fmt.Sprintf("%.0f", v)
		}
		return strconv.FormatFloat(v, 'f', -1, 64)
	case json.Number:
		return v.String()
	default:
		return fmt.Sprintf(`"%v"`, value)
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
		if v == math.Trunc(v) && !math.IsInf(v, 0) && math.Abs(v) < 1e15 {
			return fmt.Sprintf("%.0f", v)
		}
		return strconv.FormatFloat(v, 'f', -1, 64)
	case json.Number:
		return v.String()
	default:
		return fmt.Sprintf("%v", value)
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
	var result any
	if err := json.Unmarshal(b, &result); err != nil {
		return value
	}
	return result
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
