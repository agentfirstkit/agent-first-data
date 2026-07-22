# Bash authoring kit

`afdata shell bash` prints a sourceable Bash library. It gives executable
scripts AFDATA-style arguments, config reads, events, and child-process status
without changing the child program's output or interaction model.

```bash
#!/usr/bin/env bash
set -euo pipefail

_AFDATA_BASH_SOURCE="$("${AFDATA_BIN:-afdata}" shell bash)"
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE

afdata_args_begin "deploy.sh [OPTIONS] PROJECT [-- WRANGLER_ARG ...]"
afdata_args_option config_path --config PATH "Configuration file" config.toml
afdata_args_flag dry_run --dry-run "Prepare without deploying"
afdata_args_positional project PROJECT "Project to deploy"
afdata_args_rest WRANGLER_ARG "Arguments forwarded to wrangler"
afdata_args_parse "$@"

account_id="$(afdata_config_get "$config_path" cloudflare.account_id)"
afdata_log info "Preparing ${project} for account ${account_id}"

if [ "$dry_run" = false ]; then
  afdata_run wrangler deploy "${AFDATA_ARGS_REST[@]}"
fi

afdata_result "Deployment complete"
```

The library supports Bash 3.2 and later. Sourcing it is silent and does not
change `errexit`, `nounset`, `pipefail`, or any other caller option.

## Loading and pinning

Installing the CLI is sufficient; the Bash library is embedded in the binary:

```bash
cargo install --path . --force
afdata shell bash >/dev/null
```

For scripts that use the installed `afdata` version directly:

```bash
_AFDATA_BASH_SOURCE="$("${AFDATA_BIN:-afdata}" shell bash)"
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE
```

The assignment deliberately runs as its own command: under `set -e`, failure
to execute `afdata shell bash` stops the script instead of silently sourcing an
empty stream. `/dev/stdin` plus a here-string works with macOS Bash 3.2; process
substitution (`source <(...)`) is not portable across all Bash 3.2 builds.

For a reviewed, repository-pinned copy:

```bash
mkdir -p scripts/lib
afdata shell bash > scripts/lib/afdata.sh
```

Then source that file relative to the calling script. `bash/afdata.sh` is the
canonical source and is embedded into the `afdata` binary at compile time, so
`cargo install`, Homebrew, Scoop/Git Bash, and release archives all expose the
same `afdata shell bash` command without installing a separate shared-data
directory.

Set `AFDATA_BIN` to an alternate executable path when necessary. Event helpers
use `AFDATA_OUTPUT` (`json` by default) and `AFDATA_OUTPUT_TO` (`split` by
default).

After loading, `AFDATA_BASH_API_VERSION` contains the integer API version of
the helper surface (currently `1`). A script that depends on a pinned helper
contract can check this value immediately after sourcing; incompatible helper
changes increment it. `afdata_cli` is the public escape hatch for invoking the
selected `AFDATA_BIN` without duplicating its path lookup.

## Arguments

Start a declaration, add options, flags, and positional arguments, then parse:

```bash
afdata_args_begin "build.sh [OPTIONS] PACKAGE [-- CARGO_ARG ...]"
afdata_args_option config_path --config PATH "Configuration file" config.toml
afdata_args_flag release --release "Build release artifacts"
afdata_args_positional package PACKAGE "Package to build"
afdata_args_rest CARGO_ARG "Arguments forwarded to cargo"
afdata_args_parse "$@"
```

Declarations assign directly to their named Bash variables. Flags become
`true` or `false`; options and positional arguments are strings; trailing
arguments are preserved byte-for-byte in `AFDATA_ARGS_REST`. Both
`--config value` and `--config=value` work, and `--` ends option parsing.
Application variable names may not start with the reserved `_afdata_`,
`_AFDATA_`, or `AFDATA_` prefixes.

Every parser automatically supports:

- `-h` / `--help`
- `--output json|yaml|plain`
- `--output-to split|stdout|stderr`

Only long kebab-case application flags are accepted. Help exits with status
`0`; malformed arguments emit a `cli_error` event and exit with status `2`.
The selected `AFDATA_OUTPUT` and `AFDATA_OUTPUT_TO` values are exported so a
child Bash script using this authoring kit keeps the same event format and
routing.

## Config and events

`afdata_config_get FILE KEY [DEFAULT]` delegates to `afdata value`, so the same
format detection, dot-path grammar, empty-stdout-on-failure behavior, and
secret gate apply:

```bash
host="$(afdata_config_get config.toml server.host localhost)"
token_secret="$(afdata_cli value config.toml service.token_secret --reveal-secret)"
```

The event helpers are thin wrappers around `afdata emit`:

```bash
afdata_log info "Starting build"       # log event, stderr under split routing
afdata_result "Build complete"         # result event, stdout
afdata_error build_failed "Build failed" "Inspect the compiler output"
```

`afdata_error` returns status `1`. AFDATA deliberately does not scan free-form
messages for secrets, so never interpolate a secret into a message. Keep
sensitive values in `_secret`-named structured data or out of output entirely.

The equivalent CLI commands are available without sourcing the library:

```bash
afdata emit log info "Starting build"
afdata emit result "Build complete"
afdata emit error build_failed "Build failed" --hint "Inspect compiler output"
```

## Child processes

`afdata_run [--quiet] COMMAND [ARG ...]` emits a start log, runs the command,
then emits a completion log or a terminal `child_process_failed` error. It
returns the child's exact exit status even though the terminal error emitter
itself uses status `1`.

The child's stdin, stdout, stderr, signals, colors, prompts, progress bars, and
TTY access pass through unchanged. This is intentional: output from `cargo`,
`npm`, `wrangler`, and similar programs is not relabeled as AFDATA log events.
Only messages owned by the Bash script are structured. Command arguments are
also omitted from wrapper logs because they may contain secrets.

```bash
afdata_run cargo test --all-features
afdata_run npm publish
afdata_run wrangler login   # remains interactive

# Token-efficient noninteractive mode: discard successful child output, but
# replay the combined output on stderr when the child fails.
afdata_run --quiet cargo test --all-features
```

Use `afdata_run` for a raw child that is required for the enclosing operation.
A child failure is terminal, so run an expected or recoverable probe directly
inside `if` and describe the recovery with `afdata_log` instead. Also invoke a
command directly when its output is captured or piped as data: the enclosing
redirection applies to the wrapper too, so lifecycle events could enter the
captured stream.

When one AFDATA Bash script orchestrates another and the parent will emit the
final result, use `afdata_call`:

```bash
afdata_call "$SCRIPT_DIR/build-assets.sh"
afdata_call "$SCRIPT_DIR/deploy.sh"
afdata_result "Release complete"
```

`afdata_call [--] COMMAND [ARG ...]` preserves the child's live logs, errors,
TTY, and exit status, but marks the child as participating in its parent's
finite event stream. A successful child `afdata_result` becomes an `info` log;
the outermost script therefore remains the only owner of the terminal result.
A child error remains terminal. This cooperation applies only to children that
load this Bash kit. For another AFDATA CLI whose successful result is merely an
internal check, capture or suppress that result explicitly; for a raw program,
use `afdata_run`.

`--quiet` is an explicit child-output policy, independent of AFDATA log format
and routing. It buffers combined stdout/stderr in memory, discards it after a
successful command, and is intended only for noninteractive work. Buffering in
memory rather than a temporary file means an interrupted or signalled script
leaves nothing to clean up. Successful output is discarded to save tokens;
failed output is replayed to stderr before the terminal structured error. Do
not use it for prompts, progress that must remain visible, servers, or commands
whose successful output is a result the caller needs.

Passthrough mode deliberately permits native child bytes alongside AFDATA
events. Use `--quiet`, `afdata_call`, or explicit capture when a consumer needs
a protocol-only stream with exactly one terminal event.
