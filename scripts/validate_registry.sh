#!/bin/bash
# Validate the AFDATA registry and its direct documentation references.
#
# Checks: the registry's shape, that no suffix record carries a banned key,
# that every suffix appears in the formal spec and the Skill text, that the
# Skill frontmatter holds exactly `name` and `description`, and that every
# suffix is exercised by a key in the shared fixtures.
#
# Structured files are read with afdata. Fixture keys are collected from the
# fixture text (a quoted token followed by a colon): a string value never
# produces one, because its inner quotes are escaped.
#
# Usage: validate_registry.sh
set -euo pipefail

ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

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

afdata_args_begin "validate_registry.sh [OPTIONS]"
afdata_args_parse "$@"

REGISTRY="$ROOTPATH/spec/registry.json"
SPEC="$ROOTPATH/spec/agent-first-data.md"
SKILL_DIR="$ROOTPATH/skills/agent-first-data"
SKILL="$SKILL_DIR/SKILL.md"
FIXTURES="$ROOTPATH/spec/fixtures"

REQUIRED_TOP_LEVEL="\$schema schema_version name description safe_integer suffixes"
REQUIRED_SUFFIX_KEYS="suffix category json_types unit formatting redaction constraints"
BANNED_KEYS="deprecated legacy old_marker protocol_version"
ALLOWED_JSON_TYPES=" any null boolean integer number string array object "

fail() {
  afdata_error registry_invalid "registry validation failed: $1" || exit 1
  exit 1
}

# Whether word $1 is a member of the space-separated list $2.
in_list() {
  case " $2 " in *" $1 "*) return 0 ;; *) return 1 ;; esac
}

# The document value at KEY as afdata's JSON text, or the empty string.
raw_value() {
  local out
  out="$("$AFDATA_BIN" get "$REGISTRY" "$1" --output json)" || fail "cannot read $1"
  printf '%s' "$out"
}

# Whether the registry holds a value at the dot-path $1.
registry_has() {
  local present
  present="$("$AFDATA_BIN" has "$REGISTRY" "$1" </dev/null)" || fail "cannot read $1"
  [ "$present" = true ]
}

"$AFDATA_BIN" count "$REGISTRY" >/dev/null || fail "registry root must be an object"
for key in $REQUIRED_TOP_LEVEL; do
  registry_has "$key" || fail "missing top-level key: $key"
done

banned_hits=""
for key in $BANNED_KEYS; do
  hit="$(grep -nE "\"$key\"[[:space:]]*:" "$REGISTRY" || :)"
  [ -z "$hit" ] || banned_hits="$banned_hits$key: $hit"$'\n'
done
[ -z "$banned_hits" ] || fail "banned key present: ${banned_hits%$'\n'}"

version="$("$AFDATA_BIN" value "$REGISTRY" schema_version)" || fail "cannot read schema_version"
[ "$version" = "1" ] || fail "schema_version must be 1"
safe_min="$("$AFDATA_BIN" value "$REGISTRY" safe_integer.minimum)" || fail "cannot read safe_integer.minimum"
safe_max="$("$AFDATA_BIN" value "$REGISTRY" safe_integer.maximum)" || fail "cannot read safe_integer.maximum"
safe_count="$("$AFDATA_BIN" count "$REGISTRY" safe_integer)" || fail "cannot read safe_integer"
{ [ "$safe_min" = "-9007199254740991" ] && [ "$safe_max" = "9007199254740991" ] &&
  [ "$safe_count" = "2" ]; } || fail "safe_integer must be ±(2^53-1)"

indexes=() types=()
afdata_into indexes paths "$REGISTRY" suffixes || fail "suffixes must be a non-empty list"
[ "${#indexes[@]}" -gt 0 ] || fail "suffixes must be a non-empty list"
suffixes_json="$(raw_value suffixes)"
case "$suffixes_json" in *'"value":['*) : ;; *) fail "suffixes must be a non-empty list" ;; esac

SUFFIXES=()
for path in "${indexes[@]}"; do
  entry_json="$(raw_value "$path")"
  case "$entry_json" in *'"value":{'*) : ;; *) fail "$path must be an object" ;; esac
  for key in $REQUIRED_SUFFIX_KEYS; do
    registry_has "$path.$key" || fail "$path missing key: $key"
  done
  suffix="$("$AFDATA_BIN" value "$REGISTRY" "$path.suffix")" || fail "$path.suffix must be a string"
  case "$suffix" in _*) : ;; *) fail "$path.suffix must be an underscore-prefixed string" ;; esac
  if [ "${#SUFFIXES[@]}" -gt 0 ] && in_list "$suffix" "${SUFFIXES[*]}"; then
    fail "duplicate suffix $suffix"
  fi
  SUFFIXES+=("$suffix")

  case "$(raw_value "$path.json_types")" in *'"value":[]'*|*'"value":['*) : ;; *) fail "$suffix: json_types must be a non-empty list" ;; esac
  afdata_into types values "$REGISTRY" "$path.json_types.*" || fail "$suffix: json_types must be a non-empty list"
  [ "${#types[@]}" -gt 0 ] || fail "$suffix: json_types must be a non-empty list"
  for type in "${types[@]}"; do
    case "$ALLOWED_JSON_TYPES" in *" $type "*) : ;; *) fail "$suffix: unknown json_types $type" ;; esac
  done

  for key in strip_key plain_yaml; do
    registry_has "$path.formatting.$key" || fail "$suffix: formatting must include strip_key and plain_yaml"
  done
  case "$(raw_value "$path.formatting.strip_key")" in
    *'"value":true'*|*'"value":false'*) : ;;
    *) fail "$suffix: formatting.strip_key must be boolean" ;;
  esac

  registry_has "$path.redaction.mode" || fail "$suffix: redaction must include mode"
  case "$(raw_value "$path.constraints")" in *'"value":{'*) : ;; *) fail "$suffix: constraints must be an object" ;; esac
done

# --- references in the spec and the Skill text -----------------------------

frontmatter_keys="$("$AFDATA_BIN" keys "$SKILL" --input-format yaml-frontmatter)" ||
  fail "standard Skill must start with valid YAML frontmatter"
frontmatter_sorted="$(printf '%s\n' "$frontmatter_keys" | LC_ALL=C sort | tr '\n' ' ')"
[ "$frontmatter_sorted" = "description name " ] ||
  fail "standard Skill frontmatter must contain only name and description, got: $frontmatter_sorted"

skill_text="$(cat "$SKILL")"
while IFS= read -r ref; do
  skill_text="$skill_text"$'\n'"$(cat "$ref")"
done < <(find "$SKILL_DIR/references" -maxdepth 1 -type f \( -name '*.md' -o -name '*.json' \) | LC_ALL=C sort)

for suffix in "${SUFFIXES[@]}"; do
  case "$suffix" in
    *'{'*)
      literal="${suffix//\{code\}/usd}"
      if ! grep -qF -- "$literal" "$SPEC" && ! grep -qF -- "$literal" <<<"$skill_text"; then
        fail "pattern suffix $suffix has no concrete reference in spec or skill"
      fi
      ;;
    *)
      grep -qF -- "$suffix" "$SPEC" || fail "$suffix missing from formal spec"
      grep -qF -- "$suffix" <<<"$skill_text" || fail "$suffix missing from Skill text"
      ;;
  esac
done

# --- fixture coverage ------------------------------------------------------

fixture_keys="$(cat "$FIXTURES"/*.json | grep -oE '"[^"\\]*"[[:space:]]*:' | sed -E 's/^"([^"]*)".*/\1/' | LC_ALL=C sort -u)"
[ -n "$fixture_keys" ] || fail "no keys found in shared fixtures"

for suffix in "${SUFFIXES[@]}"; do
  case "$suffix" in
    *'{'*)
      word="${suffix##*_}"
      excluded=""
      for other in "${SUFFIXES[@]}"; do
        case "$other" in *'{'*) continue ;; esac
        case "$other" in *_*) [ "${other##*_}" = "$word" ] && excluded="$excluded${excluded:+|}$other" ;; esac
      done
      candidates="$(printf '%s\n' "$fixture_keys" | grep -E "_[A-Za-z]{3,4}_${word}\$" || :)"
      if [ -n "$excluded" ] && [ -n "$candidates" ]; then
        candidates="$(printf '%s\n' "$candidates" | grep -vE "(${excluded})\$" || :)"
      fi
      [ -n "$candidates" ] || fail "pattern suffix $suffix missing from shared fixtures"
      ;;
    *)
      grep -qE -- "${suffix}\$" <<<"$fixture_keys" || fail "$suffix missing from shared fixtures"
      ;;
  esac
done

afdata_result "registry ok: ${#SUFFIXES[@]} suffix records"
