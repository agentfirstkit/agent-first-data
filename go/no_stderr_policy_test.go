package afdata

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestNoStderrUsageInRuntimeSources(t *testing.T) {
	files, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatalf("glob go files: %v", err)
	}

	// Runtime protocol/log events must not use stderr.
	disallowed := regexp.MustCompile(`\bos\.Stderr\b|\bfmt\.Fprint(?:ln|f)?\s*\(\s*os\.Stderr\b|\blog\.SetOutput\s*\(\s*os\.Stderr\b|\bslog\.New(?:Text|JSON)?Handler\s*\(\s*os\.Stderr\b`)

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
			if disallowed.MatchString(line) {
				t.Fatalf("stderr usage is disallowed (%s:%d): %s", path, i+1, strings.TrimSpace(line))
			}
		}
	}
}
