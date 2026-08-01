#!/bin/bash
# Fail when a build artifact is tracked in the package.
#
# A release stages the whole spore tree, so anything tracked ships to crates.io,
# npm, and PyPI. `go build` inside an example directory writes a binary beside
# its source, and a stray `python3` run writes `__pycache__`; both have reached
# a published release this way. Detect them from content, not from a name list.
#
# Usage: validate_no_binaries.sh
set -euo pipefail

ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PROBE_BYTES=8000

# Binary formats that are legitimately part of a package.
is_allowed_suffix() {
  case "$(printf '%s' "${1##*.}" | tr '[:upper:]' '[:lower:]')" in
    png|jpg|jpeg|gif|ico|webp|pdf|woff|woff2|ttf|otf) return 0 ;;
    *) return 1 ;;
  esac
}

# A NUL in the probe means binary. Compare the probe's length with its length
# after NULs are stripped rather than matching a NUL directly, which portable
# grep cannot do. LC_ALL=C keeps `tr` byte-oriented: in a UTF-8 locale it
# rejects any non-UTF-8 byte with "Illegal byte sequence", the counts then
# differ for a reason that is not a NUL, and ordinary source is called binary.
has_nul_byte() {
  local raw stripped
  raw="$(head -c "$PROBE_BYTES" "$1" 2>/dev/null | wc -c | tr -d ' ')"
  stripped="$(head -c "$PROBE_BYTES" "$1" 2>/dev/null | LC_ALL=C tr -d '\000' | wc -c | tr -d ' ')"
  [ "$raw" != "$stripped" ]
}

cd "$ROOTPATH"

scanned=0
offenders=""
while IFS= read -r -d '' tracked; do
  scanned=$((scanned + 1))
  [ -f "$tracked" ] || continue
  is_allowed_suffix "$tracked" && continue
  if has_nul_byte "$tracked"; then
    size="$(wc -c < "$tracked" | tr -d ' ')"
    offenders="$offenders$(printf '%12s  %s' "$size" "$tracked")"$'\n'
  fi
done < <(git ls-files -z)

if [ -n "$offenders" ]; then
  echo "Binary files are tracked and would be published:" >&2
  printf '%s' "$offenders" | sort -rn >&2
  echo "Remove them and add an ignore rule before releasing." >&2
  exit 1
fi

echo "no tracked binaries: $scanned files checked"
