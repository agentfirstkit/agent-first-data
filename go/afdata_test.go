package afdata

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

func loadFixtureObject(name string) map[string]any {
	data, err := os.ReadFile(filepath.Join(fixturesDir(), name))
	if err != nil {
		panic(fmt.Sprintf("failed to read %s: %v", name, err))
	}
	var result map[string]any
	if err := json.Unmarshal(data, &result); err != nil {
		panic(fmt.Sprintf("failed to parse %s: %v", name, err))
	}
	return result
}

func redactorFromCase(tc map[string]any) Redactor {
	opts, _ := tc["options"].(map[string]any)
	redactor := Redactor{}
	if policy, ok := opts["policy"].(string); ok {
		switch policy {
		case "TraceOnly":
			redactor.Policy = RedactionTraceOnly
		case "Off":
			redactor.Policy = RedactionOff
		default:
			panic(fmt.Sprintf("unknown redaction policy: %s", policy))
		}
	}
	if names, ok := opts["secret_names"].([]any); ok {
		for _, name := range names {
			redactor.SecretNames = append(redactor.SecretNames, name.(string))
		}
	}
	return redactor
}

type secretStringFunc func()

func (secretStringFunc) String() string {
	return "sk-live-123"
}

// --- Redact fixtures ---

func TestRedactURLFixtures(t *testing.T) {
	for _, tc := range loadFixture("redact_url.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			input := tc["input"].(string)
			expected := tc["expected"].(string)
			options := redactorFromCase(tc)
			got := options.URL(input)
			if got != expected {
				t.Errorf("got %q, want %q", got, expected)
			}
		})
	}
}

func TestRedactFixtures(t *testing.T) {
	for _, tc := range loadFixture("redact.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			// Use RedactedValue instead of in-place redaction
			got := RedactedValue(tc["input"])

			b2, _ := json.Marshal(tc["expected"])
			var expected any
			json.Unmarshal(b2, &expected)

			gotJSON, _ := json.Marshal(got)
			expJSON, _ := json.Marshal(expected)
			if string(gotJSON) != string(expJSON) {
				t.Errorf("got %s, want %s", gotJSON, expJSON)
			}
		})
	}
}

func TestRedactionOptionsFixtures(t *testing.T) {
	for _, tc := range loadFixture("redaction_options.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			options := redactorFromCase(tc)
			outputOptions := OutputOptions{
				Redaction: options,
				Style:     PlainStyleReadable,
			}
			expected := tc["expected"]

			got := options.Value(tc["input"])
			gotJSON, _ := json.Marshal(got)
			expJSON, _ := json.Marshal(expected)
			if string(gotJSON) != string(expJSON) {
				t.Errorf("redacted value got %s, want %s", gotJSON, expJSON)
			}

			jsonLine := Render(tc["input"], OutputFormatJson, outputOptions)
			var gotOutput any
			if err := json.Unmarshal([]byte(jsonLine), &gotOutput); err != nil {
				t.Fatalf("invalid JSON output: %v (%s)", err, jsonLine)
			}
			gotJSON, _ = json.Marshal(gotOutput)
			if string(gotJSON) != string(expJSON) {
				t.Errorf("json output got %s, want %s", gotJSON, expJSON)
			}

			if expectedYAML, ok := tc["expected_yaml"].(string); ok {
				if got := Render(tc["input"], OutputFormatYaml, outputOptions); got != expectedYAML {
					t.Errorf("yaml got %q, want %q", got, expectedYAML)
				}
			}
			if expectedPlain, ok := tc["expected_plain"].(string); ok {
				if got := Render(tc["input"], OutputFormatPlain, outputOptions); got != expectedPlain {
					t.Errorf("plain got %q, want %q", got, expectedPlain)
				}
			}
		})
	}
}

func TestSecurityFixtures(t *testing.T) {
	fixture := loadFixtureObject("security.json")
	for _, tc := range mapSliceFromAny(t, fixture["redaction_cases"]) {
		name := tc["name"].(string)
		t.Run("redaction/"+name, func(t *testing.T) {
			options := redactorFromCase(tc)
			outputOptions := OutputOptions{Redaction: options, Style: PlainStyleReadable}
			expected := tc["expected"]
			got := options.Value(tc["input"])
			gotJSON, _ := json.Marshal(got)
			expJSON, _ := json.Marshal(expected)
			if string(gotJSON) != string(expJSON) {
				t.Fatalf("redacted value got %s, want %s", gotJSON, expJSON)
			}
			outputs := []string{
				Render(tc["input"], OutputFormatJson, outputOptions),
				Render(tc["input"], OutputFormatYaml, outputOptions),
				Render(tc["input"], OutputFormatPlain, outputOptions),
			}
			for _, output := range outputs {
				for _, needle := range stringSliceFromAny(t, tc["must_contain"]) {
					if !strings.Contains(output, needle) {
						t.Fatalf("output missing %q: %s", needle, output)
					}
				}
				for _, needle := range stringSliceFromAny(t, tc["must_not_contain"]) {
					if strings.Contains(output, needle) {
						t.Fatalf("output leaked %q: %s", needle, output)
					}
				}
			}
		})
	}
}

func mapSliceFromAny(t *testing.T, value any) []map[string]any {
	t.Helper()
	items, ok := value.([]any)
	if !ok {
		t.Fatalf("value must be array: %#v", value)
	}
	result := make([]map[string]any, 0, len(items))
	for _, item := range items {
		m, ok := item.(map[string]any)
		if !ok {
			t.Fatalf("value must contain objects: %#v", value)
		}
		result = append(result, m)
	}
	return result
}

func stringSliceFromAny(t *testing.T, value any) []string {
	t.Helper()
	items, ok := value.([]any)
	if !ok {
		t.Fatalf("value must be array: %#v", value)
	}
	result := make([]string, 0, len(items))
	for _, item := range items {
		s, ok := item.(string)
		if !ok {
			t.Fatalf("value must contain strings: %#v", value)
		}
		result = append(result, s)
	}
	return result
}

// --- Protocol fixtures ---

func TestProtocolFixtures(t *testing.T) {
	for _, tc := range loadFixture("protocol.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			if invalid, ok := tc["invalid"]; ok {
				if err := ValidateProtocolEvent(invalid, false); err == nil {
					t.Fatalf("invalid event unexpectedly passed")
				}
				return
			}
			typ := tc["type"].(string)
			args := tc["args"].(map[string]any)

			var result map[string]any
			switch typ {
			case "result":
				event := NewJSONResult(args["result"]).Build()
				result = event.Value()
			case "result_trace":
				event := NewJSONResult(args["result"]).Trace(args["trace"]).Build()
				result = event.Value()
			case "error":
				event, _ := NewJSONError(args["code"].(string), args["message"].(string)).Build()
				result = event.Value()
			case "error_trace":
				event, _ := NewJSONError(args["code"].(string), args["message"].(string)).
					Trace(args["trace"]).Build()
				result = event.Value()
			case "error_hint":
				hint := ""
				if h, ok := args["hint"].(string); ok {
					hint = h
				}
				event, _ := NewJSONError(args["code"].(string), args["message"].(string)).
					Hint(hint).Build()
				result = event.Value()
			case "error_retryable":
				retryable := false
				if r, ok := args["retryable"].(bool); ok {
					retryable = r
				}
				event, _ := NewJSONError(args["code"].(string), args["message"].(string)).
					RetryableIf(retryable).Build()
				result = event.Value()
			case "error_extension_fields":
				fields := args["fields"].(map[string]any)
				event, _ := NewJSONError(args["code"].(string), args["message"].(string)).
					Fields(fields).Build()
				result = event.Value()
			case "progress":
				payload := map[string]any{"message": args["message"]}
				if fields, ok := args["fields"].(map[string]any); ok {
					for key, value := range fields {
						payload[key] = value
					}
				}
				event := NewJSONProgress(payload).Build()
				result = event.Value()
			case "log":
				payload := map[string]any{"level": args["level"], "message": args["message"]}
				if fields, ok := args["fields"].(map[string]any); ok {
					for key, value := range fields {
						payload[key] = value
					}
				}
				event := NewJSONLog(payload).Build()
				result = event.Value()
			default:
				t.Fatalf("unknown type: %s", typ)
			}

			if err := ValidateProtocolEvent(result, true); err != nil {
				t.Fatalf("invalid event: %v", err)
			}
			if expected, ok := tc["expected"]; ok {
				// 0.16: All builder cases must pass deep equality against expected.
				exp := expected.(map[string]any)
				gotJSON, _ := json.Marshal(result)
				expJSON, _ := json.Marshal(exp)
				if string(gotJSON) != string(expJSON) {
					t.Errorf("got %s, want %s", gotJSON, expJSON)
				}
			}
		})
	}
}

func TestProtocolStreamFixtures(t *testing.T) {
	for _, tc := range loadFixture("protocol_streams.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			valid := tc["valid"].(bool)
			events := tc["events"].([]any)
			err := ValidateProtocolStream(events, false)
			if valid && err != nil {
				t.Fatalf("valid stream failed: %v", err)
			}
			if !valid && err == nil {
				t.Fatalf("invalid stream unexpectedly passed")
			}
		})
	}
}

func TestProtocolStrictFixtures(t *testing.T) {
	for _, tc := range loadFixture("protocol_strict.json") {
		events := tc["events"].([]any)
		if got := ValidateProtocolStream(events, true) == nil; got != tc["valid"].(bool) {
			t.Errorf("%s: valid=%v", tc["name"], got)
		}
	}
}

func TestErrorBuilderRejectsReservedExtensionFields(t *testing.T) {
	// In 0.16, reserved field writes are errors returned by Build(), not silently ignored
	builder := NewJSONError("explicit", "message").
		Fields(map[string]any{
			"code": "wrong", "message": "wrong", "hint": "wrong", "detail": 1,
		})
	_, err := builder.Build()
	if err == nil {
		t.Fatalf("expected error for reserved field, got nil")
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
		case "normalize_utc_offset":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				t.Run(fmt.Sprintf("normalize_utc_offset_%s", input), func(t *testing.T) {
					got, ok := NormalizeUTCOffset(input)
					if pair[1] == nil {
						if ok {
							t.Errorf("NormalizeUTCOffset(%q) = %q, want error", input, got)
						}
					} else {
						expected := pair[1].(string)
						if !ok || got != expected {
							t.Errorf("NormalizeUTCOffset(%q) = (%q, %v), want %q", input, got, ok, expected)
						}
					}
				})
			}
		case "is_valid_rfc3339_date":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				expected := pair[1].(bool)
				t.Run(fmt.Sprintf("is_valid_rfc3339_date_%s", input), func(t *testing.T) {
					got := IsValidRFC3339Date(input)
					if got != expected {
						t.Errorf("IsValidRFC3339Date(%q) = %v, want %v", input, got, expected)
					}
				})
			}
		case "is_valid_rfc3339_time":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				expected := pair[1].(bool)
				t.Run(fmt.Sprintf("is_valid_rfc3339_time_%s", input), func(t *testing.T) {
					got := IsValidRFC3339Time(input)
					if got != expected {
						t.Errorf("IsValidRFC3339Time(%q) = %v, want %v", input, got, expected)
					}
				})
			}
		case "is_valid_bcp47":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				expected := pair[1].(bool)
				t.Run(fmt.Sprintf("is_valid_bcp47_%s", input), func(t *testing.T) {
					got := IsValidBCP47(input)
					if got != expected {
						t.Errorf("IsValidBCP47(%q) = %v, want %v", input, got, expected)
					}
				})
			}
		case "is_valid_rfc3339":
			for _, c := range cases {
				pair := c.([]any)
				input := pair[0].(string)
				expected := pair[1].(bool)
				t.Run(fmt.Sprintf("is_valid_rfc3339_%s", input), func(t *testing.T) {
					got := IsValidRFC3339(input)
					if got != expected {
						t.Errorf("IsValidRFC3339(%q) = %v, want %v", input, got, expected)
					}
				})
			}
		}
	}
}

func TestOutputFormatFixtures(t *testing.T) {
	for _, tc := range loadFixture("output_formats.json") {
		name := tc["name"].(string)
		t.Run(name, func(t *testing.T) {
			input := tc["input"]
			expectedJSON := tc["expected_json"]
			expectedYAML := tc["expected_yaml"].(string)
			expectedPlain := tc["expected_plain"].(string)

			jsonLine := Render(input, OutputFormatJson, OutputOptions{})
			var gotJSON any
			if err := json.Unmarshal([]byte(jsonLine), &gotJSON); err != nil {
				t.Fatalf("invalid JSON output: %v (%s)", err, jsonLine)
			}
			gotJSONBytes, _ := json.Marshal(gotJSON)
			expJSONBytes, _ := json.Marshal(expectedJSON)
			if string(gotJSONBytes) != string(expJSONBytes) {
				t.Errorf("json mismatch: got %s, want %s", gotJSONBytes, expJSONBytes)
			}

			if got := Render(input, OutputFormatYaml, OutputOptions{}); got != expectedYAML {
				t.Errorf("yaml mismatch: got %q, want %q", got, expectedYAML)
			}
			if got := Render(input, OutputFormatPlain, OutputOptions{}); got != expectedPlain {
				t.Errorf("plain mismatch: got %q, want %q", got, expectedPlain)
			}
		})
	}
}

func TestOutputYamlRawKeepsSuffixKeysAndStructure(t *testing.T) {
	options := OutputOptions{
		Redaction: Redactor{Policy: RedactionTraceOnly},
		Style:     PlainStyleRaw,
	}
	out := Render(map[string]any{
		"code": "result",
		"rows": []any{map[string]any{
			"api_key_secret": "sk-live-1",
			"duration_ms":    int64(42),
		}},
		"trace": map[string]any{"request_secret": "top-secret"},
	}, OutputFormatYaml, options)

	assertContains(t, out, "rows:\n  -")
	assertContains(t, out, `api_key_secret: "sk-live-1"`)
	assertContains(t, out, "duration_ms: 42")
	assertContains(t, out, `request_secret: "***"`)
	assertNotContains(t, out, `duration: "42ms"`)
}

func TestOutputPlainRawKeepsSuffixKeysAndRedactsTrace(t *testing.T) {
	options := OutputOptions{
		Redaction: Redactor{Policy: RedactionTraceOnly},
		Style:     PlainStyleRaw,
	}
	out := Render(map[string]any{
		"duration_ms": int64(42),
		"trace":       map[string]any{"request_secret": "top-secret"},
	}, OutputFormatPlain, options)

	assertContains(t, out, "duration_ms=42")
	assertContains(t, out, "trace.request_secret=***")
	assertNotContains(t, out, "duration=42ms")
}

// --- Output JSON tests ---

func TestOutputJsonSingleLine(t *testing.T) {
	got := Render(map[string]any{"a": 1, "b": 2}, OutputFormatJson, OutputOptions{})
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
	got := Render(map[string]any{"api_key_secret": "sk-123", "name": "alice"}, OutputFormatJson, OutputOptions{})
	assertContains(t, got, `"api_key_secret":"***"`)
	assertContains(t, got, `"name":"alice"`)
}

func TestOutputJsonOriginalKeys(t *testing.T) {
	got := Render(map[string]any{"latency_ms": 150}, OutputFormatJson, OutputOptions{})
	assertContains(t, got, `"latency_ms"`)
}

func TestOutputJsonRawValues(t *testing.T) {
	got := Render(map[string]any{"latency_ms": 1500}, OutputFormatJson, OutputOptions{})
	assertContains(t, got, `"latency_ms":1500`)
}

func TestOutputJsonNonStringSecretRedacted(t *testing.T) {
	got := Render(map[string]any{"count_secret": 42}, OutputFormatJson, OutputOptions{})
	assertContains(t, got, `"count_secret":"***"`)
}

func TestOutputJsonNestedSecretsRedacted(t *testing.T) {
	got := Render(map[string]any{
		"trace": map[string]any{"api_key_secret": "sk-123", "duration_ms": 150},
	}, OutputFormatJson, OutputOptions{})
	assertContains(t, got, `"api_key_secret":"***"`)
	assertContains(t, got, `"duration_ms":150`)
}

func TestOutputJsonWithTraceOnlyRedactsTraceOnly(t *testing.T) {
	got := Render(map[string]any{
		"code":   "ok",
		"result": map[string]any{"api_key_secret": "sk-live-123"},
		"trace":  map[string]any{"request_secret": "top-secret"},
	}, OutputFormatJson, OutputOptionsForPolicy(RedactionTraceOnly))
	assertContains(t, got, `"request_secret":"***"`)
	assertContains(t, got, `"api_key_secret":"sk-live-123"`)
}

func TestOutputJsonWithNoneKeepsSecrets(t *testing.T) {
	got := Render(map[string]any{
		"api_key_secret": "sk-live-123",
	}, OutputFormatJson, OutputOptionsForPolicy(RedactionOff))
	assertContains(t, got, `"api_key_secret":"sk-live-123"`)
	assertNotContains(t, got, `"***"`)
}

func TestRedactedValueReturnsSafeCopy(t *testing.T) {
	input := map[string]any{
		"api_key_secret": "sk-live-123",
		"nested":         map[string]any{"token_secret": "tok"},
	}
	got := RedactedValue(input).(map[string]any)
	nested := got["nested"].(map[string]any)
	if got["api_key_secret"] != "***" {
		t.Errorf("api_key_secret = %v, want ***", got["api_key_secret"])
	}
	if nested["token_secret"] != "***" {
		t.Errorf("token_secret = %v, want ***", nested["token_secret"])
	}
	if input["api_key_secret"] != "sk-live-123" {
		t.Error("RedactedValue mutated the input")
	}
}

func TestRedactedValueRedactsSecretSubtreeByDefault(t *testing.T) {
	input := map[string]any{
		"db_secret": map[string]any{"password_secret": "real", "host": "localhost"},
	}
	defaultValue := RedactedValue(input).(map[string]any)
	if defaultValue["db_secret"] != "***" {
		t.Fatalf("db_secret = %#v, want ***", defaultValue["db_secret"])
	}
}

func TestMaxDepthMarkerIsNotSecretRedactionMarker(t *testing.T) {
	var input any = "leaf"
	for i := 0; i < 300; i++ {
		input = map[string]any{"next": input}
	}
	out := Render(input, OutputFormatJson, OutputOptions{})
	if !strings.Contains(out, `<afdata:max-depth>`) && !strings.Contains(out, `\u003cafdata:max-depth\u003e`) {
		t.Fatalf("missing max-depth marker: %s", out)
	}
	if strings.Contains(out, "***") {
		t.Fatalf("max-depth path must not use secret redaction marker: %s", out)
	}
}

func TestOutputJsonUnsupportedValueDoesNotCollapseToNull(t *testing.T) {
	got := Render(map[string]any{
		"message": "bad",
		"code":    "info",
		"meta": map[string]any{
			"api_key_secret": "sk-live-123",
			"bad":            func() {},
		},
	}, OutputFormatJson, OutputOptions{})

	var parsed map[string]any
	if err := json.Unmarshal([]byte(got), &parsed); err != nil {
		t.Fatalf("failed to parse OutputJson: %v (%s)", err, got)
	}
	if parsed["message"] != "bad" {
		t.Errorf("message = %v, want bad", parsed["message"])
	}
	if parsed["code"] != "info" {
		t.Errorf("code = %v, want info", parsed["code"])
	}
	meta, ok := parsed["meta"].(map[string]any)
	if !ok {
		t.Fatalf("meta should be object, got %T (%v)", parsed["meta"], parsed["meta"])
	}
	if meta["api_key_secret"] != "***" {
		t.Errorf("api_key_secret = %v, want ***", meta["api_key_secret"])
	}
	if _, ok := meta["bad"]; !ok {
		t.Error("expected meta.bad to be present")
	}
}

func TestOutputJsonStructWithUnsupportedFieldDoesNotLeakSecrets(t *testing.T) {
	type badMeta struct {
		APIKeySecret string `json:"api_key_secret"`
		Fn           func() `json:"fn"`
	}

	got := Render(map[string]any{
		"meta": badMeta{
			APIKeySecret: "sk-live-123",
			Fn:           func() {},
		},
	}, OutputFormatJson, OutputOptions{})

	assertNotContains(t, got, "sk-live-123")

	var parsed map[string]any
	if err := json.Unmarshal([]byte(got), &parsed); err != nil {
		t.Fatalf("failed to parse OutputJson: %v (%s)", err, got)
	}
	meta, ok := parsed["meta"].(string)
	if !ok {
		t.Fatalf("expected meta to be string, got %T (%v)", parsed["meta"], parsed["meta"])
	}
	if !strings.HasPrefix(meta, "<unsupported:") {
		t.Errorf("meta = %q, want prefix <unsupported:", meta)
	}
	if _, ok := parsed["meta"]; !ok {
		t.Fatal("expected meta key in output")
	}
}

func TestOutputYamlUnsupportedStructDoesNotLeakSecrets(t *testing.T) {
	type badMeta struct {
		APIKeySecret string `json:"api_key_secret"`
		Fn           func() `json:"fn"`
	}

	got := Render(map[string]any{
		"meta": badMeta{APIKeySecret: "sk-live-123", Fn: func() {}},
	}, OutputFormatYaml, OutputOptions{})

	assertContains(t, got, "<unsupported:")
	assertNotContains(t, got, "sk-live-123")
	assertNotContains(t, got, "0x")
}

func TestOutputPlainUnsupportedStructDoesNotLeakSecrets(t *testing.T) {
	type badMeta struct {
		APIKeySecret string `json:"api_key_secret"`
		Fn           func() `json:"fn"`
	}

	got := Render(map[string]any{
		"meta": badMeta{APIKeySecret: "sk-live-123", Fn: func() {}},
	}, OutputFormatPlain, OutputOptions{})

	assertContains(t, got, "<unsupported:")
	assertNotContains(t, got, "sk-live-123")
	assertNotContains(t, got, "0x")
}

func TestOutputPlainUnsupportedStringerDoesNotLeakSecrets(t *testing.T) {
	got := Render(map[string]any{"meta": secretStringFunc(func() {})}, OutputFormatPlain, OutputOptions{})

	assertContains(t, got, "<unsupported:")
	assertNotContains(t, got, "sk-live-123")
}

func TestOutputJsonCircularReferenceMapDoesNotCrash(t *testing.T) {
	v := map[string]any{}
	v["self"] = v

	got := Render(v, OutputFormatJson, OutputOptions{})

	var parsed map[string]any
	if err := json.Unmarshal([]byte(got), &parsed); err != nil {
		t.Fatalf("failed to parse OutputJson: %v (%s)", err, got)
	}
	if parsed["self"] != "<unsupported:circular>" {
		t.Errorf("self = %v, want <unsupported:circular>", parsed["self"])
	}
}

func TestOutputJsonCircularReferenceStillRedactsSecrets(t *testing.T) {
	v := map[string]any{
		"api_key_secret": "sk-live-123",
	}
	v["self"] = v

	got := Render(v, OutputFormatJson, OutputOptions{})
	assertNotContains(t, got, "sk-live-123")

	var parsed map[string]any
	if err := json.Unmarshal([]byte(got), &parsed); err != nil {
		t.Fatalf("failed to parse OutputJson: %v (%s)", err, got)
	}
	if parsed["api_key_secret"] != "***" {
		t.Errorf("api_key_secret = %v, want ***", parsed["api_key_secret"])
	}
	if parsed["self"] != "<unsupported:circular>" {
		t.Errorf("self = %v, want <unsupported:circular>", parsed["self"])
	}
}

// --- Output YAML tests ---

func TestOutputYamlStartsWithSeparator(t *testing.T) {
	got := Render(map[string]any{"a": 1}, OutputFormatYaml, OutputOptions{})
	if len(got) < 3 || got[:3] != "---" {
		t.Errorf("expected YAML to start with ---, got %s", got)
	}
}

// YAML never strips AFDATA suffixes or renames keys: unlike `plain`, it is
// structure-preserving, the same semantics as `json`.

func TestOutputYamlKeepsSuffixKeysUnstripped(t *testing.T) {
	got := Render(map[string]any{
		"latency_ms":         150,
		"ttl_s":              3600,
		"pause_ns":           450000,
		"query_us":           830,
		"file_bytes":         5242880,
		"cpu_percent":        85,
		"balance_msats":      50000,
		"withdrawn_sats":     1234,
		"reserve_btc":        0.5,
		"price_usd_cents":    999,
		"price_eur_cents":    850,
		"price_jpy":          1500,
		"deposit_usdt_cents": 1000,
		"timeout_minutes":    30,
		"validity_hours":     24,
		"cert_days":          365,
		"created_epoch_ms":   float64(1738886400000),
		"cached_epoch_s":     float64(1707868800),
		"expires_rfc3339":    "2026-02-14T10:30:00Z",
		"api_key_secret":     "sk-123",
		"user_name":          "alice",
	}, OutputFormatYaml, OutputOptions{})
	for _, key := range []string{
		"latency_ms", "ttl_s", "pause_ns", "query_us", "file_bytes",
		"cpu_percent", "balance_msats", "withdrawn_sats", "reserve_btc",
		"price_usd_cents", "price_eur_cents", "price_jpy", "deposit_usdt_cents",
		"timeout_minutes", "validity_hours", "cert_days", "created_epoch_ms",
		"cached_epoch_s", "expires_rfc3339", "api_key_secret", "user_name",
	} {
		if !strings.Contains(got, key+":") {
			t.Errorf("missing key %s: %s", key, got)
		}
	}
	// The secret value is still redacted; only its key is left alone.
	assertContains(t, got, `api_key_secret: "***"`)
	assertNotContains(t, got, "sk-123")
}

func TestOutputYamlNoSuffixKeysPassThrough(t *testing.T) {
	got := Render(map[string]any{"user_id": 123, "config_path": "a.yml"}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "user_id: 123")
	assertContains(t, got, `config_path: "a.yml"`)
}

func TestOutputYamlUppercaseSecretKeyUnstripped(t *testing.T) {
	got := Render(map[string]any{"API_KEY_SECRET": "sk-123"}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, `API_KEY_SECRET: "***"`)
	assertNotContains(t, got, "sk-123")
}

func TestOutputYamlUppercaseSuffixKeyUnstripped(t *testing.T) {
	got := Render(map[string]any{"CACHE_TTL_S": 3600}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "CACHE_TTL_S: 3600")
}

// --- YAML value formatting tests ---
// Every one of these values would be reformatted into a human string by
// `plain`; YAML must keep them as plain JSON-equivalent numbers/strings.

func TestOutputYamlNumbersStayRawNotFormatted(t *testing.T) {
	got := Render(map[string]any{
		"latency_ms":         1280,
		"ttl_s":              3600,
		"pause_ns":           450000,
		"query_us":           830,
		"file_bytes":         5242880,
		"cpu_percent":        85,
		"success_percent":    95.5,
		"balance_msats":      50000000,
		"withdrawn_sats":     1234,
		"price_usd_cents":    9999,
		"price_eur_cents":    850,
		"price_jpy":          1500,
		"deposit_usdt_cents": 1000,
		"timeout_minutes":    30,
		"validity_hours":     24,
		"cert_days":          365,
	}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "latency_ms: 1280")
	assertContains(t, got, "ttl_s: 3600")
	assertContains(t, got, "pause_ns: 450000")
	assertContains(t, got, "query_us: 830")
	assertContains(t, got, "file_bytes: 5242880")
	assertContains(t, got, "cpu_percent: 85")
	assertContains(t, got, "success_percent: 95.5")
	assertContains(t, got, "balance_msats: 50000000")
	assertContains(t, got, "withdrawn_sats: 1234")
	assertContains(t, got, "price_usd_cents: 9999")
	assertContains(t, got, "price_eur_cents: 850")
	assertContains(t, got, "price_jpy: 1500")
	assertContains(t, got, "deposit_usdt_cents: 1000")
	assertContains(t, got, "timeout_minutes: 30")
	assertContains(t, got, "validity_hours: 24")
	assertContains(t, got, "cert_days: 365")
	for _, lossy := range []string{
		"1.28s", "3600s", "450000ns", "830\u03bcs", "5.0MiB", "85%", "95.5%",
		"50000000msats", "1234sats", "$99.99", "\u20ac8.50", "\u00a51,500",
		"10.00 USDT", "30 minutes", "24 hours", "365 days",
	} {
		assertNotContains(t, got, lossy)
	}
}

func TestOutputYamlEpochAndRfc3339StayAsWrittenNotReformatted(t *testing.T) {
	got := Render(map[string]any{
		"created_epoch_ms": float64(1738886400000),
		"cached_epoch_s":   float64(1707868800),
		"expires_rfc3339":  "2026-02-14T10:30:00Z",
	}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "created_epoch_ms: 1738886400000")
	assertContains(t, got, "cached_epoch_s: 1707868800")
	assertContains(t, got, `expires_rfc3339: "2026-02-14T10:30:00Z"`)
	assertNotContains(t, got, "2025-02-07T00:00:00.000Z")
	assertNotContains(t, got, "2024-02-14T00:00:00.000Z")
}

func TestOutputYamlFmtBtc(t *testing.T) {
	got := Render(map[string]any{"reserve_btc": 0.5}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "reserve_btc: 0.5")
}

func TestOutputYamlFmtSecret(t *testing.T) {
	got := Render(map[string]any{"api_key_secret": "sk-123"}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, `"***"`)
	assertNotContains(t, got, "sk-123")
}

func TestOutputYamlFmtRfc3339Passthrough(t *testing.T) {
	got := Render(map[string]any{"expires_rfc3339": "2026-02-14T10:30:00Z"}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "2026-02-14T10:30:00Z")
}

func TestOutputYamlStringsQuoted(t *testing.T) {
	got := Render(map[string]any{"name": "alice"}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, `"alice"`)
}

func TestOutputYamlNumbersUnquoted(t *testing.T) {
	got := Render(map[string]any{"count": 42}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "count: 42")
}

func TestOutputYamlNestedKeysNotStripped(t *testing.T) {
	got := Render(map[string]any{
		"trace": map[string]any{"duration_ms": 1500, "source": "db"},
	}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "trace:")
	assertContains(t, got, "  duration_ms: 1500")
	assertContains(t, got, `  source: "db"`)
}

func TestOutputYamlIgnoresPlainStyle(t *testing.T) {
	// Unlike `plain`, YAML renders identically regardless of PlainStyle: it
	// is always structure-preserving.
	value := map[string]any{"duration_ms": 42, "name": "alice"}
	readable := Render(value, OutputFormatYaml, OutputOptions{
		Redaction: Redactor{Policy: RedactionOff},
		Style:     PlainStyleReadable,
	})
	raw := Render(value, OutputFormatYaml, OutputOptions{
		Redaction: Redactor{Policy: RedactionOff},
		Style:     PlainStyleRaw,
	})
	if readable != raw {
		t.Errorf("readable and raw yaml differ:\nreadable: %q\nraw: %q", readable, raw)
	}
	assertContains(t, readable, "duration_ms: 42")
	assertNotContains(t, readable, "42ms")
}

func TestOutputYamlStreamOfRecordsHasStableSeparatorFraming(t *testing.T) {
	// Simulates how a CLI streams multiple AFDATA records: each record is
	// rendered independently and concatenated. `---` framing must stay
	// stable and each record's raw keys must stay intact and in order.
	first := Render(map[string]any{"kind": "log", "duration_ms": 1}, OutputFormatYaml, OutputOptions{})
	second := Render(map[string]any{"kind": "result", "duration_ms": 2}, OutputFormatYaml, OutputOptions{})
	stream := first + "\n" + second + "\n"

	if got := strings.Count(stream, "---"); got != 2 {
		t.Fatalf("expected 2 document separators, got %d: %s", got, stream)
	}
	firstIdx := strings.Index(stream, "duration_ms: 1")
	secondIdx := strings.Index(stream, "duration_ms: 2")
	if firstIdx == -1 || secondIdx == -1 {
		t.Fatalf("expected both records present: %s", stream)
	}
	if firstIdx >= secondIdx {
		t.Fatalf("records out of order: %s", stream)
	}
}

// --- Collision tests ---

func TestOutputYamlCollisionKeepsOriginals(t *testing.T) {
	got := Render(map[string]any{"response_ms": 150, "response_bytes": 1024}, OutputFormatYaml, OutputOptions{})
	assertContains(t, got, "response_ms:")
	assertContains(t, got, "response_bytes:")
	// Values should be raw, not formatted
	assertContains(t, got, "response_ms: 150")
	assertContains(t, got, "response_bytes: 1024")
}

func TestOutputPlainCollisionKeepsOriginals(t *testing.T) {
	got := Render(map[string]any{"response_ms": 150, "response_bytes": 1024}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "response_ms=150")
	assertContains(t, got, "response_bytes=1024")
}

// --- Output Plain tests ---

func TestOutputPlainSingleLine(t *testing.T) {
	got := Render(map[string]any{"a": 1, "b": 2}, OutputFormatPlain, OutputOptions{})
	for _, c := range got {
		if c == '\n' {
			t.Error("OutputPlain should be single-line")
		}
	}
}

func TestOutputPlainKeyValuePair(t *testing.T) {
	got := Render(map[string]any{"name": "alice"}, OutputFormatPlain, OutputOptions{})
	assertEqual(t, got, "name=alice")
}

func TestOutputPlainSortedKeys(t *testing.T) {
	got := Render(map[string]any{"z": 1, "a": 2, "m": 3}, OutputFormatPlain, OutputOptions{})
	assertEqual(t, got, "a=2 m=3 z=1")
}

func TestOutputPlainDotNotation(t *testing.T) {
	got := Render(map[string]any{
		"trace": map[string]any{"source": "db"},
	}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "trace.source=db")
}

func TestOutputPlainQuotedSpaces(t *testing.T) {
	got := Render(map[string]any{"message": "hello world"}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, `message="hello world"`)
}

func TestOutputPlainArraysCommaJoined(t *testing.T) {
	got := Render(map[string]any{"fields": []any{"email", "age"}}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "fields=email,age")
}

func TestOutputPlainNullEmpty(t *testing.T) {
	got := Render(map[string]any{"value": nil}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "value=")
}

func TestOutputPlainKeyStrippingAndFormatting(t *testing.T) {
	got := Render(map[string]any{"latency_ms": 1500}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "latency=1.5s")
}

func TestOutputPlainSecretsRedacted(t *testing.T) {
	got := Render(map[string]any{"api_key_secret": "sk-123", "name": "alice"}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "api_key=***")
	assertNotContains(t, got, "sk-123")
}

func TestOutputPlainEmptyObject(t *testing.T) {
	got := Render(map[string]any{}, OutputFormatPlain, OutputOptions{})
	assertEqual(t, got, "")
}

func TestOutputPlainBoolUnquoted(t *testing.T) {
	got := Render(map[string]any{"enabled": true}, OutputFormatPlain, OutputOptions{})
	assertContains(t, got, "enabled=true")
}

func TestOutputPlainNestedSecrets(t *testing.T) {
	got := Render(map[string]any{
		"trace": map[string]any{"api_key_secret": "sk-123", "duration_ms": 150},
	}, OutputFormatPlain, OutputOptions{})
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

// TestNormalizePreservesLargeIntegers guards the normalize() UseNumber fix:
// integers beyond 2^53 inside structs/uint64 must survive OutputJson without
// collapsing to a lossy float64. Not a shared fixture — JSON-in-JS cannot
// represent these values, so it is verified per-language.
func TestNormalizePreservesLargeIntegers(t *testing.T) {
	type event struct {
		CreatedEpochNs int64  `json:"created_epoch_ns"`
		SizeBytes      uint64 `json:"size_bytes"`
	}
	in := event{CreatedEpochNs: 1707868800123456789, SizeBytes: 18446744073709551615}
	got := Render(in, OutputFormatJson, OutputOptions{})
	want := `{"created_epoch_ns":1707868800123456789,"size_bytes":18446744073709551615}`
	assertEqual(t, got, want)
}

// deepEqualIgnoringTrace compares two JSON values for deep equality, ignoring trace fields.
// Used for protocol.json fixtures which don't include default trace: {} in expected output.
func deepEqualIgnoringTrace(a, b any) bool {
	aJSON, _ := json.Marshal(a)
	bJSON, _ := json.Marshal(b)
	return string(aJSON) == string(bJSON)
}
