#!/usr/bin/env bash
# End-to-end checks for the sourceable Bash authoring kit.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOTPATH="$(cd "$SCRIPT_DIR/.." && pwd)"
AFDATA_BIN="${AFDATA_BIN:?set AFDATA_BIN to the afdata executable under test}"
TEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/afdata-bash-e2e.XXXXXX")"
trap 'rm -rf "$TEST_TMP"' EXIT

# A Windows-native Bash makes `ln -s` copy the target instead of linking to it
# unless asked for real links. Left alone, every symlink case below would be
# testing a plain directory — passing or failing for reasons that have nothing
# to do with what the guard decides about links. Asking for native links makes
# the platform either provide them or fail outright, and the assertion after the
# first one turns a silent downgrade into a named failure. Harmless elsewhere:
# nothing but MSYS reads this.
export MSYS=winsymlinks:nativestrict

fail() {
  printf 'Bash e2e failed: %s\n' "$1" >&2
  exit 1
}

"$AFDATA_BIN" shell bash > "$TEST_TMP/exported.sh"
cmp "$ROOTPATH/bash/afdata.sh" "$TEST_TMP/exported.sh" \
  || fail "afdata shell bash differs from bash/afdata.sh"
"${OLDEST_BASH:-/bin/bash}" -n "$ROOTPATH/bash/afdata.sh"

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
[ "$AFDATA_BASH_API_VERSION" -eq 2 ] \
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

if AFDATA_BIN="$AFDATA_BIN" AFDATA_OUTPUT=json AFDATA_OUTPUT_TO=split bash -c '
  set -euo pipefail
  source "$1"
  afdata_args_begin "demo.sh [OPTIONS]"
  afdata_args_parse --help --output plain
' bash "$ROOTPATH/bash/afdata.sh" > "$TEST_TMP/help-output.stdout" 2> "$TEST_TMP/help-output.stderr"; then
  fail "--help with --output returned success"
else
  help_output_status=$?
fi
[ "$help_output_status" -eq 2 ] || fail "--help with --output did not return status 2"
[ ! -s "$TEST_TMP/help-output.stdout" ] || fail "--help with --output wrote raw help to stdout"
[ "$(afdata_cli value --input-format json "$TEST_TMP/help-output.stderr" error.code)" = cli_error ] \
  || fail "--help with --output did not emit a structured error"

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

# Parser failures identify argument names and categories without copying raw
# values into the structured error message.
for leak_case in output short positional; do
  case "$leak_case" in
    output) leak_value="output-canary-secret" ;;
    short) leak_value="-short-canary-secret" ;;
    positional) leak_value="positional-canary-secret" ;;
  esac
  if AFDATA_BIN="$AFDATA_BIN" AFDATA_OUTPUT=json AFDATA_OUTPUT_TO=split bash -c '
    set -euo pipefail
    source "$1"
    afdata_args_begin "demo.sh [OPTIONS]"
    case "$2" in
      output) afdata_args_parse --output "$3" ;;
      short|positional) afdata_args_parse "$3" ;;
    esac
  ' bash "$ROOTPATH/bash/afdata.sh" "$leak_case" "$leak_value" \
    > "$TEST_TMP/leak-$leak_case.stdout" 2> "$TEST_TMP/leak-$leak_case.stderr"; then
    fail "$leak_case raw-value case returned success"
  else
    leak_status=$?
  fi
  [ "$leak_status" -eq 2 ] || fail "$leak_case raw-value case did not exit 2"
  if grep -q 'canary-secret' "$TEST_TMP/leak-$leak_case.stderr"; then
    fail "$leak_case raw value entered the structured error"
  fi
done

# A later parser failure must not trust output selectors from the same invalid
# argv: the diagnostic stays strict JSON on stderr.
if AFDATA_BIN="$AFDATA_BIN" AFDATA_OUTPUT=json AFDATA_OUTPUT_TO=split bash -c '
  set -euo pipefail
  source "$1"
  afdata_args_begin "demo.sh [OPTIONS]"
  afdata_args_parse --output yaml --output-to stdout positional-canary-secret
' bash "$ROOTPATH/bash/afdata.sh" \
  > "$TEST_TMP/unresolved-output.stdout" 2> "$TEST_TMP/unresolved-output.stderr"; then
  fail "unresolved output selector case returned success"
else
  unresolved_output_status=$?
fi
[ "$unresolved_output_status" -eq 2 ] || fail "unresolved output selector case did not exit 2"
[ ! -s "$TEST_TMP/unresolved-output.stdout" ] \
  || fail "unresolved output selector redirected its own parser error"
[ "$(afdata_cli value --input-format json "$TEST_TMP/unresolved-output.stderr" error.code)" = cli_error ] \
  || fail "unresolved output selector reformatted its own parser error"
if grep -q 'positional-canary-secret' "$TEST_TMP/unresolved-output.stderr"; then
  fail "unresolved output selector error copied the raw positional value"
fi

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

# ═══════════════════════════════════════════
# afdata guard: destructive-operand validation
# ═══════════════════════════════════════════

# The spelling `afdata guard` prints for a resolved path on this platform: a
# POSIX absolute path everywhere, and the native drive form under a
# Windows-native Bash, where the guard answers in the form the platform's own
# verbs and APIs accept. Expectations are built through this rather than being
# written as POSIX strings, which is what let the whole guard section below go
# unrun on Windows.
guard_form() {
  case "${OSTYPE:-}" in
    msys | cygwin | win32) cygpath -w "$1" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

# Inline form (the baseline every other form is judged against): a real,
# existing directory outside every reject-set entry passes straight through
# as its normalized absolute path.
guard_plain="$TEST_TMP/guard-plain"
mkdir -p "$guard_plain"
guard_plain_canonical="$(guard_form "$(cd "$guard_plain" && pwd -P)")"
[ "$(afdata_cli guard path "$guard_plain")" = "$guard_plain_canonical" ] \
  || fail "afdata guard path did not print the normalized target"

# Two-step form: the guard's own exit status and stderr are not swallowed
# inside a command substitution used as an argument (design's recommended
# form, over the inline baseline, for exactly this reason).
if guard_empty_out="$(afdata_cli guard path "" 2>"$TEST_TMP/guard-empty.stderr")"; then
  fail "afdata guard accepted an empty VALUE"
else
  guard_empty_status=$?
fi
[ "$guard_empty_status" -eq 1 ] || fail "afdata guard empty VALUE did not exit 1"
[ -z "$guard_empty_out" ] || fail "afdata guard empty VALUE wrote to stdout"
[ "$(afdata_cli value --input-format json "$TEST_TMP/guard-empty.stderr" error.code)" = guard_empty_value ] \
  || fail "afdata guard empty VALUE used the wrong error code"

# Nested composition: a value read out of a document, guarded before it ever
# reaches a destructive verb — the pattern the design recommends instead of
# adding a `--guard` flag to `value` itself.
guard_nested_root="$TEST_TMP/guard-nested"
mkdir -p "$guard_nested_root/output"
cat > "$guard_nested_root/build.json" <<'JSON'
{"output_dir": "output"}
JSON
guard_nested_result="$(
  cd "$guard_nested_root" \
    && afdata_cli guard cwd_path "$(afdata_cli value build.json output_dir)"
)"
guard_nested_expected="$(guard_form "$(cd "$guard_nested_root/output" && pwd -P)")"
[ "$guard_nested_result" = "$guard_nested_expected" ] \
  || fail "nested afdata value | afdata guard cwd_path did not resolve the documented value"

# Unquoted inline call: the guard rejecting collapses to zero shell words, so
# a downstream verb given no operand at all cannot act on any guessed or
# leftover value. BSD `rm -f` (macOS) silently no-ops on zero operands rather
# than erroring the way GNU `rm` does — a real, prototype-stage difference
# from the design's assumed "missing operand" diagnostic — so this only
# asserts the safety property every platform actually guarantees: nothing
# named by the sentinel is ever touched.
unquoted_guard_dir="$TEST_TMP/guard-unquoted"
mkdir -p "$unquoted_guard_dir"
touch "$unquoted_guard_dir/keep-me"
# The unquoted command substitution below is the exact case under test, so the
# call-position lint must leave it alone rather than the test being rewritten to
# satisfy it — this line exists to prove the shape stays harmless.
# shellcheck disable=SC2046,SC2086
rm -rf -- $(afdata_cli guard path "" 2>/dev/null) || true # afdata-lint-allow
[ -e "$unquoted_guard_dir/keep-me" ] \
  || fail "an unquoted rejected guard call still let something be removed"

# Regression probe (design's own recovery test): pointing a destructive
# verb's operand at the filesystem root, $HOME, the current directory, or a
# real directory outside a containment root must never touch that target.
guard_regression_case() {
  local probe_type="$1"
  local probe_value="$2"
  local probe_dir="$3"
  local probe_sentinel="$probe_dir/regression-sentinel"
  local probe_out
  mkdir -p "$probe_dir"
  touch "$probe_sentinel"
  probe_out="$(afdata_cli guard "$probe_type" "$probe_value" 2>/dev/null)" || true
  [ -z "$probe_out" ] \
    || fail "afdata guard $probe_type '$probe_value' should have rejected but printed output"
  rm -rf -- "$probe_out"
  [ -e "$probe_sentinel" ] \
    || fail "afdata guard $probe_type '$probe_value' let a downstream rm -rf reach $probe_dir"
}

guard_blank_out="$(afdata_cli guard path "   " 2>/dev/null)" || true
[ -z "$guard_blank_out" ] || fail "afdata guard accepted an all-whitespace VALUE"

# `--under`: an anchor the TYPE vocabulary cannot name on its own. This is the
# shape scripts/test.sh uses for its own build artifacts, where the anchor is
# the checkout root rather than the temp area or the current directory.
guard_anchor="$TEST_TMP/guard-anchor"
mkdir -p "$guard_anchor/build"
guard_anchor_child="$(guard_form "$(cd "$guard_anchor/build" && pwd -P)")"
[ "$(afdata_cli guard path "$guard_anchor/build" --under "$guard_anchor")" = "$guard_anchor_child" ] \
  || fail "afdata guard --under did not accept a child of the anchor"

guard_outside_anchor="$TEST_TMP/guard-outside-anchor"
mkdir -p "$guard_outside_anchor"
guard_under_out="$(afdata_cli guard path "$guard_outside_anchor" --under "$guard_anchor" 2>/dev/null)" || true
[ -z "$guard_under_out" ] || fail "afdata guard --under accepted a target outside the anchor"

# An unset shell variable reaching --under must not silently degrade to "under
# the current directory".
guard_blank_root_out="$(afdata_cli guard path "$guard_anchor/build" --under "" \
  2>"$TEST_TMP/guard-blank-root.stderr")" || true
[ -z "$guard_blank_root_out" ] || fail "afdata guard accepted a blank --under root"
[ "$(afdata_cli value --input-format json "$TEST_TMP/guard-blank-root.stderr" error.code)" = guard_invalid_root ] \
  || fail "a blank --under root used the wrong error code"

# A symlink sitting inside the containment root but pointing out of it: `rm`
# would only unlink it, but `>`, `chmod` and `cp` follow it, so containment is
# decided on where it points, not on where its name lives.
ln -s "$guard_outside_anchor" "$guard_anchor/escape_link"
[ -L "$guard_anchor/escape_link" ] \
  || fail "this platform did not create a real symlink, so the link cases below would test a copy"
guard_escape_out="$(afdata_cli guard path "$guard_anchor/escape_link" --under "$guard_anchor" \
  2>"$TEST_TMP/guard-escape.stderr")" || true
[ -z "$guard_escape_out" ] || fail "afdata guard accepted a symlink escaping the containment root"
[ "$(afdata_cli value --input-format json "$TEST_TMP/guard-escape.stderr" error.code)" = guard_symlink_escapes_containment ] \
  || fail "a containment-escaping symlink used the wrong error code"

# A symlink that stays inside the root still passes, and still passes as the
# link itself rather than as its target.
ln -s "$guard_anchor/build" "$guard_anchor/inside_link"
[ "$(afdata_cli guard path "$guard_anchor/inside_link" --under "$guard_anchor")" = "$(guard_form "$(cd "$guard_anchor" && pwd -P)/inside_link")" ] \
  || fail "afdata guard did not pass a contained symlink through as itself"

# `set -e` contrast, both recommended forms. These run in real child shells:
# errexit is suppressed inside a subshell used as an `if`/`||` condition, so
# the same check written inline here would pass without testing anything.
guard_sete_dir="$TEST_TMP/guard-set-e"
mkdir -p "$guard_sete_dir"
touch "$guard_sete_dir/keep-me"

# Two-step: the rejection stops the script right at the assignment, before the
# verb line is ever reached.
guard_sete_status=0
# The single-quoted program is intentionally expanded by the child Bash.
# shellcheck disable=SC2016
bash -c '
  set -euo pipefail
  target="$("$1" guard path "" 2>/dev/null)"
  rm -rf -- "$2/keep-me"
  printf "%s" "$target"
' bash "$AFDATA_BIN" "$guard_sete_dir" >/dev/null 2>&1 || guard_sete_status=$?
[ "$guard_sete_status" -ne 0 ] || fail "the two-step form did not stop the script at a rejected guard"
[ -e "$guard_sete_dir/keep-me" ] || fail "a rejected two-step guard still let the next command run"

# Inline, without `set -e`: the script keeps going (and on BSD `rm -f` even
# exits 0 on an empty operand), so the property that holds on every platform
# is not "the verb errors" but "the verb was never given a target".
# shellcheck disable=SC2016
bash -c '
  set +e
  rm -rf -- "$("$1" guard path "" 2>/dev/null)"
  exit 0
' bash "$AFDATA_BIN"
[ -e "$guard_sete_dir/keep-me" ] || fail "an inline rejected guard removed something without set -e"

guard_regression_root="$TEST_TMP/guard-regression"
# "The filesystem root" is not the same string everywhere. Under a
# Windows-native Bash `/` is the MSYS installation directory — an ordinary
# folder the guard is right to accept — and the real root is the drive the
# tests are running on. Passing `/` there would assert nothing about the reject
# set.
case "${OSTYPE:-}" in
  msys | cygwin | win32)
    guard_root_value="$(cygpath -w "$TEST_TMP")"
    guard_root_value="${guard_root_value:0:3}"
    ;;
  *) guard_root_value=/ ;;
esac
guard_regression_case path "$guard_root_value" "$guard_regression_root/root-case"
if [ -n "${HOME:-}" ]; then
  guard_regression_case path "$HOME" "$guard_regression_root/home-case"
fi
guard_regression_case tmp_path "$ROOTPATH" "$guard_regression_root/outside-tmp-case"

mkdir -p "$guard_regression_root/cwd-case"
touch "$guard_regression_root/cwd-case/regression-sentinel"
guard_cwd_out="$(cd "$guard_regression_root/cwd-case" && afdata_cli guard cwd_path . 2>/dev/null)" || true
[ -z "$guard_cwd_out" ] || fail "afdata guard cwd_path . on the current directory should have rejected"
[ -e "$guard_regression_root/cwd-case/regression-sentinel" ] \
  || fail "a cwd_path . rejection still let something reach the current directory"

# Git Bash on Windows auto-translates a POSIX-looking absolute argument
# (`/c/...`, `/tmp/...`) into native Windows form before a non-MSYS .exe ever
# sees argv — MSYSTEM is set only there, so this assertion is both specific
# and confined to the one platform where it means anything; elsewhere there
# is nothing MSYS-shaped to check, and this is not a silent pass on a
# negative assertion; it is simply out of scope.
if [ -n "${MSYSTEM:-}" ]; then
  msys_probe_dir="$TEST_TMP/guard-msys-probe"
  mkdir -p "$msys_probe_dir"
  msys_guard_out="$(afdata_cli guard path "$msys_probe_dir")"
  case "$msys_guard_out" in
    [A-Za-z]:\\*) : ;;
    *) fail "afdata guard on Git Bash did not print a native Windows path: $msys_guard_out" ;;
  esac
fi

# afdata_remove
#
# Every rejection below aims at a directory this test created inside TEST_TMP
# and guards it against a containment root it does not sit under. The refusal is
# therefore structural, and a hypothetical guard defect could only reach a
# directory the EXIT trap removes anyway — no probe here names $HOME, the
# checkout, or anything else whose loss would matter.
remove_ok_dir="$TEST_TMP/remove-ok"
mkdir -p "$remove_ok_dir/nested"
printf 'x\n' > "$remove_ok_dir/nested/file"
afdata_remove tmp_path "$remove_ok_dir"
[ ! -e "$remove_ok_dir" ] || fail "afdata_remove did not remove an approved directory"
# Idempotent like `rm -rf`, so a repeated cleanup cannot kill a `set -e` script.
afdata_remove tmp_path "$remove_ok_dir" \
  || fail "afdata_remove rejected a path that no longer exists"

remove_sentinel="$TEST_TMP/remove-sentinel"
remove_other_root="$TEST_TMP/remove-other-root"
mkdir -p "$remove_sentinel" "$remove_other_root"
printf 'intact\n' > "$remove_sentinel/marker"

remove_status=0
afdata_remove path "$remove_sentinel" --under "$remove_other_root" 2>/dev/null \
  || remove_status=$?
[ "$remove_status" -ne 0 ] || fail "afdata_remove reported success on a rejected value"
[ -e "$remove_sentinel/marker" ] || fail "a rejected afdata_remove still reached its target"
[ "$(cat "$remove_sentinel/marker")" = intact ] \
  || fail "a rejected afdata_remove modified its target"

remove_status=0
afdata_remove tmp_path 2>/dev/null || remove_status=$?
[ "$remove_status" -eq 2 ] || fail "afdata_remove accepted a call with no VALUE"

# The contrast that justifies this function existing at all, asserted from real
# child processes: errexit is suppressed inside the subshell an `if` condition
# creates, so testing it here directly would pass while verifying nothing.
cat > "$TEST_TMP/remove-errexit.sh" <<'PROBE'
set -euo pipefail
# shellcheck disable=SC1090
source "$1"
case "$4" in
  function) afdata_remove path "$2" --under "$3" ;;
  inline) rm -rf -- "$(afdata_cli guard path "$2" --under "$3")" ;; # afdata-lint-allow
esac
printf 'REACHED-PAST-CLEANUP\n'
PROBE

for remove_form in function inline; do
  remove_probe_out=""
  remove_status=0
  remove_probe_out="$(AFDATA_BIN="$AFDATA_BIN" "$legacy_bash" \
    "$TEST_TMP/remove-errexit.sh" "$ROOTPATH/bash/afdata.sh" \
    "$remove_sentinel" "$remove_other_root" "$remove_form" 2>/dev/null)" \
    || remove_status=$?
  case "$remove_form" in
    function)
      [ "$remove_status" -ne 0 ] \
        || fail "afdata_remove let a set -e script survive a guard rejection"
      [ -z "$remove_probe_out" ] \
        || fail "afdata_remove let a set -e script run past a guard rejection"
      ;;
    inline)
      # Pinned as the defect this function exists to remove, not as behaviour
      # anyone wants: in an argument position the guard's status is discarded,
      # and `rm -f` is defined to exit 0 on an operand naming nothing. Should
      # this assertion ever fail, the premise changed — revisit whether
      # afdata_remove is still needed rather than "fixing" the test.
      [ "$remove_probe_out" = REACHED-PAST-CLEANUP ] \
        || fail "the inline spelling no longer runs on past a rejection; afdata_remove's premise needs rechecking"
      ;;
  esac
  [ -e "$remove_sentinel/marker" ] \
    || fail "the $remove_form probe reached its target despite a rejection"
done

# afdata_trash
#
# Nothing here reaches a real trash. The entry point is stubbed on PATH, so what
# is under test is this kit's dispatch and its refusals — whether a desktop's
# trash can works is that desktop's contract, not this one's, and a suite that
# filled the tester's trash to find out would be its own defect.
#
# Which program gets stubbed follows the platform, because the entry point does:
# Windows ships no trash command, so the kit drives PowerShell there and hands
# the operand over in the environment rather than in argv. Stubbing whatever
# this host would really call keeps every assertion below about dispatch, on
# both shapes, instead of pinning one platform's argv.
trash_stub_dir="$TEST_TMP/trash-stub"
mkdir -p "$trash_stub_dir"
case "${OSTYPE:-}" in
  msys | cygwin | win32)
    trash_stub_name=powershell
    cat > "$trash_stub_dir/powershell" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$AFDATA_TRASH_TARGET" >> "$AFDATA_TRASH_STUB_LOG"
# The real entry point prints this as its last act, and the kit does not count a
# run without it as a success.
printf 'AFDATA-TRASH-OK'
STUB
    ;;
  *)
    trash_stub_name=trash
    cat > "$trash_stub_dir/trash" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$@" >> "$AFDATA_TRASH_STUB_LOG"
STUB
    ;;
esac
chmod +x "$trash_stub_dir/$trash_stub_name"
AFDATA_TRASH_STUB_LOG="$TEST_TMP/trash-stub.log"
export AFDATA_TRASH_STUB_LOG
: > "$AFDATA_TRASH_STUB_LOG"

trash_victim="$TEST_TMP/trash-victim"
mkdir -p "$trash_victim"
printf 'intact\n' > "$trash_victim/marker"

PATH="$trash_stub_dir:$PATH" afdata_trash tmp_path "$trash_victim"
# The operand is the last thing logged under either shape. It is checked by
# looking through it at a file only the victim holds, rather than by comparing
# text: on Windows the guard hands over a native `C:\…` path, which is the right
# operand spelled in a way no POSIX string built here would match.
trash_logged_operand="$(tail -n 1 "$AFDATA_TRASH_STUB_LOG")"
[ -n "$trash_logged_operand" ] && [ -e "$trash_logged_operand/marker" ] \
  || fail "afdata_trash did not hand the guarded path to the trash entry point"
if [ "$trash_stub_name" = trash ]; then
  grep -qxF -- '--' "$AFDATA_TRASH_STUB_LOG" \
    || fail "afdata_trash did not pass -- before the operand"
fi
# The stub does not delete, so the directory is still here — which is what lets
# the refusal checks below distinguish "was not trashed" from "was never there".
[ -e "$trash_victim/marker" ] || fail "the trash stub was expected to leave its operand alone"

# A rejected value never reaches the entry point.
: > "$AFDATA_TRASH_STUB_LOG"
trash_other_root="$TEST_TMP/trash-other-root"
mkdir -p "$trash_other_root"
remove_status=0
PATH="$trash_stub_dir:$PATH" \
  afdata_trash path "$trash_victim" --under "$trash_other_root" 2>/dev/null \
  || remove_status=$?
[ "$remove_status" -ne 0 ] || fail "afdata_trash reported success on a rejected value"
[ ! -s "$AFDATA_TRASH_STUB_LOG" ] || fail "a rejected afdata_trash still invoked the trash"
[ -e "$trash_victim/marker" ] || fail "a rejected afdata_trash still reached its target"

# Idempotent like afdata_remove, and without invoking anything.
: > "$AFDATA_TRASH_STUB_LOG"
PATH="$trash_stub_dir:$PATH" afdata_trash tmp_path "$TEST_TMP/trash-absent" \
  || fail "afdata_trash rejected a path that does not exist"
[ ! -s "$AFDATA_TRASH_STUB_LOG" ] || fail "afdata_trash invoked the trash for an absent path"

# With no trash on the host it must refuse, and above all must not fall back to
# deleting: a reversible verb that quietly becomes an irreversible one is the
# precise failure this kit exists to prevent. $AFDATA_BIN is absolute, so the
# empty PATH still leaves afdata reachable.
: > "$AFDATA_TRASH_STUB_LOG"
trash_empty_path="$TEST_TMP/trash-empty-path"
mkdir -p "$trash_empty_path"
remove_status=0
trash_unavailable_stderr="$(PATH="$trash_empty_path" \
  afdata_trash tmp_path "$trash_victim" 2>&1 >/dev/null)" || remove_status=$?
[ "$remove_status" -ne 0 ] || fail "afdata_trash succeeded with no trash available"
case "$trash_unavailable_stderr" in
  *trash_unavailable*) : ;;
  *) fail "afdata_trash did not report trash_unavailable: $trash_unavailable_stderr" ;;
esac
[ -e "$trash_victim/marker" ] \
  || fail "afdata_trash fell back to deleting when no trash was available"

remove_status=0
afdata_trash tmp_path 2>/dev/null || remove_status=$?
[ "$remove_status" -eq 2 ] || fail "afdata_trash accepted a call with no VALUE"

# The Windows entry point answers in exit codes rather than by being a command
# that either worked or did not, so the mapping from those codes to AFDATA
# errors is a surface of its own and is pinned here.
#
# It is exercised on every platform rather than only on Windows: the branch is
# chosen by $OSTYPE, so setting that and putting stub entry points on PATH runs
# the same dispatch anywhere, and a mapping that only a Windows runner ever
# checks is a mapping nobody checks while writing it. What genuinely needs
# Windows — a native path surviving `[ -e ]`, a real cygpath, a real Recycle
# Bin — is not what these cases claim to cover.
trash_saved_ostype="${OSTYPE:-}"
OSTYPE=msys
cat > "$trash_stub_dir/cygpath" <<'STUB'
#!/usr/bin/env bash
# `-w PATH` is the only form the kit uses, and off Windows there is nothing to
# convert, so the value comes straight back.
shift
printf '%s\n' "$1"
STUB
chmod +x "$trash_stub_dir/cygpath"

trash_case() {
  # $1 exit code, $2 stdout, $3 expected AFDATA error code ('' when the call
  # must succeed). The stub stands in for the entry point wholesale, so what is
  # under test is this kit's reading of the answer, not PowerShell's.
  cat > "$trash_stub_dir/powershell" <<STUB
#!/usr/bin/env bash
printf '%s' '$2'
exit $1
STUB
  chmod +x "$trash_stub_dir/powershell"
  mkdir -p "$trash_victim"
  printf 'intact\n' > "$trash_victim/marker"
  trash_case_status=0
  trash_case_stderr="$(PATH="$trash_stub_dir:$PATH" \
    afdata_trash tmp_path "$trash_victim" 2>&1 >/dev/null)" || trash_case_status=$?
  [ -e "$trash_victim/marker" ] \
    || fail "the entry point stub was expected to leave its operand alone"
  if [ -z "$3" ]; then
    [ "$trash_case_status" -eq 0 ] \
      || fail "afdata_trash rejected an entry point that reported success"
    return 0
  fi
  [ "$trash_case_status" -ne 0 ] \
    || fail "afdata_trash reported success for entry point exit $1"
  case "$trash_case_stderr" in
    *"$3"*) : ;;
    *) fail "entry point exit $1 did not become $3: $trash_case_stderr" ;;
  esac
}

trash_case 0 'AFDATA-TRASH-OK' ''
# The defect the token exists for, measured rather than imagined: PowerShell
# handed a script it could not parse to the end ran the part it had, discarded
# the rest and exited 0. Success is therefore never read from the exit code
# alone.
trash_case 0 '' trash_incomplete
trash_case 10 'the Recycle Bin is switched off for this volume' trash_unavailable
# Gone without reaching the bin is not interchangeable with "nothing was
# touched", so it must not arrive wearing the code that promises the latter.
trash_case 12 'deleted, but nothing arrived in the Recycle Bin' trash_not_reversible

# The reason the entry point gives has to survive into the error, because on
# Windows the answer turns on which volume the target sits on rather than on
# which host it is.
case "$trash_case_stderr" in
  *"nothing arrived in the Recycle Bin"*) : ;;
  *) fail "afdata_trash dropped the entry point's reason: $trash_case_stderr" ;;
esac

OSTYPE="$trash_saved_ostype"
rm -f "$trash_stub_dir/cygpath" "$trash_stub_dir/powershell"

printf 'Bash authoring kit checks passed.\n'
