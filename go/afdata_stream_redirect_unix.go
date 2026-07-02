//go:build unix

package afdata

import (
	"errors"
	"os"
	"sync/atomic"
	"syscall"
)

var streamRedirectInstalled atomic.Bool

// InstalledStreamRedirect restores original stdout/stderr when closed.
type InstalledStreamRedirect struct {
	StdoutFile string
	StderrFile string

	stdoutRestore *os.File
	stderrRestore *os.File
	closed        atomic.Bool
}

// InstallStreamRedirect redirects configured streams to append-only files.
func InstallStreamRedirect(cfg StreamRedirectConfig) (*InstalledStreamRedirect, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if cfg.StdoutFile == "" && cfg.StderrFile == "" {
		return nil, nil
	}
	if !streamRedirectInstalled.CompareAndSwap(false, true) {
		return nil, errors.New("stream redirection already installed")
	}
	installed, err := installStreamRedirect(cfg)
	if err != nil {
		streamRedirectInstalled.Store(false)
		return nil, err
	}
	return installed, nil
}

func installStreamRedirect(cfg StreamRedirectConfig) (*InstalledStreamRedirect, error) {
	stdout, err := prepareStreamRedirectTarget(int(os.Stdout.Fd()), cfg.StdoutFile)
	if err != nil {
		return nil, err
	}
	stderr, err := prepareStreamRedirectTarget(int(os.Stderr.Fd()), cfg.StderrFile)
	if err != nil {
		closePreparedStreamRedirect(stdout)
		return nil, err
	}

	flushStandardStreams()

	if stdout != nil {
		if err := syscall.Dup2(int(stdout.file.Fd()), int(os.Stdout.Fd())); err != nil {
			closePreparedStreamRedirect(stdout)
			closePreparedStreamRedirect(stderr)
			return nil, err
		}
	}
	if stderr != nil {
		if err := syscall.Dup2(int(stderr.file.Fd()), int(os.Stderr.Fd())); err != nil {
			if stdout != nil {
				_ = syscall.Dup2(int(stdout.restore.Fd()), int(os.Stdout.Fd()))
			}
			closePreparedStreamRedirect(stdout)
			closePreparedStreamRedirect(stderr)
			return nil, err
		}
	}

	installed := &InstalledStreamRedirect{
		StdoutFile: cfg.StdoutFile,
		StderrFile: cfg.StderrFile,
	}
	if stdout != nil {
		installed.stdoutRestore = stdout.restore
		_ = stdout.file.Close()
	}
	if stderr != nil {
		installed.stderrRestore = stderr.restore
		_ = stderr.file.Close()
	}
	return installed, nil
}

type preparedStreamRedirect struct {
	file    *os.File
	restore *os.File
}

func prepareStreamRedirectTarget(targetFD int, path string) (*preparedStreamRedirect, error) {
	if path == "" {
		return nil, nil
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o666)
	if err != nil {
		return nil, err
	}
	restoreFD, err := syscall.Dup(targetFD)
	if err != nil {
		_ = file.Close()
		return nil, err
	}
	return &preparedStreamRedirect{
		file:    file,
		restore: os.NewFile(uintptr(restoreFD), "afdata-stream-restore"),
	}, nil
}

func closePreparedStreamRedirect(target *preparedStreamRedirect) {
	if target == nil {
		return
	}
	_ = target.file.Close()
	_ = target.restore.Close()
}

// Close restores original stdout/stderr.
func (r *InstalledStreamRedirect) Close() error {
	if r == nil || r.closed.Swap(true) {
		return nil
	}
	flushStandardStreams()
	var firstErr error
	if r.stdoutRestore != nil {
		if err := syscall.Dup2(int(r.stdoutRestore.Fd()), int(os.Stdout.Fd())); err != nil && firstErr == nil {
			firstErr = err
		}
		_ = r.stdoutRestore.Close()
	}
	if r.stderrRestore != nil {
		if err := syscall.Dup2(int(r.stderrRestore.Fd()), int(os.Stderr.Fd())); err != nil && firstErr == nil {
			firstErr = err
		}
		_ = r.stderrRestore.Close()
	}
	streamRedirectInstalled.Store(false)
	return firstErr
}
