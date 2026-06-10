package afdata

import (
	"context"
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
// Each log line contains timestamp_epoch_ms, message, code="log", level,
// plus any span-level (WithAttrs) and event-level fields.
// Output is formatted via the library's own OutputJson/OutputPlain/OutputYaml.
type AfdataHandler struct {
	out       io.Writer
	mu        *sync.Mutex
	attrs     []slog.Attr
	format    LogFormat
	level     slog.Level
	redaction RedactionOptions
}

// NewAfdataHandler creates a new AFDATA handler writing to w with the given format.
func NewAfdataHandler(w io.Writer, format LogFormat) *AfdataHandler {
	return NewAfdataHandlerWithLevel(w, format, slog.LevelInfo)
}

// NewAfdataHandlerWithLevel creates a new AFDATA handler with a minimum enabled level.
func NewAfdataHandlerWithLevel(w io.Writer, format LogFormat, level slog.Level) *AfdataHandler {
	return NewAfdataHandlerWithOptions(w, format, level, RedactionOptions{})
}

// NewAfdataHandlerWithOptions creates a new AFDATA handler with explicit redaction options.
func NewAfdataHandlerWithOptions(w io.Writer, format LogFormat, level slog.Level, redaction RedactionOptions) *AfdataHandler {
	return &AfdataHandler{
		out:       w,
		mu:        &sync.Mutex{},
		format:    format,
		level:     level,
		redaction: cloneRedactionOptions(redaction),
	}
}

// InitJson sets up the default slog logger with AFDATA JSON output to stdout.
func InitJson() {
	InitJsonLevel(slog.LevelInfo)
}

// InitJsonLevel sets up the default slog logger with AFDATA JSON output and minimum level.
func InitJsonLevel(level slog.Level) {
	InitJsonLevelWithOptions(level, RedactionOptions{})
}

// InitJsonWithOptions sets up the default slog logger with AFDATA JSON output and redaction options.
func InitJsonWithOptions(redaction RedactionOptions) {
	InitJsonLevelWithOptions(slog.LevelInfo, redaction)
}

// InitJsonLevelWithOptions sets up the default slog logger with AFDATA JSON output, level, and redaction options.
func InitJsonLevelWithOptions(level slog.Level, redaction RedactionOptions) {
	slog.SetDefault(slog.New(NewAfdataHandlerWithOptions(os.Stdout, FormatJson, level, redaction)))
}

// InitPlain sets up the default slog logger with AFDATA plain/logfmt output to stdout.
func InitPlain() {
	InitPlainLevel(slog.LevelInfo)
}

// InitPlainLevel sets up the default slog logger with AFDATA plain output and minimum level.
func InitPlainLevel(level slog.Level) {
	InitPlainLevelWithOptions(level, RedactionOptions{})
}

// InitPlainWithOptions sets up the default slog logger with AFDATA plain output and redaction options.
func InitPlainWithOptions(redaction RedactionOptions) {
	InitPlainLevelWithOptions(slog.LevelInfo, redaction)
}

// InitPlainLevelWithOptions sets up the default slog logger with AFDATA plain output, level, and redaction options.
func InitPlainLevelWithOptions(level slog.Level, redaction RedactionOptions) {
	slog.SetDefault(slog.New(NewAfdataHandlerWithOptions(os.Stdout, FormatPlain, level, redaction)))
}

// InitYaml sets up the default slog logger with AFDATA YAML output to stdout.
func InitYaml() {
	InitYamlLevel(slog.LevelInfo)
}

// InitYamlLevel sets up the default slog logger with AFDATA YAML output and minimum level.
func InitYamlLevel(level slog.Level) {
	InitYamlLevelWithOptions(level, RedactionOptions{})
}

// InitYamlWithOptions sets up the default slog logger with AFDATA YAML output and redaction options.
func InitYamlWithOptions(redaction RedactionOptions) {
	InitYamlLevelWithOptions(slog.LevelInfo, redaction)
}

// InitYamlLevelWithOptions sets up the default slog logger with AFDATA YAML output, level, and redaction options.
func InitYamlLevelWithOptions(level slog.Level, redaction RedactionOptions) {
	slog.SetDefault(slog.New(NewAfdataHandlerWithOptions(os.Stdout, FormatYaml, level, redaction)))
}

// Enabled returns whether the level is enabled for this handler.
func (h *AfdataHandler) Enabled(_ context.Context, level slog.Level) bool {
	return level >= h.level
}

// Handle outputs a single AFDATA-compliant log line.
func (h *AfdataHandler) Handle(_ context.Context, r slog.Record) error {
	m := make(map[string]any, 4+len(h.attrs)+r.NumAttrs())

	m["timestamp_epoch_ms"] = r.Time.UnixMilli()
	m["message"] = r.Message
	m["code"] = "log"
	m["level"] = levelName(r.Level)

	// Span-level fields (from WithAttrs)
	for _, a := range h.attrs {
		m[a.Key] = attrValue(a.Value)
	}

	// Event-level fields override span fields, except protocol code is always "log".
	r.Attrs(func(a slog.Attr) bool {
		if a.Key == "code" {
			return true
		}
		m[a.Key] = attrValue(a.Value)
		return true
	})

	// Format using the library's own output functions
	var line string
	options := OutputOptions{Redaction: h.redaction}
	switch h.format {
	case FormatPlain:
		line = OutputPlainWithOptions(m, options)
	case FormatYaml:
		line = OutputYamlWithOptions(m, options)
	default:
		line = OutputJsonWithOptions(m, options)
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
	return &AfdataHandler{
		out:       h.out,
		mu:        h.mu,
		attrs:     combined,
		format:    h.format,
		level:     h.level,
		redaction: h.redaction,
	}
}

// WithGroup returns the handler unchanged (groups are not used in AFDATA output).
func (h *AfdataHandler) WithGroup(_ string) slog.Handler {
	return h
}

func levelName(l slog.Level) string {
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
	case slog.KindLogValuer:
		return attrValue(v.Resolve())
	case slog.KindAny:
		if err, ok := v.Any().(error); ok {
			return err.Error()
		}
		return sanitizeForJSON(v.Any())
	default:
		return sanitizeForJSON(v.Any())
	}
}

func cloneRedactionOptions(redaction RedactionOptions) RedactionOptions {
	if redaction.SecretNames != nil {
		redaction.SecretNames = append([]string(nil), redaction.SecretNames...)
	}
	return redaction
}

// Span runs fn with a logger that carries the given fields.
//
// Deprecated: Span temporarily mutates slog.Default and is not suited for
// concurrent request handling. Prefer WithSpan + LoggerFromContext.
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
