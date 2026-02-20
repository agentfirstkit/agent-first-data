package main

import (
	"testing"

	afdata "github.com/cmnspore/agent-first-data/go"
)

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
	v := afdata.BuildCliError("bad flag")
	if v["code"] != "error" {
		t.Errorf("code = %v", v["code"])
	}
	if v["retryable"] != false {
		t.Errorf("retryable = %v", v["retryable"])
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
	v := afdata.BuildCliError("oops")
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
