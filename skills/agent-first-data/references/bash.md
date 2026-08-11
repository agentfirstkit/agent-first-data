<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# AFDATA Bash authoring

Use the bundled Bash 3.2+ authoring kit for argument parsing, config reads,
structured events, and child-process handling. Do not rebuild these pieces with
custom long-option parsing or ad hoc JSON.

## Contents

- [Load](#load)
- [Declare arguments](#declare-arguments)
- [Config and events](#config-and-events)
- [Destructive operands](#destructive-operands)
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
structured output routing — never redeclare `--output`, `--output-to`, or
`--help`. `afdata_args_rest` takes only a display name and a description;
trailing arguments always land in `AFDATA_ARGS_REST`, never a variable you name.
Usage errors name the option or argument category but never echo a supplied
value. Output selectors from an argv are committed only after that whole argv
parses successfully, so a malformed call cannot redirect or reformat its own
usage error. Preserve both properties in wrapper diagnostics.

## Config and events

- `afdata_config_get` reads a raw scalar config value.
- `afdata_log LEVEL MESSAGE` emits a structured log.
- `afdata_result MESSAGE` emits terminal success.
- `afdata_error CODE MESSAGE` emits terminal failure.

Keep secret values in `_secret`-named structured fields. Never interpolate a
secret into a free-form message or child-command description.

## Destructive operands

A variable flowing into a verb that deletes or overwrites is validated with
`afdata guard <TYPE> <VALUE> [--under ROOT]`. It prints the normalized absolute
path and fails closed to blank stdout. Quoting rules govern how a string is
passed along; they never judge whether the value is a disaster, because `/`,
`$HOME`, and a project root are all non-empty strings.

**Never place a guard call in an argument position.** There the guard's exit
status is discarded, and `rm -f` is defined to exit 0 when its operand names
nothing — so a rejected value leaves the target untouched while the script runs
on as though the cleanup had succeeded. Use the kit function for removal, or
assign first and use the variable:

```bash
afdata_remove tmp_path "$work_dir"                 # removal: one command
afdata_trash path "$doc" --under "$library"        # reversible removal
target="$(afdata_cli guard path "$out_dir/$name" --under "$out_dir")"
printf '%s\n' "$rendered" > "$target"              # any other verb: two steps
```

Put the guard ahead of the first command that touches the filesystem, including
the `mkdir -p` that creates the directory about to be written. It accepts a path
that does not exist yet, so the order costs nothing, while the reverse order
lets the least informative failure win: an unset value dies as
`mkdir: : No such file or directory` instead of as `guard_empty_value`.

Choose between the two removals in the source, never at runtime. Build output,
staging directories, and `mktemp` scratch go to `afdata_remove`: they are meant
to disappear, and trashing every run's scratch space fills a person's trash with
things nobody will restore. Files a person owns go to `afdata_trash`, where the
desktop keeps expiry and restore.

`afdata_trash` fails with `trash_unavailable` rather than deleting whenever this
target cannot be made reversible — no trash on the host, or a target on a
network location, on a volume whose trash is switched off, or too large for the
trash to hold. Nothing is touched in that case, so work that must proceed anyway
wanted `afdata_remove` and should say so. Two rarer codes do not mean that:
`trash_not_reversible` says the entry is already gone and cannot be restored,
and `trash_incomplete` says the run stopped somewhere unknown and the target's
state has to be checked. Neither is a retry; both want a person to look.

```bash
rm -rf -- "$(afdata_cli guard tmp_path "$work_dir")"   # never: cannot stop
```

Submit the operand the verb will actually receive, not a prefix of it. A suffix
holding a variable must be part of the guarded value, since an empty or `..`
suffix is exactly what containment exists to catch. A suffix written literally
at the call site is exempt — it can be neither empty nor traversal — so guard
the directory once and write literal names beneath it. The exemption is about
containment only: verbs that follow a final symlink (`>`, `cp`, `chmod`) are
governed per path, so guard the full path when the directory holds entries this
script did not create.

Choose `--under ROOT` as the directory whose entire contents you would accept
losing, not as a sandbox boundary. A checkout root reads as safe and is the
wrong answer: it leaves every sibling directory reachable. `VALUE` carrying a
`..` segment is refused outright (`guard_traversal_segment`) rather than
normalized away.

The root belongs to the call site, not to the function being called. A helper
shared by callers that legitimately write all over a checkout cannot name one,
and the checkout root is not the fallback — it reads as protection while leaving
every sibling reachable. Use `path` there, which still refuses an unset value,
`/`, `$HOME` and the current directory, and give the root in the callers that
have a tight one. Saying "no honest root exists here" is a real answer; a wide
root that looks like compliance is not.

Never assign a guard's output back over its own input — `dir="$(afdata_cli
guard tmp_path "$dir")"` reads as normalization and protects no verb. To assert
the environment up front, discard stdout and keep the status:

```bash
afdata_cli guard tmp_path "$work_dir" >/dev/null || exit 1
```

When stating what a rejection guarantees, say the target is not touched. Do not
say the verb will report an error: `-f` suppresses exactly that, on every
platform.

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
