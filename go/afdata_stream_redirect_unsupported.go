//go:build !unix

package afdata

import "errors"

// InstalledStreamRedirect is returned by InstallStreamRedirect on supported platforms.
type InstalledStreamRedirect struct {
	StdoutFile string
	StderrFile string
}

// InstallStreamRedirect reports that fd-level stream redirection is unavailable.
func InstallStreamRedirect(cfg StreamRedirectConfig) (*InstalledStreamRedirect, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.StdoutFile == "" && cfg.StderrFile == "" {
		return nil, nil
	}
	return nil, errors.New("stream redirection is only supported on Unix platforms")
}

// Close is a no-op on unsupported platforms.
func (r *InstalledStreamRedirect) Close() error {
	return nil
}
