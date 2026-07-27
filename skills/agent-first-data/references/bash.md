<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# AFDATA Bash authoring

Use the bundled Bash 3.2+ authoring kit for argument parsing, config reads,
structured events, and child-process handling. Do not rebuild these pieces with
custom long-option parsing or ad hoc JSON.

## Contents

- [Load](#load)
- [Declare arguments](#declare-arguments)
- [Config and events](#config-and-events)
- [Child processes](#child-processes)
- [Forwarding trailing arguments](#forwarding-trailing-arguments)
- [Minimal pattern](#minimal-pattern)

## Load

```bash
_AFDATA_BASH_SOURCE="$("${AFDATA_BIN:-afdata}" shell bash)"
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE
```

Pin `AFDATA_BIN` when reproducibility requires a specific binary.

## Declare arguments

Call `afdata_args_begin`, declare options in help order, then parse:

```bash
afdata_args_begin "build.sh [OPTIONS] PACKAGE [-- CARGO_ARG ...]"
afdata_args_option profile --profile NAME "Build profile" "dev"
afdata_args_flag release --release "Build release artifacts"
afdata_args_positional package PACKAGE "Package to build"
afdata_args_rest CARGO_ARG "Arguments forwarded to cargo"
afdata_args_parse "$@"
```

Use `afdata_args_option`, `afdata_args_flag`, `afdata_args_positional`, and
`afdata_args_rest`. The kit owns help, usage errors, AFDATA output flags, and
secret-default redaction — never redeclare `--output`, `--output-to`, or
`--help`. `afdata_args_rest` takes only a display name and a description;
trailing arguments always land in `AFDATA_ARGS_REST`, never a variable you name.

## Config and events

- `afdata_config_get` reads a raw scalar config value.
- `afdata_log LEVEL MESSAGE` emits a structured log.
- `afdata_result MESSAGE` emits terminal success.
- `afdata_error CODE MESSAGE` emits terminal failure.

Keep secret values in `_secret`-named structured fields. Never interpolate a
secret into a free-form message or child-command description.

## Child processes

Use `afdata_run command args...`. It executes the child directly, preserving
stdin, stdout, stderr, TTY interaction, colors, prompts, and exit status.
Start/completion are log events; failure becomes terminal
`child_process_failed`.

For a noninteractive command whose successful output is irrelevant, use
`afdata_run --quiet`. It discards successful child output to save tokens and
replays combined output on stderr if the child fails. Never use quiet mode for
prompts, interactive commands, or servers.

When an AFDATA Bash parent calls another AFDATA Bash script and the parent must
own the only terminal result, use `afdata_call`. It preserves child logs and
errors but converts a successful child result to an informational log.

## Forwarding trailing arguments

Bash 3.2 — the stock macOS shell — treats `"${arr[@]}"` on an *empty* array as
an unbound variable under `set -u`, aborting the script mid-run. Always expand a
possibly-empty array with the guarded form:

```bash
afdata_run cargo build ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
```

Prefer plain branches over building an array when the choice is a single flag.

## Minimal pattern

```bash
#!/usr/bin/env bash
set -euo pipefail

_AFDATA_BASH_SOURCE="$("${AFDATA_BIN:-afdata}" shell bash)"
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE

afdata_args_begin "build.sh [OPTIONS] PACKAGE [-- CARGO_ARG ...]"
afdata_args_flag release --release "Build release artifacts"
afdata_args_positional package PACKAGE "Package to build"
afdata_args_rest CARGO_ARG "Arguments forwarded to cargo"
afdata_args_parse "$@"

afdata_log info "Building ${package}"
if [ "$release" = true ]; then
  afdata_run cargo build --release ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
else
  afdata_run cargo build ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
fi
afdata_result "Build complete"
```

In a repository checkout, consult `docs/bash.md` only when an API detail or
edge case is not covered here.
