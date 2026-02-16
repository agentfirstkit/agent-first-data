package afd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func fixturesDir() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..", "spec", "fixtures")
}

func loadFixture(name string) []map[string]any {
	data, err := os.ReadFile(filepath.Join(fixturesDir(), name))
	if err != nil {
		panic(fmt.Sprintf("failed to read %s: %v", name, err))
	}
	var result []map[string]any
	if err := json.Unmarshal(data, &result); err != nil {
		panic(fmt.Sprintf("failed to parse %s: %v", name, err))
	}
	return result
}

// --- Redact fixtures ---

func TestRedactFixtures(t *testing.T) {
	for _, tc := range loadFixture("redact.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			// Deep copy via JSON round-trip
			b, _ := json.Marshal(tc["input"])
			var inp any
			json.Unmarshal(b, &inp)
			InternalRedactSecrets(inp)

			b2, _ := json.Marshal(tc["expected"])
			var expected any
			json.Unmarshal(b2, &expected)

			gotJSON, _ := json.Marshal(inp)
			expJSON, _ := json.Marshal(expected)
			if string(gotJSON) != string(expJSON) {
				t.Errorf("got %s, want %s", gotJSON, expJSON)
			}
		})
	}
}

// --- Protocol fixtures ---

func TestProtocolFixtures(t *testing.T) {
	for _, tc := range loadFixture("protocol.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			typ := tc["type"].(string)
			args := tc["args"].(map[string]any)

			var result map[string]any
			switch typ {
			case "ok":
				result = BuildJsonOk(args["result"], nil)
			case "ok_trace":
				result = BuildJsonOk(args["result"], args["trace"])
			case "error":
				result = BuildJsonError(args["message"].(string), nil)
			case "error_trace":
				result = BuildJsonError(args["message"].(string), args["trace"])
			case "startup":
				result = BuildJsonStartup(args["config"], args["args"], args["env"])
			case "status":
				result = BuildJson(args["code"].(string), args["fields"], nil)
			default:
				t.Fatalf("unknown type: %s", typ)
			}

			if expected, ok := tc["expected"]; ok {
				gotJSON, _ := json.Marshal(result)
				expJSON, _ := json.Marshal(expected)
				if string(gotJSON) != string(expJSON) {
					t.Errorf("got %s, want %s", gotJSON, expJSON)
				}
			}
			if ec, ok := tc["expected_contains"]; ok {
				ecMap := ec.(map[string]any)
				for k, v := range ecMap {
					gotJSON, _ := json.Marshal(result[k])
					expJSON, _ := json.Marshal(v)
					if string(gotJSON) != string(expJSON) {
						t.Errorf("key %s: got %s, want %s", k, gotJSON, expJSON)
					}
				}
			}
		})
	}
}

// --- Helper fixtures ---

func TestHelperFixtures(t *testing.T) {
	for _, tc := range loadFixture("helpers.json") {
		name := tc["name"].(string)
		cases := tc["cases"].([]any)

		switch name {
		case "format_bytes_human":
			for _, c := range cases {
				pair := c.([]any)
				input := int64(pair[0].(float64))
				expected := pair[1].(string)
				t.Run(fmt.Sprintf("bytes_%d", input), func(t *testing.T) {
					got := formatBytesHuman(input)
					if got != expected {
						t.Errorf("formatBytesHuman(%d) = %q, want %q", input, got, expected)
					}
				})
			}
		case "format_with_commas":
			for _, c := range cases {
				pair := c.([]any)
				input := uint64(pair[0].(float64))
				expected := pair[1].(string)
				t.Run(fmt.Sprintf("commas_%d", input), func(t *testing.T) {
					got := formatWithCommas(input)
					if got != expected {
						t.Errorf("formatWithCommas(%d) = %q, want %q", input, got, expected)
					}
				})
			}
		case "extract_currency_code":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				var expected string
				if pair[1] != nil {
					expected = pair[1].(string)
				}
				t.Run(fmt.Sprintf("currency_%s", input), func(t *testing.T) {
					got := extractCurrencyCode(input)
					if got != expected {
						t.Errorf("extractCurrencyCode(%q) = %q, want %q", input, got, expected)
					}
				})
			}
		case "parse_size":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				t.Run(fmt.Sprintf("parse_size_%s", input), func(t *testing.T) {
					got, ok := ParseSize(input)
					if pair[1] == nil {
						if ok {
							t.Errorf("ParseSize(%q) = %d, want error", input, got)
						}
					} else {
						expected := uint64(pair[1].(float64))
						if !ok || got != expected {
							t.Errorf("ParseSize(%q) = (%d, %v), want %d", input, got, ok, expected)
						}
					}
				})
			}
		}
	}
}

// --- Output JSON tests ---

func TestOutputJsonSingleLine(t *testing.T) {
	got := OutputJson(map[string]any{"a": 1, "b": 2})
	if got[0] != '{' || got[len(got)-1] != '}' {
		t.Errorf("expected JSON object, got %s", got)
	}
	for _, c := range got {
		if c == '\n' {
			t.Error("OutputJson should be single-line")
		}
	}
}

func TestOutputJsonSecretsRedacted(t *testing.T) {
	got := OutputJson(map[string]any{"api_key_secret": "sk-123", "name": "alice"})
	assertContains(t, got, `"api_key_secret":"***"`)
	assertContains(t, got, `"name":"alice"`)
}

func TestOutputJsonOriginalKeys(t *testing.T) {
	got := OutputJson(map[string]any{"latency_ms": 150})
	assertContains(t, got, `"latency_ms"`)
}

func TestOutputJsonRawValues(t *testing.T) {
	got := OutputJson(map[string]any{"latency_ms": 1500})
	assertContains(t, got, `"latency_ms":1500`)
}

func TestOutputJsonNonStringSecretRedacted(t *testing.T) {
	got := OutputJson(map[string]any{"count_secret": 42})
	assertContains(t, got, `"count_secret":"***"`)
}

func TestOutputJsonNestedSecretsRedacted(t *testing.T) {
	got := OutputJson(map[string]any{
		"trace": map[string]any{"api_key_secret": "sk-123", "duration_ms": 150},
	})
	assertContains(t, got, `"api_key_secret":"***"`)
	assertContains(t, got, `"duration_ms":150`)
}

// --- Output YAML tests ---

func TestOutputYamlStartsWithSeparator(t *testing.T) {
	got := OutputYaml(map[string]any{"a": 1})
	if len(got) < 3 || got[:3] != "---" {
		t.Errorf("expected YAML to start with ---, got %s", got)
	}
}

func TestOutputYamlStripMs(t *testing.T) {
	got := OutputYaml(map[string]any{"latency_ms": 150})
	assertContains(t, got, "latency:")
	assertNotContains(t, got, "latency_ms:")
}

func TestOutputYamlStripS(t *testing.T) {
	got := OutputYaml(map[string]any{"ttl_s": 3600})
	assertContains(t, got, "ttl:")
	assertNotContains(t, got, "ttl_s:")
}

func TestOutputYamlStripNs(t *testing.T) {
	got := OutputYaml(map[string]any{"pause_ns": 450000})
	assertContains(t, got, "pause:")
	assertNotContains(t, got, "pause_ns:")
}

func TestOutputYamlStripUs(t *testing.T) {
	got := OutputYaml(map[string]any{"query_us": 830})
	assertContains(t, got, "query:")
	assertNotContains(t, got, "query_us:")
}

func TestOutputYamlStripBytes(t *testing.T) {
	got := OutputYaml(map[string]any{"file_bytes": 5242880})
	assertContains(t, got, "file:")
	assertNotContains(t, got, "file_bytes:")
}

func TestOutputYamlStripEpochMs(t *testing.T) {
	got := OutputYaml(map[string]any{"created_epoch_ms": float64(1738886400000)})
	assertContains(t, got, "created:")
	assertNotContains(t, got, "created_epoch_ms:")
}

func TestOutputYamlStripEpochS(t *testing.T) {
	got := OutputYaml(map[string]any{"cached_epoch_s": float64(1707868800)})
	assertContains(t, got, "cached:")
	assertNotContains(t, got, "cached_epoch_s:")
}

func TestOutputYamlStripRfc3339(t *testing.T) {
	got := OutputYaml(map[string]any{"expires_rfc3339": "2026-02-14T10:30:00Z"})
	assertContains(t, got, "expires:")
	assertNotContains(t, got, "expires_rfc3339:")
}

func TestOutputYamlStripSecret(t *testing.T) {
	got := OutputYaml(map[string]any{"api_key_secret": "sk-123"})
	assertContains(t, got, "api_key:")
	assertNotContains(t, got, "api_key_secret:")
}

func TestOutputYamlStripPercent(t *testing.T) {
	got := OutputYaml(map[string]any{"cpu_percent": 85})
	assertContains(t, got, "cpu:")
	assertNotContains(t, got, "cpu_percent:")
}

func TestOutputYamlStripMsats(t *testing.T) {
	got := OutputYaml(map[string]any{"balance_msats": 50000})
	assertContains(t, got, "balance:")
	assertNotContains(t, got, "balance_msats:")
}

func TestOutputYamlStripSats(t *testing.T) {
	got := OutputYaml(map[string]any{"withdrawn_sats": 1234})
	assertContains(t, got, "withdrawn:")
	assertNotContains(t, got, "withdrawn_sats:")
}

func TestOutputYamlStripBtc(t *testing.T) {
	got := OutputYaml(map[string]any{"reserve_btc": 0.5})
	assertContains(t, got, "reserve:")
	assertNotContains(t, got, "reserve_btc:")
}

func TestOutputYamlStripUsdCents(t *testing.T) {
	got := OutputYaml(map[string]any{"price_usd_cents": 999})
	assertContains(t, got, "price:")
	assertNotContains(t, got, "price_usd_cents:")
}

func TestOutputYamlStripEurCents(t *testing.T) {
	got := OutputYaml(map[string]any{"price_eur_cents": 850})
	assertContains(t, got, "price:")
	assertNotContains(t, got, "price_eur_cents:")
}

func TestOutputYamlStripJpy(t *testing.T) {
	got := OutputYaml(map[string]any{"price_jpy": 1500})
	assertContains(t, got, "price:")
	assertNotContains(t, got, "price_jpy:")
}

func TestOutputYamlStripGenericCents(t *testing.T) {
	got := OutputYaml(map[string]any{"deposit_usdt_cents": 1000})
	assertContains(t, got, "deposit:")
	assertNotContains(t, got, "deposit_usdt_cents:")
}

func TestOutputYamlStripMinutes(t *testing.T) {
	got := OutputYaml(map[string]any{"timeout_minutes": 30})
	assertContains(t, got, "timeout:")
	assertNotContains(t, got, "timeout_minutes:")
}

func TestOutputYamlStripHours(t *testing.T) {
	got := OutputYaml(map[string]any{"validity_hours": 24})
	assertContains(t, got, "validity:")
	assertNotContains(t, got, "validity_hours:")
}

func TestOutputYamlStripDays(t *testing.T) {
	got := OutputYaml(map[string]any{"cert_days": 365})
	assertContains(t, got, "cert:")
	assertNotContains(t, got, "cert_days:")
}

func TestOutputYamlNoStripSize(t *testing.T) {
	got := OutputYaml(map[string]any{"buffer_size": "10M"})
	assertContains(t, got, "buffer_size:")
}

func TestOutputYamlNoStripNoSuffix(t *testing.T) {
	got := OutputYaml(map[string]any{"user_name": "alice"})
	assertContains(t, got, "user_name:")
}

func TestOutputYamlStripUppercaseSecret(t *testing.T) {
	got := OutputYaml(map[string]any{"API_KEY_SECRET": "sk-123"})
	assertContains(t, got, "API_KEY:")
	assertNotContains(t, got, "API_KEY_SECRET:")
}

func TestOutputYamlStripUppercaseS(t *testing.T) {
	got := OutputYaml(map[string]any{"CACHE_TTL_S": 3600})
	assertContains(t, got, "CACHE_TTL:")
	assertNotContains(t, got, "CACHE_TTL_S:")
}

// --- YAML value formatting tests ---

func TestOutputYamlFmtMsSmall(t *testing.T) {
	got := OutputYaml(map[string]any{"latency_ms": 150})
	assertContains(t, got, `"150ms"`)
}

func TestOutputYamlFmtMsToSeconds(t *testing.T) {
	got := OutputYaml(map[string]any{"latency_ms": 1280})
	assertContains(t, got, `"1.28s"`)
}

func TestOutputYamlFmtMs5000(t *testing.T) {
	got := OutputYaml(map[string]any{"latency_ms": 5000})
	assertContains(t, got, `"5.0s"`)
}

func TestOutputYamlFmtS(t *testing.T) {
	got := OutputYaml(map[string]any{"ttl_s": 3600})
	assertContains(t, got, `"3600s"`)
}

func TestOutputYamlFmtNs(t *testing.T) {
	got := OutputYaml(map[string]any{"pause_ns": 450000})
	assertContains(t, got, `"450000ns"`)
}

func TestOutputYamlFmtUs(t *testing.T) {
	got := OutputYaml(map[string]any{"query_us": 830})
	assertContains(t, got, "830\u03bcs")
}

func TestOutputYamlFmtMinutes(t *testing.T) {
	got := OutputYaml(map[string]any{"timeout_minutes": 30})
	assertContains(t, got, "30 minutes")
}

func TestOutputYamlFmtHours(t *testing.T) {
	got := OutputYaml(map[string]any{"validity_hours": 24})
	assertContains(t, got, "24 hours")
}

func TestOutputYamlFmtDays(t *testing.T) {
	got := OutputYaml(map[string]any{"cert_days": 365})
	assertContains(t, got, "365 days")
}

func TestOutputYamlFmtEpochMs(t *testing.T) {
	got := OutputYaml(map[string]any{"created_epoch_ms": float64(1738886400000)})
	assertContains(t, got, "2025-02-07T00:00:00.000Z")
}

func TestOutputYamlFmtEpochS(t *testing.T) {
	got := OutputYaml(map[string]any{"cached_epoch_s": float64(1707868800)})
	assertContains(t, got, "2024-02-14T00:00:00.000Z")
}

func TestOutputYamlFmtBytes(t *testing.T) {
	got := OutputYaml(map[string]any{"file_bytes": 5242880})
	assertContains(t, got, "5.0MB")
}

func TestOutputYamlFmtBytesKb(t *testing.T) {
	got := OutputYaml(map[string]any{"file_bytes": 456789})
	assertContains(t, got, "446.1KB")
}

func TestOutputYamlFmtUsdCents(t *testing.T) {
	got := OutputYaml(map[string]any{"price_usd_cents": 9999})
	assertContains(t, got, "$99.99")
}

func TestOutputYamlFmtEurCents(t *testing.T) {
	got := OutputYaml(map[string]any{"price_eur_cents": 850})
	assertContains(t, got, "\u20ac8.50")
}

func TestOutputYamlFmtJpy(t *testing.T) {
	got := OutputYaml(map[string]any{"price_jpy": 1500})
	assertContains(t, got, "\u00a51,500")
}

func TestOutputYamlFmtGenericCents(t *testing.T) {
	got := OutputYaml(map[string]any{"deposit_usdt_cents": 1000})
	assertContains(t, got, "10.00 USDT")
}

func TestOutputYamlFmtMsats(t *testing.T) {
	got := OutputYaml(map[string]any{"balance_msats": 50000})
	assertContains(t, got, "50000msats")
}

func TestOutputYamlFmtSats(t *testing.T) {
	got := OutputYaml(map[string]any{"withdrawn_sats": 1234})
	assertContains(t, got, "1234sats")
}

func TestOutputYamlFmtBtc(t *testing.T) {
	got := OutputYaml(map[string]any{"reserve_btc": 0.5})
	assertContains(t, got, "0.5 BTC")
}

func TestOutputYamlFmtPercentInt(t *testing.T) {
	got := OutputYaml(map[string]any{"cpu_percent": 85})
	assertContains(t, got, "85%")
}

func TestOutputYamlFmtPercentFloat(t *testing.T) {
	got := OutputYaml(map[string]any{"success_percent": 95.5})
	assertContains(t, got, "95.5%")
}

func TestOutputYamlFmtSecret(t *testing.T) {
	got := OutputYaml(map[string]any{"api_key_secret": "sk-123"})
	assertContains(t, got, `"***"`)
	assertNotContains(t, got, "sk-123")
}

func TestOutputYamlFmtRfc3339Passthrough(t *testing.T) {
	got := OutputYaml(map[string]any{"expires_rfc3339": "2026-02-14T10:30:00Z"})
	assertContains(t, got, "2026-02-14T10:30:00Z")
}

func TestOutputYamlFmtSizePassthrough(t *testing.T) {
	got := OutputYaml(map[string]any{"buffer_size": "10M"})
	assertContains(t, got, `buffer_size: "10M"`)
}

func TestOutputYamlStringsQuoted(t *testing.T) {
	got := OutputYaml(map[string]any{"name": "alice"})
	assertContains(t, got, `"alice"`)
}

func TestOutputYamlNumbersUnquoted(t *testing.T) {
	got := OutputYaml(map[string]any{"count": 42})
	assertContains(t, got, "count: 42")
}

func TestOutputYamlNestedKeyStripping(t *testing.T) {
	got := OutputYaml(map[string]any{
		"trace": map[string]any{"duration_ms": 1500, "source": "db"},
	})
	assertContains(t, got, "duration:")
	assertContains(t, got, `"1.5s"`)
}

// --- Collision tests ---

func TestOutputYamlCollisionKeepsOriginals(t *testing.T) {
	got := OutputYaml(map[string]any{"response_ms": 150, "response_bytes": 1024})
	assertContains(t, got, "response_ms:")
	assertContains(t, got, "response_bytes:")
	// Values should be raw, not formatted
	assertContains(t, got, "response_ms: 150")
	assertContains(t, got, "response_bytes: 1024")
}

func TestOutputPlainCollisionKeepsOriginals(t *testing.T) {
	got := OutputPlain(map[string]any{"response_ms": 150, "response_bytes": 1024})
	assertContains(t, got, "response_ms=150")
	assertContains(t, got, "response_bytes=1024")
}

// --- Output Plain tests ---

func TestOutputPlainSingleLine(t *testing.T) {
	got := OutputPlain(map[string]any{"a": 1, "b": 2})
	for _, c := range got {
		if c == '\n' {
			t.Error("OutputPlain should be single-line")
		}
	}
}

func TestOutputPlainKeyValuePair(t *testing.T) {
	got := OutputPlain(map[string]any{"name": "alice"})
	assertEqual(t, got, "name=alice")
}

func TestOutputPlainSortedKeys(t *testing.T) {
	got := OutputPlain(map[string]any{"z": 1, "a": 2, "m": 3})
	assertEqual(t, got, "a=2 m=3 z=1")
}

func TestOutputPlainDotNotation(t *testing.T) {
	got := OutputPlain(map[string]any{
		"trace": map[string]any{"source": "db"},
	})
	assertContains(t, got, "trace.source=db")
}

func TestOutputPlainQuotedSpaces(t *testing.T) {
	got := OutputPlain(map[string]any{"message": "hello world"})
	assertContains(t, got, `message="hello world"`)
}

func TestOutputPlainArraysCommaJoined(t *testing.T) {
	got := OutputPlain(map[string]any{"fields": []any{"email", "age"}})
	assertContains(t, got, "fields=email,age")
}

func TestOutputPlainNullEmpty(t *testing.T) {
	got := OutputPlain(map[string]any{"value": nil})
	assertContains(t, got, "value=")
}

func TestOutputPlainKeyStrippingAndFormatting(t *testing.T) {
	got := OutputPlain(map[string]any{"latency_ms": 1500})
	assertContains(t, got, "latency=1.5s")
}

func TestOutputPlainSecretsRedacted(t *testing.T) {
	got := OutputPlain(map[string]any{"api_key_secret": "sk-123", "name": "alice"})
	assertContains(t, got, "api_key=***")
	assertNotContains(t, got, "sk-123")
}

func TestOutputPlainEmptyObject(t *testing.T) {
	got := OutputPlain(map[string]any{})
	assertEqual(t, got, "")
}

func TestOutputPlainBoolUnquoted(t *testing.T) {
	got := OutputPlain(map[string]any{"enabled": true})
	assertContains(t, got, "enabled=true")
}

func TestOutputPlainNestedSecrets(t *testing.T) {
	got := OutputPlain(map[string]any{
		"trace": map[string]any{"api_key_secret": "sk-123", "duration_ms": 150},
	})
	assertContains(t, got, "trace.api_key=***")
	assertNotContains(t, got, "sk-123")
}

// --- Test helpers ---

func assertContains(t *testing.T, got, want string) {
	t.Helper()
	if !strings.Contains(got, want) {
		t.Errorf("expected %q to contain %q", got, want)
	}
}

func assertNotContains(t *testing.T, got, notWant string) {
	t.Helper()
	if strings.Contains(got, notWant) {
		t.Errorf("expected %q NOT to contain %q", got, notWant)
	}
}

func assertEqual(t *testing.T, got, want string) {
	t.Helper()
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
