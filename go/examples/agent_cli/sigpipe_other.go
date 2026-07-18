//go:build !unix

package main

// installSigpipeHandler is a no-op on platforms without SIGPIPE; a broken-pipe
// write already surfaces as an error there, which CliEmitter.Finish handles.
func installSigpipeHandler() {}
