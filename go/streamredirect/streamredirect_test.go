package streamredirect

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestParseStreamRedirectArgs(t *testing.T) {
	cfg, err := ParseStreamRedirectArgs([]string{
		"agent-cli",
		"--stdout-file",
		"/tmp/agent-cli.out",
		"--stderr-file=/tmp/agent-cli.err",
		"ping",
	})
	if err != nil {
		t.Fatalf("ParseStreamRedirectArgs returned error: %v", err)
	}
	if cfg == nil {
		t.Fatal("expected stream redirection config")
	}
	if cfg.StdoutFile != "/tmp/agent-cli.out" {
		t.Fatalf("StdoutFile = %q", cfg.StdoutFile)
	}
	if cfg.StderrFile != "/tmp/agent-cli.err" {
		t.Fatalf("StderrFile = %q", cfg.StderrFile)
	}
}

func TestParseStreamRedirectArgsDisabled(t *testing.T) {
	cfg, err := ParseStreamRedirectArgs([]string{"agent-cli", "ping"})
	if err != nil {
		t.Fatalf("ParseStreamRedirectArgs returned error: %v", err)
	}
	if cfg != nil {
		t.Fatalf("expected nil config, got %#v", cfg)
	}
}

func TestParseStreamRedirectArgsMissingValue(t *testing.T) {
	if _, err := ParseStreamRedirectArgs([]string{"agent-cli", "--stderr-file", "--help"}); err == nil {
		t.Fatal("expected missing value error")
	}
}

func TestInstallStreamRedirectUnsupportedOnWindows(t *testing.T) {
	if runtime.GOOS != "windows" {
		t.Skip("windows-only unsupported-platform assertion")
	}
	installed, err := InstallStreamRedirect(StreamRedirectConfig{StdoutFile: "stdout.log"})
	if installed != nil {
		_ = installed.Close()
	}
	if err == nil {
		t.Fatal("expected unsupported stream redirection error")
	}
	if !strings.Contains(err.Error(), "only supported on Unix") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestInstallStreamRedirectRedirectsOutput(t *testing.T) {
	dir := t.TempDir()
	stdoutPath := filepath.Join(dir, "stdout.log")
	stderrPath := filepath.Join(dir, "stderr.log")
	if err := os.WriteFile(stdoutPath, []byte("existing stdout\n"), 0o644); err != nil {
		t.Fatalf("prewrite stdout file: %v", err)
	}

	installed, err := InstallStreamRedirect(StreamRedirectConfig{
		StdoutFile: stdoutPath,
		StderrFile: stderrPath,
	})
	if err != nil {
		if strings.Contains(err.Error(), "only supported on Unix") {
			t.Skip(err)
		}
		t.Fatalf("InstallStreamRedirect returned error: %v", err)
	}
	closed := false
	defer func() {
		if !closed {
			_ = installed.Close()
		}
	}()

	if _, err := os.Stdout.WriteString("stdout bytes\n"); err != nil {
		t.Fatalf("write redirected stdout: %v", err)
	}
	if _, err := os.Stderr.WriteString("stderr bytes\n"); err != nil {
		t.Fatalf("write redirected stderr: %v", err)
	}
	if err := installed.Close(); err != nil {
		t.Fatalf("Close returned error: %v", err)
	}
	closed = true

	stdoutData, err := os.ReadFile(stdoutPath)
	if err != nil {
		t.Fatalf("read stdout file: %v", err)
	}
	if string(stdoutData) != "existing stdout\nstdout bytes\n" {
		t.Fatalf("stdout file = %q", stdoutData)
	}
	stderrData, err := os.ReadFile(stderrPath)
	if err != nil {
		t.Fatalf("read stderr file: %v", err)
	}
	if string(stderrData) != "stderr bytes\n" {
		t.Fatalf("stderr file = %q", stderrData)
	}
	stderrInfo, err := os.Stat(stderrPath)
	if err != nil {
		t.Fatalf("stat stderr file: %v", err)
	}
	if stderrInfo.Mode().Perm() != 0o600 {
		t.Fatalf("stderr file mode = %o, want 600", stderrInfo.Mode().Perm())
	}
}

func TestInstallStreamRedirectRejectsSymlink(t *testing.T) {
	dir := t.TempDir()
	realPath := filepath.Join(dir, "real.log")
	linkPath := filepath.Join(dir, "stdout.log")
	if err := os.WriteFile(realPath, []byte{}, 0o600); err != nil {
		t.Fatalf("create real file: %v", err)
	}
	if err := os.Symlink(realPath, linkPath); err != nil {
		t.Skipf("symlink unsupported: %v", err)
	}

	installed, err := InstallStreamRedirect(StreamRedirectConfig{StdoutFile: linkPath})
	if installed != nil {
		_ = installed.Close()
	}
	if err == nil {
		t.Fatal("expected symlink target error")
	}
	if strings.Contains(err.Error(), "only supported on Unix") {
		t.Skip(err)
	}
	if !strings.Contains(err.Error(), "symbolic link") && !strings.Contains(err.Error(), "too many") {
		t.Fatalf("unexpected error: %v", err)
	}
}
