// Package afd implements Agent-First Data (AFD) output formatting
// and protocol templates.
package afd

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

// OutputFormat selects the rendering mode.
type OutputFormat int

const (
	JSON  OutputFormat = iota
	YAML
	Plain
)

// Format renders a value as a compact string.
func (f OutputFormat) Format(value any) string {
	switch f {
	case JSON:
		b, _ := json.Marshal(value)
		return string(b)
	case YAML:
		return ToYAML(value)
	case Plain:
		return ToPlain(value)
	}
	return ""
}

// FormatPretty renders a value with indentation (JSON only).
func (f OutputFormat) FormatPretty(value any) string {
	switch f {
	case JSON:
		b, _ := json.MarshalIndent(value, "", "  ")
		return string(b)
	case YAML:
		return ToYAML(value)
	case Plain:
		return ToPlain(value)
	}
	return ""
}

// ═══════════════════════════════════════════
// YAML
// ═══════════════════════════════════════════

// ToYAML converts a value to a YAML document string.
func ToYAML(value any) string {
	lines := []string{"---"}
	renderYAML(normalize(value), 0, &lines)
	return strings.Join(lines, "\n")
}

func renderYAML(value any, indent int, lines *[]string) {
	prefix := strings.Repeat("  ", indent)
	switch v := value.(type) {
	case map[string]any:
		for _, k := range sortedKeys(v) {
			val := v[k]
			switch inner := val.(type) {
			case map[string]any:
				if len(inner) > 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, k))
					renderYAML(inner, indent+1, lines)
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s: {}", prefix, k))
				}
			case []any:
				if len(inner) == 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s: []", prefix, k))
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, k))
					for _, item := range inner {
						if m, ok := item.(map[string]any); ok {
							*lines = append(*lines, fmt.Sprintf("%s  -", prefix))
							renderYAML(m, indent+2, lines)
						} else {
							*lines = append(*lines, fmt.Sprintf("%s  - %s", prefix, yamlScalar(item)))
						}
					}
				}
			default:
				*lines = append(*lines, fmt.Sprintf("%s%s: %s", prefix, k, yamlScalar(val)))
			}
		}
	default:
		*lines = append(*lines, fmt.Sprintf("%s%s", prefix, yamlScalar(value)))
	}
}

func yamlScalar(value any) string {
	switch v := value.(type) {
	case string:
		escaped := strings.ReplaceAll(v, `\`, `\\`)
		escaped = strings.ReplaceAll(escaped, `"`, `\"`)
		escaped = strings.ReplaceAll(escaped, "\n", `\n`)
		escaped = strings.ReplaceAll(escaped, "\r", `\r`)
		escaped = strings.ReplaceAll(escaped, "\t", `\t`)
		return fmt.Sprintf(`"%s"`, escaped)
	case nil:
		return "null"
	case bool:
		if v {
			return "true"
		}
		return "false"
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
// Plain
// ═══════════════════════════════════════════

// ToPlain converts a value to human-readable plain text with suffix-driven formatting.
func ToPlain(value any) string {
	lines := []string{}
	renderPlain(normalize(value), 0, &lines)
	return strings.Join(lines, "\n")
}

func renderPlain(value any, indent int, lines *[]string) {
	prefix := strings.Repeat("  ", indent)
	switch v := value.(type) {
	case map[string]any:
		for _, k := range sortedKeys(v) {
			val := v[k]
			switch inner := val.(type) {
			case map[string]any:
				*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, k))
				renderPlain(inner, indent+1, lines)
			case []any:
				if len(inner) == 0 {
					*lines = append(*lines, fmt.Sprintf("%s%s: []", prefix, k))
				} else if allScalar(inner) {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, k))
					for _, item := range inner {
						*lines = append(*lines, fmt.Sprintf("%s  - %s", prefix, plainScalar(item)))
					}
				} else {
					*lines = append(*lines, fmt.Sprintf("%s%s:", prefix, k))
					for _, item := range inner {
						if m, ok := item.(map[string]any); ok {
							*lines = append(*lines, fmt.Sprintf("%s  -", prefix))
							renderPlain(m, indent+2, lines)
						} else {
							*lines = append(*lines, fmt.Sprintf("%s  - %s", prefix, plainScalar(item)))
						}
					}
				}
			default:
				*lines = append(*lines, fmt.Sprintf("%s%s: %s", prefix, k, formatPlainField(k, val)))
			}
		}
	default:
		*lines = append(*lines, fmt.Sprintf("%s%s", prefix, plainScalar(value)))
	}
}

func formatPlainField(key string, value any) string {
	lower := strings.ToLower(key)

	// Secret
	if strings.HasSuffix(lower, "_secret") {
		return "***"
	}

	// Timestamps
	if strings.HasSuffix(lower, "_epoch_ms") {
		if n, ok := asInt64(value); ok {
			return formatRFC3339Ms(n)
		}
	}
	if strings.HasSuffix(lower, "_epoch_s") {
		if n, ok := asInt64(value); ok {
			return formatRFC3339Ms(n * 1000)
		}
	}
	if strings.HasSuffix(lower, "_epoch_ns") {
		if n, ok := asInt64(value); ok {
			ms := n / 1_000_000
			if n%1_000_000 < 0 {
				ms--
			}
			return formatRFC3339Ms(ms)
		}
	}
	if strings.HasSuffix(lower, "_rfc3339") {
		return plainScalar(value)
	}

	// Size
	if strings.HasSuffix(lower, "_bytes") {
		if n, ok := asInt64(value); ok {
			return formatBytesHuman(n)
		}
	}

	// Percentage
	if strings.HasSuffix(lower, "_percent") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "%"
		}
	}

	// Currency — Bitcoin
	if strings.HasSuffix(lower, "_msats") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "msats"
		}
	}
	if strings.HasSuffix(lower, "_sats") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "sats"
		}
	}
	if strings.HasSuffix(lower, "_btc") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + " BTC"
		}
	}

	// Currency — Fiat
	if strings.HasSuffix(lower, "_usd_cents") {
		if n, ok := asNonNegInt64(value); ok {
			return fmt.Sprintf("$%d.%02d", n/100, n%100)
		}
	}
	if strings.HasSuffix(lower, "_eur_cents") {
		if n, ok := asNonNegInt64(value); ok {
			return fmt.Sprintf("\u20ac%d.%02d", n/100, n%100)
		}
	}
	if strings.HasSuffix(lower, "_jpy") {
		if n, ok := asNonNegInt64(value); ok {
			return fmt.Sprintf("\u00a5%s", formatWithCommas(uint64(n)))
		}
	}
	if strings.HasSuffix(lower, "_cents") {
		if code := extractCurrencyCode(lower); code != "" {
			if n, ok := asNonNegInt64(value); ok {
				return fmt.Sprintf("%d.%02d %s", n/100, n%100, strings.ToUpper(code))
			}
		}
	}

	// Duration — long
	if strings.HasSuffix(lower, "_minutes") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + " minutes"
		}
	}
	if strings.HasSuffix(lower, "_hours") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + " hours"
		}
	}
	if strings.HasSuffix(lower, "_days") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + " days"
		}
	}

	// Duration — ms
	if strings.HasSuffix(lower, "_ms") && !strings.HasSuffix(lower, "_epoch_ms") {
		if n, ok := asFloat64(value); ok {
			if n >= 1000 {
				return fmt.Sprintf("%.2fs", n/1000)
			}
			return plainScalar(value) + "ms"
		}
	}

	// Duration — ns, us, s
	if strings.HasSuffix(lower, "_ns") && !strings.HasSuffix(lower, "_epoch_ns") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "ns"
		}
	}
	if strings.HasSuffix(lower, "_us") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "\u03bcs"
		}
	}
	if strings.HasSuffix(lower, "_s") && !strings.HasSuffix(lower, "_epoch_s") {
		if _, ok := asFloat64(value); ok {
			return plainScalar(value) + "s"
		}
	}

	return plainScalar(value)
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
// Secret redaction
// ═══════════════════════════════════════════

// RedactSecrets walks a map/slice tree and replaces any string value
// whose key ends in "_secret" with "***".
func RedactSecrets(value any) {
	switch v := value.(type) {
	case map[string]any:
		for k := range v {
			if strings.HasSuffix(strings.ToLower(k), "_secret") {
				if _, ok := v[k].(string); ok {
					v[k] = "***"
				}
			}
			RedactSecrets(v[k])
		}
	case []any:
		for _, item := range v {
			RedactSecrets(item)
		}
	}
}

// ═══════════════════════════════════════════
// AFD Protocol templates
// ═══════════════════════════════════════════

// Ok builds {"code": "ok", "result": ...}.
func Ok(result any) map[string]any {
	return map[string]any{"code": "ok", "result": result}
}

// OkTrace builds {"code": "ok", "result": ..., "trace": ...}.
func OkTrace(result, trace any) map[string]any {
	return map[string]any{"code": "ok", "result": result, "trace": trace}
}

// Error builds {"code": "error", "error": "message"}.
func Error(message string) map[string]any {
	return map[string]any{"code": "error", "error": message}
}

// ErrorTrace builds {"code": "error", "error": "message", "trace": ...}.
func ErrorTrace(message string, trace any) map[string]any {
	return map[string]any{"code": "error", "error": message, "trace": trace}
}

// Startup builds {"code": "startup", "config": ..., "args": ..., "env": ...}.
func Startup(config, args, env any) map[string]any {
	return map[string]any{"code": "startup", "config": config, "args": args, "env": env}
}

// Status builds {"code": "<custom>", ...fields}.
func Status(code string, fields map[string]any) map[string]any {
	result := make(map[string]any, len(fields)+1)
	for k, v := range fields {
		result[k] = v
	}
	result["code"] = code
	return result
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

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

func extractCurrencyCode(key string) string {
	if !strings.HasSuffix(key, "_cents") {
		return ""
	}
	withoutCents := strings.TrimSuffix(key, "_cents")
	idx := strings.LastIndex(withoutCents, "_")
	if idx < 0 {
		return ""
	}
	return withoutCents[idx+1:]
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

func asInt64(value any) (int64, bool) {
	switch v := value.(type) {
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
	case float64:
		return v, true
	case json.Number:
		if n, err := v.Float64(); err == nil {
			return n, true
		}
	}
	return 0, false
}

func allScalar(items []any) bool {
	for _, item := range items {
		switch item.(type) {
		case map[string]any, []any:
			return false
		}
	}
	return true
}

// normalize converts a Go value through JSON round-trip to get map[string]any.
func normalize(value any) any {
	switch value.(type) {
	case map[string]any:
		return value
	case []any:
		return value
	case string, float64, bool, nil, json.Number:
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

// sortedKeys returns keys sorted by UTF-16 code unit order (JCS, RFC 8785).
func sortedKeys(m map[string]any) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool {
		return jcsLess(keys[i], keys[j])
	})
	return keys
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
