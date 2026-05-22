package main

import (
	"strings"
	"testing"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

func TestCompleteHelpContainsAllSubcommands(t *testing.T) {
	help := formatCompleteHelp()
	for _, want := range []string{"echo", "ping", "--output", "--dry-run", "--host"} {
		if !containsStr(help, want) {
			t.Errorf("root --help missing %q", want)
		}
	}
}

func TestSubcommandHelpScoped(t *testing.T) {
	echoHelp := formatSubcommandHelp("echo")
	if !containsStr(echoHelp, "--dry-run") {
		t.Error("echo --help missing --dry-run")
	}
	if containsStr(echoHelp, "--host") {
		t.Error("echo --help should NOT contain --host")
	}
}

func containsStr(s, sub string) bool {
	return len(s) > 0 && len(sub) > 0 && strings.Contains(s, sub)
}

func TestParseOutputAllVariants(t *testing.T) {
	for _, s := range []string{"json", "yaml", "plain"} {
		if _, err := afdata.CliParseOutput(s); err != nil {
			t.Errorf("CliParseOutput(%q): %v", s, err)
		}
	}
}

func TestParseLogNormalizes(t *testing.T) {
	got := afdata.CliParseLogFilters([]string{"Query", " ERROR ", "query"})
	if len(got) != 2 || got[0] != "query" || got[1] != "error" {
		t.Errorf("unexpected: %v", got)
	}
}

func TestBuildCliErrorStructure(t *testing.T) {
	v := afdata.BuildCliError("bad flag", "")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["retryable"] != false {
		t.Errorf("retryable = %v", v["retryable"])
	}
}

func TestBuildCliErrorWithHint(t *testing.T) {
	v := afdata.BuildCliError("unknown action: foo", "valid actions: echo, ping")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["hint"] != "valid actions: echo, ping" {
		t.Errorf("hint = %v", v["hint"])
	}
}

func TestBuildJsonErrorWithHint(t *testing.T) {
	v := afdata.BuildJsonError("not configured", "set PING_HOST", nil)
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["error"] != "not configured" {
		t.Errorf("error = %v", v["error"])
	}
	if v["hint"] != "set PING_HOST" {
		t.Errorf("hint = %v", v["hint"])
	}
}

func TestBuildJsonErrorWithoutHint(t *testing.T) {
	v := afdata.BuildJsonError("something failed", "", nil)
	if _, ok := v["hint"]; ok {
		t.Errorf("hint should not be present, got %v", v["hint"])
	}
}

func TestCliOutputAllFormats(t *testing.T) {
	v := map[string]any{"code": "ok"}
	for _, f := range []afdata.OutputFormat{afdata.OutputFormatJson, afdata.OutputFormatYaml, afdata.OutputFormatPlain} {
		out := afdata.CliOutput(v, f)
		if out == "" {
			t.Errorf("CliOutput(%v) returned empty string", f)
		}
	}
}

func TestErrorRoundTripIsValidJsonl(t *testing.T) {
	v := afdata.BuildCliError("oops", "")
	s := afdata.OutputJson(v)
	if len(s) == 0 {
		t.Error("empty json")
	}
	for _, c := range s {
		if c == '\n' {
			t.Error("json contains newline")
		}
	}
}
