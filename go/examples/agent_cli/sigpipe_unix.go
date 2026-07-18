//go:build unix

package main

import (
	"os"
	"os/signal"
	"syscall"
)

// installSigpipeHandler switches Go's default SIGPIPE behavior for stdout/stderr
// (fd 1 and 2) from "terminate the process" to "return an EPIPE error from the
// write". CliEmitter.Finish then sees that write failure and maps a broken pipe
// to a clean exit (0), so the tool never dies with a signal or a stack trace
// when its reader hangs up.
func installSigpipeHandler() {
	// Registering the signal is what flips the behavior; the channel is
	// intentionally never drained — only the registration matters.
	signal.Notify(make(chan os.Signal, 1), syscall.SIGPIPE)
}
