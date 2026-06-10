package afdata

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"testing"
)

func parseJSONLine(t *testing.T, buf *bytes.Buffer) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal(buf.Bytes(), &m); err != nil {
		t.Fatalf("failed to parse JSON: %v\nraw: %s", err, buf.String())
	}
	buf.Reset()
	return m
}

func setDefaultLoggerForTest(t *testing.T, logger *slog.Logger) {
	t.Helper()
	prev := slog.Default()
	slog.SetDefault(logger)
	t.Cleanup(func() {
		slog.SetDefault(prev)
	})
}

func TestAfdataHandlerBasicFields(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("hello world")
	m := parseJSONLine(t, &buf)

	if m["message"] != "hello world" {
		t.Errorf("message = %v, want hello world", m["message"])
	}
	if m["code"] != "log" {
		t.Errorf("code = %v, want log", m["code"])
	}
	if m["level"] != "info" {
		t.Errorf("level = %v, want info", m["level"])
	}
	if _, ok := m["timestamp_epoch_ms"]; !ok {
		t.Error("missing timestamp_epoch_ms")
	}
}

func TestAfdataHandlerLevelCodes(t *testing.T) {
	tests := []struct {
		level slog.Level
		code  string
	}{
		{slog.LevelDebug, "debug"},
		{slog.LevelInfo, "info"},
		{slog.LevelWarn, "warn"},
		{slog.LevelError, "error"},
	}

	for _, tt := range tests {
		var buf bytes.Buffer
		logger := slog.New(NewAfdataHandlerWithLevel(&buf, FormatJson, slog.LevelDebug))
		logger.Log(context.Background(), tt.level, "test")
		m := parseJSONLine(t, &buf)
		if m["code"] != "log" {
			t.Errorf("level %v: code = %v, want log", tt.level, m["code"])
		}
		if m["level"] != tt.code {
			t.Errorf("level %v: level = %v, want %v", tt.level, m["level"], tt.code)
		}
	}
}

func TestAfdataHandlerDefaultLevelIsInfo(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Debug("debug should be filtered")
	if buf.Len() != 0 {
		t.Fatalf("debug log should be filtered by default, got: %s", buf.String())
	}

	logger.Info("info should pass")
	m := parseJSONLine(t, &buf)
	if m["code"] != "log" {
		t.Errorf("code = %v, want log", m["code"])
	}
	if m["level"] != "info" {
		t.Errorf("level = %v, want info", m["level"])
	}
}

func TestAfdataHandlerCustomLevelAllowsDebug(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandlerWithLevel(&buf, FormatJson, slog.LevelDebug))

	logger.Debug("debug should pass")
	m := parseJSONLine(t, &buf)
	if m["code"] != "log" {
		t.Errorf("code = %v, want log", m["code"])
	}
	if m["level"] != "debug" {
		t.Errorf("level = %v, want debug", m["level"])
	}
}

func TestAfdataHandlerCodeOverride(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("ready", "code", "startup")
	m := parseJSONLine(t, &buf)

	if m["code"] != "log" {
		t.Errorf("code = %v, want log", m["code"])
	}
}

func TestAfdataHandlerWithAttrsSpan(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdataHandler(&buf, FormatJson)

	// Simulate a span by creating a child handler with attrs
	child := handler.WithAttrs([]slog.Attr{slog.String("request_id", "abc-123")})
	logger := slog.New(child)

	logger.Info("processing", "domain", "example.com")
	m := parseJSONLine(t, &buf)

	if m["request_id"] != "abc-123" {
		t.Errorf("request_id = %v, want abc-123", m["request_id"])
	}
	if m["domain"] != "example.com" {
		t.Errorf("domain = %v, want example.com", m["domain"])
	}
	if m["message"] != "processing" {
		t.Errorf("message = %v, want processing", m["message"])
	}
}

func TestAfdataHandlerEventOverridesSpan(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdataHandler(&buf, FormatJson)

	child := handler.WithAttrs([]slog.Attr{slog.String("source", "parent")})
	logger := slog.New(child)

	logger.Info("test", "source", "child")
	m := parseJSONLine(t, &buf)

	if m["source"] != "child" {
		t.Errorf("source = %v, want child (event should override span)", m["source"])
	}
}

func TestAfdataHandlerAnyMapSecretsAreRedacted(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("event", "meta", map[string]any{"api_key_secret": "sk-live-123"})
	m := parseJSONLine(t, &buf)

	meta, ok := m["meta"].(map[string]any)
	if !ok {
		t.Fatalf("meta should be object, got %T (%v)", m["meta"], m["meta"])
	}
	if meta["api_key_secret"] != "***" {
		t.Errorf("api_key_secret = %v, want ***", meta["api_key_secret"])
	}
}

func TestAfdataHandlerSecretNamesRedactionOptions(t *testing.T) {
	tests := []struct {
		name   string
		format LogFormat
	}{
		{"json", FormatJson},
		{"plain", FormatPlain},
		{"yaml", FormatYaml},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var buf bytes.Buffer
			logger := slog.New(NewAfdataHandlerWithOptions(
				&buf,
				tt.format,
				slog.LevelInfo,
				RedactionOptions{SecretNames: []string{"authorization"}},
			))

			logger.Info(
				"authorization appears in message but is not name-redacted",
				"authorization", "Bearer legacy",
				"request_url", "https://example.test/path?authorization=legacy&ok=1",
			)
			line := buf.String()

			if !bytes.Contains([]byte(line), []byte("***")) {
				t.Fatalf("expected redacted marker in output, got: %s", line)
			}
			if bytes.Contains([]byte(line), []byte("Bearer legacy")) {
				t.Fatalf("legacy field value should be redacted, got: %s", line)
			}
			if bytes.Contains([]byte(line), []byte("authorization=legacy")) {
				t.Fatalf("legacy URL query parameter should be redacted, got: %s", line)
			}
			if !bytes.Contains([]byte(line), []byte("authorization appears in message")) {
				t.Fatalf("message should stay readable, got: %s", line)
			}
		})
	}
}

func TestAfdataHandlerSecretNamesDefaultLeavesLegacyFieldVisible(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("event", "authorization", "Bearer visible")
	m := parseJSONLine(t, &buf)

	if m["authorization"] != "Bearer visible" {
		t.Errorf("authorization = %v, want Bearer visible", m["authorization"])
	}
}

func TestAfdataHandlerUnsupportedAnyDoesNotEmitNullLine(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("bad", "meta", map[string]any{
		"api_key_secret": "sk-live-123",
		"bad":            func() {},
	})
	m := parseJSONLine(t, &buf)

	if m["message"] != "bad" {
		t.Errorf("message = %v, want bad", m["message"])
	}
	meta, ok := m["meta"].(map[string]any)
	if !ok {
		t.Fatalf("meta should be object, got %T (%v)", m["meta"], m["meta"])
	}
	if meta["api_key_secret"] != "***" {
		t.Errorf("api_key_secret = %v, want ***", meta["api_key_secret"])
	}
	if _, ok := meta["bad"]; !ok {
		t.Error("expected meta.bad to be present")
	}
}

func TestAfdataHandlerErrorFieldIsReadableString(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatJson))

	logger.Info("request failed", "error", errors.New("timeout"))
	m := parseJSONLine(t, &buf)

	if m["error"] != "timeout" {
		t.Errorf("error = %v, want timeout", m["error"])
	}
}

func TestWithSpanContext(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdataHandler(&buf, FormatJson)
	setDefaultLoggerForTest(t, slog.New(handler))

	ctx := context.Background()
	ctx = WithSpan(ctx, map[string]any{"request_id": "ctx-456"})

	logger := LoggerFromContext(ctx)
	logger.Info("from context")
	m := parseJSONLine(t, &buf)

	if m["request_id"] != "ctx-456" {
		t.Errorf("request_id = %v, want ctx-456", m["request_id"])
	}
}

func TestNestedSpanContext(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdataHandler(&buf, FormatJson)
	setDefaultLoggerForTest(t, slog.New(handler))

	ctx := context.Background()
	ctx = WithSpan(ctx, map[string]any{"request_id": "outer"})
	ctx = WithSpan(ctx, map[string]any{"step": "inner"})

	logger := LoggerFromContext(ctx)
	logger.Info("nested")
	m := parseJSONLine(t, &buf)

	if m["request_id"] != "outer" {
		t.Errorf("request_id = %v, want outer", m["request_id"])
	}
	if m["step"] != "inner" {
		t.Errorf("step = %v, want inner", m["step"])
	}
}

func TestAfdataHandlerPlainFormat(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatPlain))

	logger.Info("hello")
	line := buf.String()

	// Plain format is single-line logfmt with stripped keys
	if line == "" {
		t.Fatal("no output")
	}
	if line[len(line)-1] != '\n' {
		t.Error("plain output should end with newline")
	}
	// Should contain message= (plain uses logfmt key=value)
	if !bytes.Contains(buf.Bytes(), []byte("message=")) {
		t.Errorf("plain output should contain message=, got: %s", line)
	}
	if !bytes.Contains(buf.Bytes(), []byte("code=log")) {
		t.Errorf("plain output should contain code=log, got: %s", line)
	}
}

func TestAfdataHandlerYamlFormat(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdataHandler(&buf, FormatYaml))

	logger.Info("hello")
	line := buf.String()

	if line == "" {
		t.Fatal("no output")
	}
	// YAML format starts with ---
	if !bytes.HasPrefix(buf.Bytes(), []byte("---")) {
		t.Errorf("yaml output should start with ---, got: %s", line)
	}
}
