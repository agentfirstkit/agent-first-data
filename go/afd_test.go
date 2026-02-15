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

func asStringSlice(v any) []string {
	arr, ok := v.([]any)
	if !ok {
		return nil
	}
	result := make([]string, len(arr))
	for i, item := range arr {
		result[i] = item.(string)
	}
	return result
}

// --- Plain fixtures ---

func TestPlainFixtures(t *testing.T) {
	for _, tc := range loadFixture("plain.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			plain := ToPlain(tc["input"])
			for _, s := range asStringSlice(tc["contains"]) {
				if !strings.Contains(plain, s) {
					t.Errorf("expected %q in %q", s, plain)
				}
			}
			if nc, ok := tc["not_contains"]; ok {
				for _, s := range asStringSlice(nc) {
					if strings.Contains(plain, s) {
						t.Errorf("unexpected %q in %q", s, plain)
					}
				}
			}
		})
	}
}

// --- YAML fixtures ---

func TestYamlFixtures(t *testing.T) {
	for _, tc := range loadFixture("yaml.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			yaml := ToYAML(tc["input"])
			if sw, ok := tc["starts_with"]; ok {
				if !strings.HasPrefix(yaml, sw.(string)) {
					t.Errorf("expected starts_with %q, got %q", sw, yaml)
				}
			}
			if contains, ok := tc["contains"]; ok {
				for _, s := range asStringSlice(contains) {
					if !strings.Contains(yaml, s) {
						t.Errorf("expected %q in %q", s, yaml)
					}
				}
			}
		})
	}
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
			RedactSecrets(inp)

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
				result = Ok(args["result"])
			case "ok_trace":
				result = OkTrace(args["result"], args["trace"])
			case "error":
				result = Error(args["message"].(string))
			case "error_trace":
				result = ErrorTrace(args["message"].(string), args["trace"])
			case "startup":
				result = Startup(args["config"], args["args"], args["env"])
			case "status":
				fields := make(map[string]any)
				if f, ok := args["fields"].(map[string]any); ok {
					fields = f
				}
				result = Status(args["code"].(string), fields)
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

// --- Exact fixtures ---

func TestExactFixtures(t *testing.T) {
	for _, tc := range loadFixture("exact.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			format := tc["format"].(string)
			expected := tc["expected"].(string)
			var got string
			switch format {
			case "plain":
				got = ToPlain(tc["input"])
			case "yaml":
				got = ToYAML(tc["input"])
			default:
				t.Fatalf("unknown format: %s", format)
			}
			if got != expected {
				t.Errorf("got %q, want %q", got, expected)
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

// --- OutputFormat (not in fixtures) ---

func TestOutputFormatJSON(t *testing.T) {
	m := map[string]any{"status": "ok"}
	out := JSON.Format(m)
	if out != `{"status":"ok"}` {
		t.Errorf("got %q", out)
	}
}

func TestOutputFormatYAML(t *testing.T) {
	m := map[string]any{"status": "ok"}
	out := YAML.Format(m)
	if !strings.HasPrefix(out, "---\n") || !strings.Contains(out, `status: "ok"`) {
		t.Errorf("got %q", out)
	}
}

func TestOutputFormatPlain(t *testing.T) {
	m := map[string]any{"status": "ok"}
	out := Plain.Format(m)
	if out != "status: ok" {
		t.Errorf("got %q", out)
	}
}
