#!/bin/bash
# Enforce the dependency direction between the CLI core and AFDATA.
#
# `rust/src/cli_spec/` is the closed-world CLI compiler: it decides whether an
# invocation is legal and what it resolved to. `rust/src/cli_afdata.rs` is the
# only place that expresses such an outcome as an AFDATA protocol event.
#
# Keeping that direction one-way is what makes the core extractable into its own
# crate later without a redesign, so it is checked rather than trusted:
#
# - nothing under `cli_spec/` may reference `crate::` — no protocol events, no
#   emitter, no rendering, no redaction — nor escape upwards via
#   `super::super::`, which resolves to the same crate root;
# - nothing under `cli_spec/` may name `serde_json::Value` in its API. AFDATA
#   turns on `serde_json/arbitrary_precision`, and cargo unifies features across
#   a whole binary, so a parsed value's number semantics would depend on what
#   else happens to be linked. Raw JSON argument values stay source text;
# - nothing under `cli_spec/` may touch process I/O — resolving argv must not
#   print, read files, or install sinks;
# - nothing under `cli_spec/` may hardcode AFDATA's `_secret` naming convention;
#   the core carries an opaque `sensitive` bit and the adapter owns the policy.
#
# The complementary direction is checked by the build itself: `scripts/test.sh
# static` builds `--no-default-features`, which links neither the compiler nor
# its adapter and so only succeeds while the rest of the crate depends on
# neither.
#
# Usage: validate_cli_core_boundary.sh
set -euo pipefail

ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

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

afdata_args_begin "validate_cli_core_boundary.sh [OPTIONS]"
afdata_args_parse "$@"

CORE_REL="rust/src/cli_spec"
ADAPTER_REL="rust/src/cli_afdata.rs"
CORE="$ROOTPATH/$CORE_REL"
ADAPTER="$ROOTPATH/$ADAPTER_REL"

if [ ! -d "$CORE" ]; then
  afdata_error cli_core_missing "missing CLI core directory: $CORE_REL" || exit 1
fi
if [ ! -f "$ADAPTER" ]; then
  afdata_error cli_adapter_missing "missing AFDATA adapter: $ADAPTER_REL" || exit 1
fi

# ERE, why it is banned in the core. A word boundary is spelled out because
# BSD grep has no \b.
W_BEFORE='(^|[^A-Za-z0-9_])'
W_AFTER='($|[^A-Za-z0-9_])'
BANNED_PATTERNS=(
  "${W_BEFORE}crate::"
  "${W_BEFORE}super::super::"
  "${W_BEFORE}serde_json::Value${W_AFTER}"
  "${W_BEFORE}std::io${W_AFTER}"
  "${W_BEFORE}std::fs${W_AFTER}"
  "${W_BEFORE}std::process${W_AFTER}"
  "${W_BEFORE}(e)?print(ln)?!"
  "_secret"
)
BANNED_WHY=(
  "references another AFDATA module"
  "escapes to the crate root, bypassing the crate:: ban"
  "names serde_json::Value (feature-unification hazard)"
  "performs process I/O"
  "touches the filesystem"
  "touches the process"
  "writes to a standard stream"
  "hardcodes AFDATA's \`_secret\` naming convention"
)

FAILURES=""
CHECKED=0
while IFS= read -r path; do
  CHECKED=$((CHECKED + 1))
  rel="${path#"$ROOTPATH"/}"
  # Source lines with comments stripped, so prose may discuss the rules: a line
  # that starts with `//` is dropped, and a trailing `//` comment is cut off.
  # Line numbers are kept by emitting `N:code` for every line; the `:` after N
  # is a non-word character, so a rule anchored at the start of code still
  # matches. One grep per rule over the whole file, not one per line: a process
  # per line takes minutes where starting a process is expensive.
  stripped="$(awk '
    { stripped = $0; sub(/^[ \t\r]+/, "", stripped) }
    stripped ~ /^\/\// { next }
    { code = $0; i = index(code, "//"); if (i > 0) code = substr(code, 1, i - 1); print NR ":" code }
  ' "$path")"
  index=0
  while [ "$index" -lt "${#BANNED_PATTERNS[@]}" ]; do
    while IFS= read -r numbered; do
      number="${numbered%%:*}"
      line="${numbered#*:}"
      trimmed="$(printf '%s' "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
      FAILURES="$FAILURES$rel:$number: ${BANNED_WHY[$index]}"$'\n'"    $trimmed"$'\n'
    done < <(printf '%s\n' "$stripped" | grep -E -- "${BANNED_PATTERNS[$index]}" || :)
    index=$((index + 1))
  done
done < <(find "$CORE" -name '*.rs' -type f | LC_ALL=C sort)

if ! grep -q 'crate::cli_spec' "$ADAPTER" || ! grep -q 'crate::protocol' "$ADAPTER"; then
  FAILURES="$FAILURES$ADAPTER_REL: the adapter must bridge crate::cli_spec and crate::protocol; if it no longer does, the boundary moved"$'\n'
fi

if [ -n "$FAILURES" ]; then
  afdata_error cli_core_boundary_violation "CLI core boundary violations:"$'\n'"${FAILURES%$'\n'}" || exit 1
fi

afdata_result "CLI core boundary OK ($CHECKED files under $CORE_REL/)"
