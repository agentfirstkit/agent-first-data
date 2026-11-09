#!/bin/bash
# Keep `cli-spec-v1.schema.json` in step with the Rust types it describes.
#
# `cli-spec-v1` is published as a machine-readable contract, and every object in
# it is `additionalProperties: false`. That makes the schema strict in one
# direction only: adding a field to the Rust structs does not fail any build, it
# just quietly produces documents that no longer validate against their own
# published schema. That is precisely how `display_name`, `build`, and
# `sensitive` drifted out of sync.
#
# So this gate compares, field by field, the serde field names of the four
# serializable spec types against the schema locations that describe them, in
# both directions. It is deliberately structural rather than a JSON Schema
# validation run: it needs no serialized sample and no schema library, and it
# fails the moment a field exists on one side and not the other.
#
# Usage: validate_cli_spec_schema.sh
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

afdata_args_begin "validate_cli_spec_schema.sh [OPTIONS]"
afdata_args_parse "$@"

SPEC_RS="$ROOTPATH/rust/src/cli_spec/spec.rs"
SCHEMA="$ROOTPATH/spec/cli-spec-v1.schema.json"

# Rust struct, then the dot path of the schema object describing its fields.
STRUCTS=(CliSpec CommandSpec ArgSpec Combination)
# `$defs` is a literal JSON Schema key, not a shell expansion.
# shellcheck disable=SC2016
SCHEMA_PATHS=(
  'properties'
  '$defs.command.properties'
  '$defs.argument.properties'
  '$defs.combination.properties'
)

# The `pub name:` fields of one struct, one per line. The body runs from the
# struct's opening line to the first line that starts with `}`.
rust_fields() {
  awk -v header="pub struct $1 {" '
    $0 == header { inside = 1; next }
    inside && /^}/ { exit }
    inside && match($0, /^[ \t]*pub [a-z_][a-z0-9_]*[ \t]*:/) {
      field = substr($0, RSTART, RLENGTH)
      sub(/^[ \t]*pub /, "", field)
      sub(/[ \t]*:$/, "", field)
      print field
    }
  ' "$SPEC_RS"
}

# Whether $1 is one of the remaining arguments.
in_array() {
  local needle="$1" item
  shift
  for item in "$@"; do
    [ "$item" != "$needle" ] || return 0
  done
  return 1
}

declared=() described=()
FAILURES=""
TOTAL=0
index=0
while [ "$index" -lt "${#STRUCTS[@]}" ]; do
  struct="${STRUCTS[$index]}"
  path="${SCHEMA_PATHS[$index]}"
  index=$((index + 1))

  if ! grep -q "^pub struct $struct {" "$SPEC_RS"; then
    afdata_error cli_spec_struct_missing "$SPEC_RS: cannot find \`pub struct $struct\`" || exit 1
  fi
  declared=()
  while IFS= read -r name; do
    declared+=("$name")
  done < <(rust_fields "$struct")
  TOTAL=$((TOTAL + ${#declared[@]}))

  if ! afdata_into described keys "$SCHEMA" "$path" 2>/dev/null; then
    FAILURES="$FAILURES  - schema is missing $path for \`$struct\`"$'\n'
    continue
  fi

  where="${path//./\/}"
  for name in ${declared[@]+"${declared[@]}"}; do
    if ! in_array "$name" ${described[@]+"${described[@]}"}; then
      FAILURES="$FAILURES  - \`$struct.$name\` has no \`$where.$name\` in cli-spec-v1: a spec carrying it fails its own schema (additionalProperties is false)"$'\n'
    fi
  done
  for name in ${described[@]+"${described[@]}"}; do
    if ! in_array "$name" ${declared[@]+"${declared[@]}"}; then
      FAILURES="$FAILURES  - cli-spec-v1 describes \`$where.$name\`, but \`$struct\` has no such field"$'\n'
    fi
  done
done

if [ -n "$FAILURES" ]; then
  afdata_error cli_spec_schema_drift "cli-spec-v1 schema drift:"$'\n'"${FAILURES%$'\n'}" || exit 1
fi

afdata_result "cli-spec-v1 schema ok: $TOTAL fields across ${#STRUCTS[@]} types"
