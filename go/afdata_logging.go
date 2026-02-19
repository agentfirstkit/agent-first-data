package afdata

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"os"
	"sync"
)

// LogFormat controls the output format of the AFDATA handler.
type LogFormat int

const (
	// FormatJson outputs single-line JSONL (secrets redacted, original keys).
	FormatJson LogFormat = iota
	// FormatPlain outputs single-line logfmt (keys stripped, values formatted).
	FormatPlain
	// FormatYaml outputs multi-line YAML (keys stripped, values formatted).
	FormatYaml
)

// AfdataHandler implements slog.Handler, outputting AFDATA-compliant log lines.
//
// Each log line contains: timestamp_epoch_ms, message, code, plus
// any span-level (WithAttrs) and event-level fields.
// Output is formatted via the library's own OutputJson/OutputPlain/OutputYaml.
type AfdataHandler struct {
	out    io.Writer
	mu     *sync.Mutex
	attrs  []slog.Attr
	format LogFormat
}

// NewAfdataHandler creates a new AFDATA handler writing to w with the given format.
func NewAfdataHandler(w io.Writer, format LogFormat) *AfdataHandler {
	return &AfdataHandler{out: w, mu: &sync.Mutex{}, format: format}
}

// InitJson sets up the default slog logger with AFDATA JSON output to stdout.
func InitJson() {
	slog.SetDefault(slog.New(NewAfdataHandler(os.Stdout, FormatJson)))
}

// InitPlain sets up the default slog logger with AFDATA plain/logfmt output to stdout.
func InitPlain() {
	slog.SetDefault(slog.New(NewAfdataHandler(os.Stdout, FormatPlain)))
}

// InitYaml sets up the default slog logger with AFDATA YAML output to stdout.
func InitYaml() {
	slog.SetDefault(slog.New(NewAfdataHandler(os.Stdout, FormatYaml)))
}

// Enabled returns true for all levels (filtering is done at the slog level).
func (h *AfdataHandler) Enabled(_ context.Context, _ slog.Level) bool {
	return true
}

// Handle outputs a single AFDATA-compliant log line.
func (h *AfdataHandler) Handle(_ context.Context, r slog.Record) error {
	m := make(map[string]any, 4+len(h.attrs)+r.NumAttrs())

	m["timestamp_epoch_ms"] = r.Time.UnixMilli()
	m["message"] = r.Message

	defaultCode := levelToCode(r.Level)

	// Span-level fields (from WithAttrs)
	for _, a := range h.attrs {
		m[a.Key] = attrValue(a.Value)
	}

	// Event-level fields (override span fields on collision)
	hasCode := false
	r.Attrs(func(a slog.Attr) bool {
		if a.Key == "code" {
			hasCode = true
		}
		m[a.Key] = attrValue(a.Value)
		return true
	})

	if !hasCode {
		m["code"] = defaultCode
	}

	// Format using the library's own output functions
	var line string
	switch h.format {
	case FormatPlain:
		line = OutputPlain(m)
	case FormatYaml:
		line = OutputYaml(m)
	default:
		line = OutputJson(m)
	}

	h.mu.Lock()
	defer h.mu.Unlock()
	_, err := io.WriteString(h.out, line+"\n")
	return err
}

// WithAttrs returns a new handler with additional span-level fields.
func (h *AfdataHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	combined := make([]slog.Attr, len(h.attrs), len(h.attrs)+len(attrs))
	copy(combined, h.attrs)
	combined = append(combined, attrs...)
	return &AfdataHandler{out: h.out, mu: h.mu, attrs: combined, format: h.format}
}

// WithGroup returns the handler unchanged (groups are not used in AFDATA output).
func (h *AfdataHandler) WithGroup(_ string) slog.Handler {
	return h
}

func levelToCode(l slog.Level) string {
	switch {
	case l < slog.LevelDebug:
		return "trace"
	case l < slog.LevelInfo:
		return "debug"
	case l < slog.LevelWarn:
		return "info"
	case l < slog.LevelError:
		return "warn"
	default:
		return "error"
	}
}

func attrValue(v slog.Value) any {
	switch v.Kind() {
	case slog.KindString:
		return v.String()
	case slog.KindInt64:
		return v.Int64()
	case slog.KindUint64:
		return v.Uint64()
	case slog.KindFloat64:
		return v.Float64()
	case slog.KindBool:
		return v.Bool()
	case slog.KindDuration:
		return v.Duration().Milliseconds()
	case slog.KindTime:
		return v.Time().UnixMilli()
	case slog.KindGroup:
		attrs := v.Group()
		m := make(map[string]any, len(attrs))
		for _, a := range attrs {
			m[a.Key] = attrValue(a.Value)
		}
		return m
	default:
		return v.String()
	}
}

// Span runs fn with a logger that carries the given fields.
func Span(fields map[string]any, fn func()) {
	parent := slog.Default()
	attrs := make([]slog.Attr, 0, len(fields))
	for k, v := range fields {
		attrs = append(attrs, slog.Any(k, v))
	}
	child := slog.New(parent.Handler().WithAttrs(attrs))
	slog.SetDefault(child)
	defer slog.SetDefault(parent)
	fn()
}

type spanKey struct{}

// WithSpan returns a context carrying a logger with the given fields.
func WithSpan(ctx context.Context, fields map[string]any) context.Context {
	parent := LoggerFromContext(ctx)
	attrs := make([]slog.Attr, 0, len(fields))
	for k, v := range fields {
		attrs = append(attrs, slog.Any(k, v))
	}
	child := slog.New(parent.Handler().WithAttrs(attrs))
	return context.WithValue(ctx, spanKey{}, child)
}

// LoggerFromContext returns the span logger from the context, or slog.Default().
func LoggerFromContext(ctx context.Context) *slog.Logger {
	if l, ok := ctx.Value(spanKey{}).(*slog.Logger); ok {
		return l
	}
	return slog.Default()
}

// ensure AfdataHandler implements slog.Handler at compile time
var _ slog.Handler = (*AfdataHandler)(nil)

// ensure json is imported (used by OutputJson fallback)
var _ = json.Marshal
