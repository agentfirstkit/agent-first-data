package streamredirect

import (
	"errors"
	"fmt"
	"os"
	"strings"
)

// Canonical CLI arguments for stream redirection.
const (
	StdoutFileArg = "--stdout-file"
	StderrFileArg = "--stderr-file"
)

// StreamRedirectConfig describes optional stdout/stderr file redirection.
type StreamRedirectConfig struct {
	StdoutFile string
	StderrFile string
}

// ParseStreamRedirectArgs resolves --stdout-file/--stderr-file from raw CLI args.
// It returns nil when neither stream is redirected.
func ParseStreamRedirectArgs(args []string) (*StreamRedirectConfig, error) {
	var cfg StreamRedirectConfig
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if arg == "--" {
			break
		}
		switch {
		case arg == StdoutFileArg:
			value, next, err := takeStreamRedirectValue(args, i, StdoutFileArg)
			if err != nil {
				return nil, err
			}
			cfg.StdoutFile = value
			i = next
		case strings.HasPrefix(arg, StdoutFileArg+"="):
			cfg.StdoutFile = strings.TrimPrefix(arg, StdoutFileArg+"=")
			if cfg.StdoutFile == "" {
				return nil, fmt.Errorf("%s must not be empty", StdoutFileArg)
			}
		case arg == StderrFileArg:
			value, next, err := takeStreamRedirectValue(args, i, StderrFileArg)
			if err != nil {
				return nil, err
			}
			cfg.StderrFile = value
			i = next
		case strings.HasPrefix(arg, StderrFileArg+"="):
			cfg.StderrFile = strings.TrimPrefix(arg, StderrFileArg+"=")
			if cfg.StderrFile == "" {
				return nil, fmt.Errorf("%s must not be empty", StderrFileArg)
			}
		}
	}
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.StdoutFile == "" && cfg.StderrFile == "" {
		return nil, nil
	}
	return &cfg, nil
}

// InstallStreamRedirectFromArgs installs file redirection from raw CLI args.
// It returns nil when neither --stdout-file nor --stderr-file is set.
func InstallStreamRedirectFromArgs(args []string) (*InstalledStreamRedirect, error) {
	cfg, err := ParseStreamRedirectArgs(args)
	if err != nil {
		return nil, err
	}
	if cfg == nil {
		return nil, nil
	}
	return InstallStreamRedirect(*cfg)
}

// Validate checks stream redirection paths.
func (c StreamRedirectConfig) Validate() error {
	if c.StdoutFile == "" && c.StderrFile == "" {
		return nil
	}
	if c.StdoutFile != "" && strings.TrimSpace(c.StdoutFile) == "" {
		return errors.New("--stdout-file must not be empty")
	}
	if c.StderrFile != "" && strings.TrimSpace(c.StderrFile) == "" {
		return errors.New("--stderr-file must not be empty")
	}
	return nil
}

func takeStreamRedirectValue(args []string, idx int, flag string) (string, int, error) {
	next := idx + 1
	if next >= len(args) || args[next] == "" || strings.HasPrefix(args[next], "--") {
		return "", idx, fmt.Errorf("%s requires a value", flag)
	}
	return args[next], next, nil
}

func flushStandardStreams() {
	_ = os.Stdout.Sync()
	_ = os.Stderr.Sync()
}
