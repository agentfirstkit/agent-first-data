#!/bin/bash
# Prove the ShellCheck gate never inherits an indefinitely open stdin.
#
# `shellcheck -x` follows a `source=/dev/stdin` directive by opening that path.
# This repo's scripts use exactly that directive to load the Bash kit that
# `afdata shell bash` writes at runtime, so an analyzer handed a stdin that
# never reaches EOF waits on it forever — silently, with no output and no
# timeout. CI happens to hand the gate a closed stdin, so the failure only
# appears on a developer's terminal, where it reads as "the tests hang".
#
# The check runs the gate against `scripts/fixtures/shellcheck-open-stdin.sh`
# with a pipe held open and requires it to finish within a deadline.
#
# Usage: validate_shellcheck_stdin.sh
set -euo pipefail

ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT_DIR="$ROOTPATH/scripts"

# Prefer the binary this tree builds over whatever is installed.
if [ -z "${AFDATA_BIN:-}" ]; then
  if [ -x "$ROOTPATH/target/debug/afdata" ]; then
    AFDATA_BIN="$ROOTPATH/target/debug/afdata"
  else
    # Falling back to an installed afdata would check an older tree than the
    # one being edited, and pass or fail for the wrong reason.
    printf '%s\n' "$(basename "$0"): no AFDATA_BIN and no target/debug/afdata; run scripts/test.sh, or build with: cargo build --bin afdata" >&2
    exit 1
  fi
fi
export AFDATA_BIN

_AFDATA_BASH_SOURCE="$("$AFDATA_BIN" shell bash)"
# shellcheck source=/dev/stdin
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE

afdata_args_begin "validate_shellcheck_stdin.sh [OPTIONS]"
afdata_args_parse "$@"

INSTALLER="$SCRIPT_DIR/install-shellcheck.sh"
FIXTURE_DIR="$SCRIPT_DIR/fixtures"
DEADLINE_S=20

if [ ! -f "$INSTALLER" ]; then
  afdata_error shellcheck_installer_missing "missing $INSTALLER" || exit 1
fi
if [ ! -f "$FIXTURE_DIR/shellcheck-open-stdin.sh" ]; then
  afdata_error shellcheck_fixture_missing "missing the open-stdin fixture under $FIXTURE_DIR" || exit 1
fi

WORK="$(mktemp -d)"
release_holder() {
  if [ -s "$WORK/holder.pid" ]; then
    kill "$(cat "$WORK/holder.pid")" 2>/dev/null || :
  fi
}
cleanup() {
  release_holder
  afdata_remove tmp_path "$WORK" || :
}
trap cleanup EXIT

# Name the interpreter rather than relying on the shebang: Windows resolves a
# program through `CreateProcess`, which has no idea what a `#!` line is.
#
# The gate's stdin is a pipe whose writer, a `sleep`, writes nothing and never
# closes it: an interactive terminal's shape, not a redirect the gate could
# accidentally satisfy. The sleep outlives the deadline, so its pid is kept and
# it is killed once the verdict is in.
{
  sleep $((DEADLINE_S + 30)) &
  echo $! >"$WORK/holder.pid"
  wait
} 2>/dev/null | bash "$INSTALLER" --check "$FIXTURE_DIR" >"$WORK/out" 2>"$WORK/err" &
gate=$!

waited=0
while kill -0 "$gate" 2>/dev/null && [ "$waited" -lt "$((DEADLINE_S * 10))" ]; do
  sleep 0.1
  waited=$((waited + 1))
done

if kill -0 "$gate" 2>/dev/null; then
  kill "$gate" 2>/dev/null || :
  release_holder
  wait "$gate" 2>/dev/null || :
  afdata_error shellcheck_stdin_inherited \
    "the ShellCheck gate inherited an open stdin and did not finish within ${DEADLINE_S}s" || exit 1
fi

# `wait` on a pipeline's pid waits for the whole pipeline, sleep included, so
# the holder goes first; the gate itself has already finished.
release_holder
status=0
wait "$gate" || status=$?

if [ "$status" -ne 0 ]; then
  cat "$WORK/out" "$WORK/err" >&2
  afdata_error shellcheck_gate_failed "the ShellCheck gate exited $status on its own fixture" || exit 1
fi

# Finishing proves nothing where no analyzer ran: the deadline is about
# ShellCheck's own stdin, and on a platform upstream publishes no build for
# there is none to hold open. Say that, rather than report a pass earned by the
# gate having had nothing to do.
if grep -q "ShellCheck skipped" "$WORK/out"; then
  afdata_log warn "no pinned ShellCheck on this platform, so the gate has no stdin to inherit; the Linux and macOS runs carry this check"
fi
afdata_result "ShellCheck gate finishes with an open stdin"
