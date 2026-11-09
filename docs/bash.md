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
  afdata_run wrangler deploy ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
fi

afdata_result "Deployment complete"
```

The library supports Bash 3.2 and later. Sourcing it is silent and does not
change `errexit`, `nounset`, `pipefail`, or any other caller option.

Bash 3.2 — still the stock macOS shell — aborts on `"${arr[@]}"` when the array
is empty and `nounset` is set. `AFDATA_ARGS_REST` is empty whenever the caller
passes no trailing arguments, so forward it with the guarded expansion
`${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}` shown above, and use the same
form for any array of your own that may be empty.

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
the helper surface (currently `2`). A script that depends on a pinned helper
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

- `--help`
- `--output json|yaml|plain`
- `--output-to split|stdout|stderr`

Only long kebab-case application flags are accepted. Help exits with status
`0`; malformed arguments emit a `cli_error` event and exit with status `2`.
The selected `AFDATA_OUTPUT` and `AFDATA_OUTPUT_TO` values are exported so a
child Bash script using this authoring kit keeps the same event format and
routing.

## Scripts whose stdout is a bare value

Some scripts exist to be captured: they print one path or token for a caller's
`$(...)`. Such a script must not let `--output-to stdout`, or an inherited
`AFDATA_OUTPUT_TO=stdout`, mix its events into the captured value. This is the
Bash counterpart of the CLI's raw commands (`value`, `guard`), which write a
bare value and reject `--output` and `--output-to`.

Declare it with `afdata_args_raw_output`, between `afdata_args_begin` and
`afdata_args_parse`. The declaration is shell-internal state, never inherited,
so a child script is unaffected. Calling it before `afdata_args_begin` (which
resets the declaration) or after `afdata_args_parse` is a usage error (status 2):

```bash
afdata_args_begin "locate.sh [OPTIONS] NAME"
afdata_args_raw_output
afdata_args_positional name NAME "Name to locate"
afdata_args_parse "$@"

afdata_log info "Resolving ${name}"
printf '%s' "/opt/tools/${name}"
```

With the declaration the parser rejects `--output` and `--output-to`
(`cli_error`, exit `2`), every event the kit emits goes to stderr whatever
`AFDATA_OUTPUT_TO` the parent exported, and `afdata_result` is a function usage
error (status `2`) because the script writes its value itself. Errors still
reach stderr with the usual exit status.

The caller captures it directly. Do not route it through `afdata_call`, which
exists for children that take part in the parent's event stream and would not
hand back the bare value:

```bash
tool_path="$(./locate.sh widget)"
```

Use this form only for a script whose whole job is that one value; a script
that reports a result belongs on `afdata_result`.

## Config and events

`afdata_config_get FILE KEY [DEFAULT]` delegates to `afdata value`, so the same
format detection, dot-path grammar, empty-stdout-on-failure behavior, and
secret gate apply:

```bash
host="$(afdata_config_get config.toml server.host localhost)"
token_secret="$(afdata_cli value config.toml service.token_secret --reveal-secret)"
```

To build the JSON a config file or `afdata_result --json` needs, use
`afdata object` and `afdata array` instead of `printf` templates; see the
documents reference, "Building JSON in a shell".

### Iterating a container safely

`afdata paths`, `keys` and `values` print one entry per line, which is
convenient until a key or value contains a newline — JSON permits it, and a
line-based loop then silently reads one entry as two. It does not fail; it just
quietly does the wrong thing.

`afdata_into ARRAY_NAME VERB ARGS...` reads the entries into a Bash array with
NUL framing, so every entry survives intact and an empty entry stays an empty
element. `VERB` is `values`, `keys` or `paths`; `--null` is added for you:

```bash
afdata_into names values config.json 'names.*' --missing-ok
for name in ${names[@]+"${names[@]}"}; do
  afdata_log info "found ${name}"
done
```

Call it as a plain command, not inside `$(...)`, so afdata's exit status reaches
`set -e`. On failure the array is left empty and afdata's status is returned;
the helper returns, it never exits. `ARRAY_NAME` must be a plain variable name.
It works on Bash 3.2 (no `mapfile`, no namerefs) by reading through a temporary
file that is removed before it returns. It reads scalar arrays and object keys
or child paths; for arrays of objects, list the paths and read each element.

When you would rather not fill an array, use `--null` yourself; NUL is the one
byte a key or value cannot contain:

```bash
# Wrong for a key like "weird\nkey" — no error, just an extra iteration.
afdata paths config.json | while IFS= read -r path; do ...; done

# Unambiguous.
while IFS= read -r -d "" path; do
  afdata_log info "found ${path}"
done < <(afdata paths config.json --null)

# Or hand the whole set to another program.
afdata paths config.json --null | xargs -0 -n1 echo
```

Use `--null` whenever the output is consumed by a program rather than read by a
person. `keys` and `values` take the same flag.

The event helpers are thin wrappers around `afdata emit`:

```bash
afdata_log info "Starting build"       # log event, stderr under split routing
afdata_result "Build complete"         # result event, stdout
afdata_result --json '{"duration_ms":12,"count":3}'  # typed, redacted result
afdata_error build_failed "Build failed" "Inspect the compiler output"
```

`afdata_error` returns status `1`. AFDATA deliberately does not scan free-form
messages for secrets, so never interpolate a secret into a message. Keep
sensitive values in `_secret`-named structured data or out of output entirely.

The equivalent CLI commands are available without sourcing the library:

```bash
afdata emit log info "Starting build"
afdata emit result "Build complete"
afdata emit result --json '{"duration_ms":12,"count":3}'
afdata emit error build_failed "Build failed" --hint "Inspect compiler output"
```

## Destructive operands

`afdata_remove [--quiet] TYPE VALUE [--under ROOT]` removes a path, but only after
`afdata guard` has approved it. `TYPE` and `--under` are forwarded to the guard
untouched, so the call reads as the guard it wraps:

```bash
afdata_remove tmp_path "$work_dir"
afdata_remove path "$out_dir/$name" --under "$out_dir"
```

It exists because the inline spelling cannot be made to fail. In an argument
position a command substitution's exit status is discarded, and `rm -f` is
defined to exit 0 when its operand names nothing, so
`afdata guard tmp_path "$dir"` substituted into the operand of `rm -rf --` leaves the target untouched — the
guard working — while the script continues as though the cleanup had happened.
As a command, `afdata_remove` returns the guard's own status, so `errexit`
stops on a rejection. Removing a path that does not exist succeeds, matching
`rm -rf`, so repeated cleanup stays idempotent. In an `EXIT` trap that cleans
several paths, `errexit` makes the first rejection abort the trap and the later
paths are never reached; append `|| :` per call when every path must be tried.

`--quiet` silences the guard's rejection event while keeping its exit status.

`afdata_mktemp [-d] VAR` creates a temporary file (directory with `-d`) under
`$TMPDIR`, checks it with `guard tmp_path`, stores the path in `VAR`, and removes
it when the shell exits. A `$TMPDIR` outside the platform temp area is reported
as `tmpdir_outside_platform_temp` and nothing is left behind. The cleanup is
chained onto an `EXIT` trap the caller already set, which runs first and must
not call `exit` (the cleanup would be skipped); a trap set afterwards
replaces it, and the next `afdata_mktemp` call re-attaches it. Call it in the
main shell, not inside `$(...)`, where neither `VAR` nor the trap would survive.

`afdata_try OUT_VAR STATUS_VAR COMMAND [ARG ...]` captures stdout and stderr
merged into `OUT_VAR` and the exit status into `STATUS_VAR`, and always returns
0, so `set -e` never fires on the captured failure:

```bash
afdata_try out status some-command --flag
[ "$status" -eq 3 ] || fail "unexpected status $status"
```

The command runs in a subshell, so a function's variable assignments do not
reach the caller.

`afdata_trash TYPE VALUE [--under ROOT]` has the same shape but moves the entry
to the platform's trash instead of deleting it. Which of the two to use is a
decision made in the source, not guessed at runtime: build output and `mktemp`
scratch belong to `afdata_remove`, because trashing every run's scratch fills a
person's trash with things nobody will restore; a person's own files belong to
`afdata_trash`, where the system owns expiry and restore.

```bash
afdata_trash path "$document" --under "$library"
```

It never falls back to deleting. When the entry cannot be made reversible it
fails with `trash_unavailable` and touches nothing — no trash on the host, or a
target on a network location, on a volume whose trash is switched off, or one
too large for the trash to hold. Work that must proceed there wanted
`afdata_remove` and should say so. Two rarer codes mean something else:
`trash_not_reversible` says the entry is gone and cannot be restored, and
`trash_incomplete` says the run stopped somewhere unknown, so the target's state
has to be checked. Neither is a retry.

The trash entry point is the platform's own — `trash`, `gio trash`,
`trash-put`, or on Windows the shell operation Explorer performs. Placement,
naming collisions, expiry and the "put back" record stay that platform's
contract; this library adds no state of its own.

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

### Background processes

A script that starts a daemon for an acceptance run needs the same four
steps every time: start it, wait for its first line, wait for it with a
deadline, and stop it without leaving a zombie. The kit provides them:

```bash
afdata_spawn out.log "&1" my-daemon --port 0   # sets AFDATA_SPAWN_PID
line="$(afdata_wait_line out.log "$AFDATA_SPAWN_PID" 10)" || exit 1
afdata_stop INT "-$AFDATA_SPAWN_PID"           # whole group; TERM-ignorers get KILL
afdata_timeout 30 some-command arg             # 124 when it ran past 30 seconds
```

- `afdata_spawn OUT ERR COMMAND [ARG ...]` runs the command with stdin from
  `/dev/null`, stdout to `OUT`, stderr to `ERR` (the literal `&1` merges it
  into `OUT`), in its own process group so a signal reaches its descendants.
  It enables `set -m` only for the launch and restores the caller's monitor
  state, so the kit's promise not to change shell options holds.
- `afdata_wait_line OUT PID SECONDS` prints the first complete line of `OUT`.
  It returns `1` on timeout. If `PID` exits first, it still prints a complete
  line written before exiting and returns `0`; with no complete line (nothing
  written, or only a partial line) it prints nothing and returns `3`.
  `SECONDS` is a whole number.
- `afdata_stop SIGNAL TARGET [SECONDS]` signals a pid, or `-pid` for the whole
  group, waits `SECONDS` (default 10, whole seconds), escalates to `KILL` with a `warn` log,
  and always reaps the child. It returns `0` once the process is gone.
- `afdata_timeout SECONDS COMMAND [ARG ...]` (whole seconds; anything else is a
  usage error, status `2`) keeps the caller's stdin and
  redirections and runs the command in its own process group. A command that
  runs too long has its whole group stopped with `afdata_stop TERM` (KILL after
  5 seconds), so children it started cannot hold a `$(...)` pipe open; it logs a
  `warn` and returns `124`. Otherwise it returns the command's own status. The
  command runs in a subshell: a function's variable assignments do not reach the
  caller. The wait uses `command sleep`, so a caller-defined `sleep` function
  does not affect the deadline. Its own process group is not the terminal's
  foreground group, so a command that reads the terminal (a password prompt)
  is stopped until the deadline; give it non-interactive input instead.
- A command started by `afdata_spawn` or `afdata_timeout` never carries the
  caller's `EXIT` trap, so stopping it early cannot run the caller's cleanup
  in the child.

Deadlines count `sleep 0.1` ticks rather than reading a clock, because Bash 3.2
has no sub-second clock; a deadline can arrive late but never early. Job-control
notices such as `Terminated: 15` are suppressed during `afdata_stop`.
