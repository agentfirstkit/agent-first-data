#!/bin/bash
# Assert every committed copy of this spore's version agrees.
#
# One spore ships four packages — crates.io, npm, PyPI, and a Go module — and
# the release pipeline reads the version from `Cargo.toml` alone. Every registry
# step is idempotent by version, so a bump that misses one language does not
# fail: npm and PyPI quietly skip publishing while crates.io and the GitHub
# release move forward, and `afdata.Version` in Go reports a version that was
# never shipped. That failure is invisible at release time and permanent
# afterwards, so the agreement is checked here.
#
# Every structured file is read with afdata rather than a regex. This one is
# worth dogfooding: the previous implementation matched `^version\s*=` inside a
# hand-rolled TOML section scan, which is the exact fragility afdata exists to
# remove, and it could not address `package-lock.json`'s root package at all —
# npm keys it by the empty string, which afdata's path grammar rejected until
# 0.27.1.
#
# `go/version.go` is Go source, not a document, so it stays a grep.
#
# Usage: validate_versions.sh
set -euo pipefail

ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Prefer the binary this tree builds over whatever is installed: a validator for
# this spore must judge it with its own code, not with a release that may
# predate the change under test.
AFDATA="${AFDATA_BIN:-}"
if [ -z "$AFDATA" ]; then
  if [ -x "$ROOTPATH/target/debug/afdata" ]; then
    AFDATA="$ROOTPATH/target/debug/afdata"
  else
    AFDATA="afdata"
  fi
fi

read_doc() {
  # FILE KEY [--input-format FORMAT ...]
  local file="$1" key="$2"
  shift 2
  "$AFDATA" value "$ROOTPATH/$file" "$key" "$@" </dev/null
}

# The lockfile's own entry, whose index moves as dependencies come and go.
#
# One call for every package name. `value` re-parses the whole lockfile per
# invocation, so the obvious loop is 110 processes and 110 parses (632ms here);
# a pattern is one of each.
cargo_lock_self_version() {
  local name index
  name="$(read_doc Cargo.toml package.name --input-format toml)"
  index=0
  while IFS= read -r entry_name; do
    if [ "$entry_name" = "$name" ]; then
      read_doc Cargo.lock "package.$index.version" --input-format toml
      return 0
    fi
    index=$((index + 1))
  done < <("$AFDATA" values "$ROOTPATH/Cargo.lock" 'package.*.name' \
    --input-format toml 2>/dev/null || true)
  echo "Cargo.lock: no self entry for $name" >&2
  return 1
}

CANONICAL="$(read_doc Cargo.toml package.version --input-format toml)"

# where<TAB>found, one per line.
FOUND="$(
  printf 'Cargo.toml\t%s\n' "$CANONICAL"
  printf 'Cargo.lock\t%s\n' "$(cargo_lock_self_version)"
  printf 'python/pyproject.toml\t%s\n' \
    "$(read_doc python/pyproject.toml project.version --input-format toml)"
  printf 'typescript/package.json\t%s\n' \
    "$(read_doc typescript/package.json version)"
  printf 'typescript/package-lock.json\t%s\n' \
    "$(read_doc typescript/package-lock.json version)"
  # npm keys the root package by the empty string: `packages..version`.
  printf 'typescript/package-lock.json (packages)\t%s\n' \
    "$(read_doc typescript/package-lock.json packages..version)"
  printf 'spore.core.json\t%s\n' "$(read_doc spore.core.json version)"
  printf 'go/version.go\t%s\n' \
    "$(sed -n 's/^const Version = "\(.*\)"$/\1/p' "$ROOTPATH/go/version.go")"
)"

DISAGREE=""
COUNT=0
while IFS=$'\t' read -r where found; do
  [ -n "$where" ] || continue
  COUNT=$((COUNT + 1))
  if [ "$found" != "$CANONICAL" ]; then
    DISAGREE="$DISAGREE  - $where: ${found:-<unreadable>}"$'\n'
  fi
done <<< "$FOUND"

if [ -n "$DISAGREE" ]; then
  echo "version disagreement (Cargo.toml says $CANONICAL):" >&2
  echo "" >&2
  printf '%s' "$DISAGREE" >&2
  exit 1
fi

echo "versions ok: $CANONICAL in $COUNT places"
