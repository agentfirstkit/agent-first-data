#!/bin/bash
# Assert that CLI help-v2 JSON and plain render the same registered shapes.
#
# Walks every command reachable from the root through the JSON help's
# `subcommands`, and for each one requires that the plain help carries the same
# syntax lines, argument notes and defaults as the JSON help.
#
# Usage: validate_help_formats_agree.sh [PATH_TO_AFDATA]
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

# binary is assigned by afdata_args_parse, which this file gets from
# `afdata shell bash` at runtime — not from a file a static checker can read.
# shellcheck disable=SC2154
afdata_args_begin "validate_help_formats_agree.sh [OPTIONS] [PATH_TO_AFDATA]"
afdata_args_positional binary PATH_TO_AFDATA "Binary whose help is compared (default: target/debug/afdata)" optional
afdata_args_parse "$@"

if [ -z "$binary" ]; then
  binary="$ROOTPATH/target/debug/afdata"
fi
if [ ! -e "$binary" ]; then
  afdata_error binary_not_found "binary not found: $binary" || exit 1
fi

# The plain syntax lines start with the invocation itself: the tool's file name
# without its extension.
tool_name="${binary##*/}"
tool_name="${tool_name%.*}"
TOOL_PREFIX="$tool_name "

WORK="$(mktemp -d)"
cleanup() {
  afdata_remove tmp_path "$WORK" || :
}
trap cleanup EXIT

disagree() {
  afdata_error help_formats_disagree "Help formats disagree: $1" || exit 1
  exit 1
}

# Read one scalar of the saved JSON help, or the fallback when it is absent.
help_value() {
  "$AFDATA_BIN" value "$WORK/help.json" "$1" --default "${2-}" </dev/null
}

# The JSON help of one command, saved to $WORK/help.json, and its plain help,
# saved to $WORK/help.txt. $@ is the command path (empty for the root).
load_help() {
  "$binary" "$@" --help --output json </dev/null >"$WORK/help.json"
  "$binary" "$@" --help --output plain </dev/null >"$WORK/help.txt"
}

# Children of a container in the saved JSON help, into the array named $1;
# empty when the container is absent.
help_keys_into() {
  afdata_into "$1" keys "$WORK/help.json" "$2" --missing-ok </dev/null
}

PENDING=("")
shape_indexes=() note_names=() default_names=() sub_indexes=()
VISITED=" "
SHAPES_COUNT=0
VISITED_COUNT=0
while [ "${#PENDING[@]}" -gt 0 ]; do
  last=$((${#PENDING[@]} - 1))
  command_path="${PENDING[$last]}"
  unset "PENDING[$last]"
  if [[ "$VISITED" == *" [$command_path] "* ]]; then
    continue
  fi
  VISITED="${VISITED}[$command_path] "
  VISITED_COUNT=$((VISITED_COUNT + 1))

  # shellcheck disable=SC2086
  load_help $command_path

  kind="$(help_value kind)"
  code="$(help_value result.code)"
  schema="$(help_value result.help.schema)"
  model_path="$(help_value result.help.command_path)"
  [ "$kind" = result ] || disagree "help for [$command_path] is not a result event"
  [ "$code" = help ] || disagree "help for [$command_path] has code '$code'"
  [ "$schema" = cli-help-v2 ] || disagree "help for [$command_path] has schema '$schema'"

  plain="$(cat "$WORK/help.txt")"
  help_keys_into shape_indexes result.help.shapes
  shape_count=0
  expected=""
  for index in ${shape_indexes[@]+"${shape_indexes[@]}"}; do
    shape_count=$((shape_count + 1))
    expected="$expected$(help_value "result.help.shapes.$index.usage")"$'\n'
  done
  SHAPES_COUNT=$((SHAPES_COUNT + shape_count))

  # One round trip, so JSON and plain must carry the same complete syntax for
  # every shape — there is no second level to defer to. A syntax line is
  # exactly a line that starts with the invocation itself; the command's
  # description, a shape's id/description, and the subcommand pointers never do.
  actual=""
  while IFS= read -r line; do
    case "$line" in
      "$TOOL_PREFIX"*) actual="$actual$line"$'\n' ;;
    esac
  done <<<"$plain"
  if [ "$expected" != "$actual" ]; then
    disagree "help syntax differs for $model_path: json=[$expected], plain=[$actual]"
  fi

  # Every argument meaning and default the structured form carries has to reach
  # the plain reader too. Comparing only the syntax lines let plain drop
  # `notes` and `defaults` entirely while this check stayed green — the "human
  # catalog" was strictly weaker than the JSON.
  missing_notes=""
  help_keys_into note_names result.help.notes
  for name in ${note_names[@]+"${note_names[@]}"}; do
    note="$(help_value "result.help.notes.$name")"
    if [[ "$plain" != *"$name"* || "$plain" != *"$note"* ]]; then
      missing_notes="$missing_notes $name: $note;"
    fi
  done
  if [ -n "$missing_notes" ]; then
    disagree "plain help for $model_path omits argument notes the JSON carries:$missing_notes"
  fi
  missing_defaults=""
  help_keys_into default_names result.help.defaults
  for name in ${default_names[@]+"${default_names[@]}"}; do
    if [[ "$plain" != *"$name"* ]]; then
      missing_defaults="$missing_defaults $name"
    fi
  done
  if [ -n "$missing_defaults" ]; then
    disagree "plain help for $model_path omits defaults the JSON carries:$missing_defaults"
  fi

  if [ "$shape_count" -gt 1 ]; then
    undescribed=""
    for index in ${shape_indexes[@]+"${shape_indexes[@]}"}; do
      if [ -z "$(help_value "result.help.shapes.$index.about")" ]; then
        undescribed="$undescribed $(help_value "result.help.shapes.$index.id")"
      fi
    done
    if [ -n "$undescribed" ]; then
      disagree "$model_path has sibling shapes without a description:$undescribed"
    fi
  elif [ "$shape_count" -eq 1 ] && [ -n "$(help_value result.help.shapes.0.about)" ]; then
    disagree "$model_path has one shape that repeats a description"
  fi

  # Each subcommand is spelled `afdata <path...> --help`; recurse on the path.
  help_keys_into sub_indexes result.help.subcommands
  for index in ${sub_indexes[@]+"${sub_indexes[@]}"}; do
    template="$(help_value "result.help.subcommands.$index")"
    case "$template" in
      "afdata "*" --help") ;;
      *) disagree "non-canonical subcommand help template: '$template'" ;;
    esac
    template="${template#afdata }"
    PENDING+=("${template% --help}")
  done
done

afdata_result "help formats agree: $VISITED_COUNT commands, $SHAPES_COUNT registered shapes"
