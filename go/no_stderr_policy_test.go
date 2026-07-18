package afdata

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// Ad-hoc stderr writes bypass the emitter and are forbidden: writing protocol
// or log events straight to the process error stream defeats the emitter's
// kind-based channel routing (see the spec's CLI Event Framing). These match an
// actual write to stderr, not a mere reference.
var adHocStderrWrite = regexp.MustCompile(
	`\bfmt\.Fprint(?:ln|f)?\s*\(\s*os\.Stderr\b` +
		`|\bos\.Stderr\.Write(?:String)?\s*\(` +
		`|\blog\.SetOutput\s*\(\s*os\.Stderr\b` +
		`|\bslog\.New(?:Text|JSON)?Handler\s*\(\s*os\.Stderr\b`,
)

// A bare os.Stderr reference is allowed only when it is handed to the emitter
// as a sink (the finite/from-output-to constructors). That sanctioned wiring is
// the one blessed route to stderr; any other reference could stash the stream
// and write around the emitter, so it stays forbidden.
var bareStderrRef = regexp.MustCompile(`\bos\.Stderr\b`)
var sanctionedEmitterSink = regexp.MustCompile(`\bNewCliEmitter\w*\s*\(`)

// stderrPolicyViolation reports whether a source line violates the stderr
// policy. Ad-hoc writes always fail; a bare os.Stderr reference fails unless it
// is on a line that constructs a CliEmitter (the sanctioned diagnostic sink).
func stderrPolicyViolation(line string) bool {
	if adHocStderrWrite.MatchString(line) {
		return true
	}
	if bareStderrRef.MatchString(line) && !sanctionedEmitterSink.MatchString(line) {
		return true
	}
	return false
}

func TestNoStderrUsageInRuntimeSources(t *testing.T) {
	files, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatalf("glob go files: %v", err)
	}

	for _, path := range files {
		if strings.HasSuffix(path, "_test.go") {
			continue
		}

		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("read %s: %v", path, err)
		}

		lines := strings.Split(string(data), "\n")
		for i, line := range lines {
			if stderrPolicyViolation(line) {
				t.Fatalf("ad-hoc stderr usage is disallowed (%s:%d): %s", path, i+1, strings.TrimSpace(line))
			}
		}
	}
}

// TestStderrPolicyDistinguishesSanctionedFromAdHoc pins the policy semantics:
// the emitter's diagnostic sink passes while stray stderr still fails.
func TestStderrPolicyDistinguishesSanctionedFromAdHoc(t *testing.T) {
	sanctioned := []string{
		`return NewCliEmitterFiniteWithOptions(os.Stdout, os.Stderr, format, outputOptions)`,
		`return NewCliEmitterWithOptions(os.Stderr, format, outputOptions)`,
		`emitter := NewCliEmitterFinite(resultBuf, os.Stderr, OutputFormatJson)`,
	}
	for _, line := range sanctioned {
		if stderrPolicyViolation(line) {
			t.Errorf("sanctioned emitter sink flagged as violation: %s", line)
		}
	}

	adHoc := []string{
		`fmt.Fprintln(os.Stderr, "boom")`,
		`fmt.Fprintf(os.Stderr, "%v", err)`,
		`os.Stderr.Write([]byte("x"))`,
		`os.Stderr.WriteString("x")`,
		`log.SetOutput(os.Stderr)`,
		`handler := slog.NewJSONHandler(os.Stderr, nil)`,
		`w := os.Stderr`,
	}
	for _, line := range adHoc {
		if !stderrPolicyViolation(line) {
			t.Errorf("ad-hoc stderr usage not flagged: %s", line)
		}
	}
}
