#!/usr/bin/env bash
# End-to-end checks for the sourceable Bash authoring kit.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOTPATH="$(cd "$SCRIPT_DIR/.." && pwd)"
AFDATA_BIN="${AFDATA_BIN:?set AFDATA_BIN to the afdata executable under test}"
TEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/afdata-bash-e2e.XXXXXX")"
trap 'rm -rf "$TEST_TMP"' EXIT

fail() {
  printf 'Bash e2e failed: %s\n' "$1" >&2
  exit 1
}

"$AFDATA_BIN" shell bash > "$TEST_TMP/exported.sh"
cmp "$ROOTPATH/bash/afdata.sh" "$TEST_TMP/exported.sh" \
  || fail "afdata shell bash differs from bash/afdata.sh"
bash -n "$ROOTPATH/bash/afdata.sh"

source_output="$({
  AFDATA_BIN="$AFDATA_BIN" bash -c '
    set -euo pipefail
    before="$(set +o)"
    source "$1"
    after="$(set +o)"
    [ "$before" = "$after" ]
  ' bash "$ROOTPATH/bash/afdata.sh"
} 2>&1)"
[ -z "$source_output" ] || fail "sourcing the library produced output"

legacy_bash=/bin/bash
[ -x "$legacy_bash" ] || legacy_bash="$(command -v bash)"
# The single-quoted program is intentionally expanded by the child Bash.
# shellcheck disable=SC2016
"$legacy_bash" -c '
  set -euo pipefail
  exported_library="$("$1" shell bash)"
  source /dev/stdin <<<"$exported_library"
  unset exported_library
  declare -F afdata_log >/dev/null
' bash "$AFDATA_BIN"

# The absolute path is resolved at runtime; the library is checked separately.
# shellcheck disable=SC1091
source "$ROOTPATH/bash/afdata.sh"
export AFDATA_BIN
export AFDATA_OUTPUT="json"
export AFDATA_OUTPUT_TO="split"
# This file tests top-level behavior, so it must start from a clean
# event-ownership state even when launched inside an afdata_call subtree — as
# the project test runner does, which exports _AFDATA_BASH_CHILD=1 into every
# descendant. An inherited marker would demote afdata_result to an info log.
# The afdata_call sections below re-establish it deliberately for the child.
unset _AFDATA_BASH_CHILD
[ "$AFDATA_BASH_API_VERSION" -eq 1 ] \
  || fail "unexpected Bash authoring API version"

cat > "$TEST_TMP/config.toml" <<'TOML'
[server]
host = "example.com"
TOML

[ "$(afdata_config_get "$TEST_TMP/config.toml" server.host)" = example.com ] \
  || fail "afdata_config_get did not read the configured value"
[ "$(afdata_config_get "$TEST_TMP/config.toml" server.port 8080)" = 8080 ] \
  || fail "afdata_config_get did not apply the default"

result_output="$(afdata_result "Build complete")"
[ "$(printf '%s' "$result_output" | afdata_cli value - kind)" = result ] \
  || fail "afdata_result did not emit a result event"
[ "$(printf '%s' "$result_output" | afdata_cli value - result.message)" = "Build complete" ] \
  || fail "afdata_result changed the message"

afdata_log info "Building project" > "$TEST_TMP/log.stdout" 2> "$TEST_TMP/log.stderr"
[ ! -s "$TEST_TMP/log.stdout" ] || fail "afdata_log wrote to stdout under split routing"
[ "$(afdata_cli value --input-format json "$TEST_TMP/log.stderr" kind)" = log ] \
  || fail "afdata_log did not emit a log event"
[ "$(afdata_cli value --input-format json "$TEST_TMP/log.stderr" log.level)" = info ] \
  || fail "afdata_log changed the level"

if afdata_error build_failed "Build failed" "Inspect child output" \
  > "$TEST_TMP/error.stdout" 2> "$TEST_TMP/error.stderr"; then
  fail "afdata_error returned success"
else
  error_status=$?
fi
[ "$error_status" -eq 1 ] || fail "afdata_error did not return status 1"
[ ! -s "$TEST_TMP/error.stdout" ] || fail "afdata_error wrote to stdout under split routing"
[ "$(afdata_cli value --input-format json "$TEST_TMP/error.stderr" error.code)" = build_failed ] \
  || fail "afdata_error changed the code"

config_path=""
dry_run=false
project=""
afdata_args_begin "demo.sh [OPTIONS] PROJECT"
afdata_args_option config_path --config PATH "Configuration file" config.toml
afdata_args_flag dry_run --dry-run "Do not perform writes"
afdata_args_positional project PROJECT "Project to build"
afdata_args_rest ARG "Arguments forwarded to the child command"
afdata_args_parse --config custom.toml --dry-run --output plain demo -- --locked feature-x
[ "$config_path" = custom.toml ] || fail "argument option was not assigned"
[ "$dry_run" = true ] || fail "argument flag was not assigned"
[ "$project" = demo ] || fail "positional argument was not assigned"
[ "$AFDATA_OUTPUT" = plain ] || fail "built-in --output was not assigned"
[ "${AFDATA_ARGS_REST[0]}" = --locked ] || fail "first rest argument was not preserved"
[ "${AFDATA_ARGS_REST[1]}" = feature-x ] || fail "second rest argument was not preserved"

propagated_output="$(
  unset AFDATA_OUTPUT AFDATA_OUTPUT_TO
  afdata_args_begin "propagate.sh"
  afdata_args_parse --output plain --output-to stdout
  bash -c 'printf "%s:%s" "$AFDATA_OUTPUT" "$AFDATA_OUTPUT_TO"'
)"
[ "$propagated_output" = plain:stdout ] \
  || fail "argument output routing was not exported to child commands"

# An AFDATA Bash child contributes logs to its parent's stream, but only the
# outermost script owns the unique terminal result.
AFDATA_OUTPUT=json
AFDATA_OUTPUT_TO=stdout
{
  # The single-quoted program is intentionally expanded by the child Bash.
  # shellcheck disable=SC2016
  afdata_call bash -c '
    set -euo pipefail
    source "$1"
    afdata_log info "Child started"
    afdata_result "Child complete"
  ' bash "$ROOTPATH/bash/afdata.sh"
  afdata_result "Parent complete"
} > "$TEST_TMP/call.stream"
AFDATA_OUTPUT_TO="split"
afdata_cli validate "$TEST_TMP/call.stream" >/dev/null \
  || fail "afdata_call did not produce a valid parent-owned event stream"
[ "$(grep -c '"kind":"log"' "$TEST_TMP/call.stream")" -eq 2 ] \
  || fail "afdata_call did not keep child completion diagnostic"
[ "$(grep -c '"kind":"result"' "$TEST_TMP/call.stream")" -eq 1 ] \
  || fail "afdata_call allowed more than one terminal result"
grep -q '"message":"Child complete"' "$TEST_TMP/call.stream" \
  || fail "afdata_call discarded the child completion message"
[ "${_AFDATA_BASH_CHILD:-0}" = 0 ] \
  || fail "afdata_call leaked its child marker into the caller"

# A cooperating child's error remains the unique terminal event and its status
# propagates unchanged; a parent must not recover and append a later result.
AFDATA_OUTPUT_TO=stdout
# The single-quoted program is intentionally expanded by the child Bash.
# shellcheck disable=SC2016
if afdata_call bash -c '
  set -euo pipefail
  source "$1"
  afdata_error child_failed "Child failed"
' bash "$ROOTPATH/bash/afdata.sh" > "$TEST_TMP/call-failure.stream"; then
  fail "afdata_call discarded a child error"
else
  call_failure_status=$?
fi
AFDATA_OUTPUT_TO="split"
[ "$call_failure_status" -eq 1 ] || fail "afdata_call changed the child error status"
afdata_cli validate "$TEST_TMP/call-failure.stream" >/dev/null \
  || fail "afdata_call child failure was not a valid terminal stream"
[ "$(grep -c '"kind":"error"' "$TEST_TMP/call-failure.stream")" -eq 1 ] \
  || fail "afdata_call child failure did not contain exactly one terminal error"

# Bash uses dynamic scope, so application variable names must not collide with
# parser locals. These names collided before parser internals gained a reserved
# prefix; the optional positional also verifies declaration-time initialization.
index="sentinel"
arg="sentinel"
mode="sentinel"
afdata_args_begin "collision.sh [OPTIONS] [MODE]"
afdata_args_option index --index VALUE "Index value"
afdata_args_flag arg --arg "Argument flag"
afdata_args_positional mode MODE "Optional mode" optional
afdata_args_parse --index chosen --arg
[ "$index" = chosen ] || fail "option variable collided with parser internals"
[ "$arg" = true ] || fail "flag variable collided with parser internals"
[ -z "$mode" ] \
  || fail "optional positional variable collided with declaration internals"

if AFDATA_BIN="$AFDATA_BIN" AFDATA_OUTPUT=json AFDATA_OUTPUT_TO=split bash -c '
  set -euo pipefail
  source "$1"
  afdata_args_begin "demo.sh [OPTIONS]"
  afdata_args_parse --unknown
' bash "$ROOTPATH/bash/afdata.sh" > "$TEST_TMP/args.stdout" 2> "$TEST_TMP/args.stderr"; then
  fail "unknown argument returned success"
else
  args_status=$?
fi
[ "$args_status" -eq 2 ] || fail "unknown argument did not return status 2"
[ ! -s "$TEST_TMP/args.stdout" ] || fail "argument error wrote to stdout under split routing"
[ "$(afdata_cli value --input-format json "$TEST_TMP/args.stderr" error.code)" = cli_error ] \
  || fail "argument error was not structured"

AFDATA_OUTPUT=json
if afdata_run bash -c '
  printf "child stdout\n"
  printf "child stderr\n" >&2
  exit 7
' > "$TEST_TMP/run.stdout" 2> "$TEST_TMP/run.stderr"; then
  fail "afdata_run discarded the child failure"
else
  run_status=$?
fi
[ "$run_status" -eq 7 ] || fail "afdata_run changed the child exit status"
[ "$(cat "$TEST_TMP/run.stdout")" = "child stdout" ] \
  || fail "afdata_run changed child stdout"
grep -qx 'child stderr' "$TEST_TMP/run.stderr" \
  || fail "afdata_run changed child stderr"
[ "$(grep -c '"kind":"log"' "$TEST_TMP/run.stderr")" -eq 1 ] \
  || fail "afdata_run did not emit its start log"
[ "$(grep -c '"kind":"error"' "$TEST_TMP/run.stderr")" -eq 1 ] \
  || fail "afdata_run did not emit one terminal failure"
[ "$(grep '"kind":"error"' "$TEST_TMP/run.stderr" | afdata_cli value - error.code)" = child_process_failed ] \
  || fail "afdata_run used the wrong child failure code"

afdata_run --quiet bash -c '
  printf "quiet child stdout\n"
  printf "quiet child stderr\n" >&2
' > "$TEST_TMP/quiet.stdout" 2> "$TEST_TMP/quiet.stderr"
[ ! -s "$TEST_TMP/quiet.stdout" ] || fail "quiet afdata_run leaked successful stdout"
if grep -q 'quiet child' "$TEST_TMP/quiet.stderr"; then
  fail "quiet afdata_run leaked successful child output"
fi
[ "$(grep -c '"kind":"log"' "$TEST_TMP/quiet.stderr")" -eq 2 ] \
  || fail "quiet afdata_run did not emit lifecycle logs"

if afdata_run --quiet bash -c '
  printf "failed child stdout\n"
  printf "failed child stderr\n" >&2
  exit 9
' > "$TEST_TMP/quiet-failure.stdout" 2> "$TEST_TMP/quiet-failure.stderr"; then
  fail "quiet afdata_run discarded the child failure"
else
  quiet_failure_status=$?
fi
[ "$quiet_failure_status" -eq 9 ] || fail "quiet afdata_run changed the child exit status"
[ ! -s "$TEST_TMP/quiet-failure.stdout" ] || fail "quiet afdata_run replayed failure on stdout"
grep -qx 'failed child stdout' "$TEST_TMP/quiet-failure.stderr" \
  || fail "quiet afdata_run did not replay failed stdout"
grep -qx 'failed child stderr' "$TEST_TMP/quiet-failure.stderr" \
  || fail "quiet afdata_run did not replay failed stderr"
[ "$(grep -c '"kind":"log"' "$TEST_TMP/quiet-failure.stderr")" -eq 1 ] \
  || fail "quiet afdata_run did not emit its start log"
[ "$(grep -c '"kind":"error"' "$TEST_TMP/quiet-failure.stderr")" -eq 1 ] \
  || fail "quiet afdata_run did not emit one terminal failure"

# Quiet mode keeps a unified machine stream protocol-only even when the raw
# child fails; replayed diagnostics stay on stderr and the exact status survives.
AFDATA_OUTPUT_TO=stdout
if afdata_run --quiet bash -c '
  printf "unified failure diagnostic\n"
  exit 6
' > "$TEST_TMP/unified-failure.stream" 2> "$TEST_TMP/unified-failure.stderr"; then
  fail "unified quiet afdata_run discarded the child failure"
else
  unified_failure_status=$?
fi
AFDATA_OUTPUT_TO="split"
[ "$unified_failure_status" -eq 6 ] \
  || fail "unified quiet afdata_run changed the child exit status"
grep -qx 'unified failure diagnostic' "$TEST_TMP/unified-failure.stderr" \
  || fail "unified quiet afdata_run did not replay native diagnostics"
afdata_cli validate "$TEST_TMP/unified-failure.stream" >/dev/null \
  || fail "unified quiet afdata_run did not emit a valid terminal stream"
[ "$(grep -c '"kind":"error"' "$TEST_TMP/unified-failure.stream")" -eq 1 ] \
  || fail "unified quiet afdata_run did not emit exactly one terminal error"

# afdata_run also accepts shell functions. Its locals must not intercept
# assignments made by that function through Bash's dynamic scope.
command_name=before
child_status=before
mutate_caller_state() {
  command_name=after
  child_status=after
}
afdata_run mutate_caller_state > "$TEST_TMP/function.stdout" 2> "$TEST_TMP/function.stderr"
[ "$command_name" = after ] || fail "afdata_run shadowed a child function variable"
[ "$child_status" = after ] || fail "afdata_run shadowed a child function status variable"

printf 'Bash authoring kit checks passed.\n'
