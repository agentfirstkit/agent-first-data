package afd

import (
	"bytes"
	"context"
	"encoding/json"
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

func TestAfdHandlerBasicFields(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdHandler(&buf, FormatJson))

	logger.Info("hello world")
	m := parseJSONLine(t, &buf)

	if m["message"] != "hello world" {
		t.Errorf("message = %v, want hello world", m["message"])
	}
	if m["code"] != "info" {
		t.Errorf("code = %v, want info", m["code"])
	}
	if _, ok := m["timestamp_epoch_ms"]; !ok {
		t.Error("missing timestamp_epoch_ms")
	}
}

func TestAfdHandlerLevelCodes(t *testing.T) {
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
		logger := slog.New(NewAfdHandler(&buf, FormatJson))
		logger.Log(context.Background(), tt.level, "test")
		m := parseJSONLine(t, &buf)
		if m["code"] != tt.code {
			t.Errorf("level %v: code = %v, want %v", tt.level, m["code"], tt.code)
		}
	}
}

func TestAfdHandlerCodeOverride(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdHandler(&buf, FormatJson))

	logger.Info("ready", "code", "startup")
	m := parseJSONLine(t, &buf)

	if m["code"] != "startup" {
		t.Errorf("code = %v, want startup", m["code"])
	}
}

func TestAfdHandlerWithAttrsSpan(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdHandler(&buf, FormatJson)

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

func TestAfdHandlerEventOverridesSpan(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdHandler(&buf, FormatJson)

	child := handler.WithAttrs([]slog.Attr{slog.String("source", "parent")})
	logger := slog.New(child)

	logger.Info("test", "source", "child")
	m := parseJSONLine(t, &buf)

	if m["source"] != "child" {
		t.Errorf("source = %v, want child (event should override span)", m["source"])
	}
}

func TestWithSpanContext(t *testing.T) {
	var buf bytes.Buffer
	handler := NewAfdHandler(&buf, FormatJson)
	slog.SetDefault(slog.New(handler))

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
	handler := NewAfdHandler(&buf, FormatJson)
	slog.SetDefault(slog.New(handler))

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

func TestAfdHandlerPlainFormat(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdHandler(&buf, FormatPlain))

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
	if !bytes.Contains(buf.Bytes(), []byte("code=info")) {
		t.Errorf("plain output should contain code=info, got: %s", line)
	}
}

func TestAfdHandlerYamlFormat(t *testing.T) {
	var buf bytes.Buffer
	logger := slog.New(NewAfdHandler(&buf, FormatYaml))

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
