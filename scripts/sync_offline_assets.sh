#!/bin/bash
# Sync or validate the offline AFDATA assets bundled by release packages.
#
# The exact machine-readable contracts stay byte-identical in every language
# package and in the installed skill. Narrative skill references stay focused
# and are maintained separately.
#
# Syncing only ever writes the canonical list, so a contract deleted from
# `spec/` lingers in all four package trees, and `typescript/package.json`
# (`files: assets`) and `python/pyproject.toml` (`assets/*.json`) would ship it.
# Checking for missing files alone cannot see that, so orphans are reported too.
#
# Usage: sync_offline_assets.sh [--check]

# check is assigned by afdata_args_parse, which this file gets from
# `afdata shell bash` at runtime — not from a file a static checker can read.
# shellcheck disable=SC2154
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

afdata_args_begin "sync_offline_assets.sh [OPTIONS]"
afdata_args_flag check --check "Validate without writing"
afdata_args_parse "$@"

# Canonical contract, relative to the root.
CANONICAL_FILES=(
  spec/registry.json
  spec/protocol-v1.schema.json
  spec/cli-help-v2.schema.json
  spec/cli-spec-v1.schema.json
)

PACKAGE_ASSET_ROOTS=(
  go/assets
  python/agent_first_data/assets
  typescript/assets
  skills/agent-first-data/references
)

cd "$ROOTPATH"

FAILURES=""
fail() {
  FAILURES="$FAILURES$1"$'\n'
}

for asset_root in "${PACKAGE_ASSET_ROOTS[@]}"; do
  for canonical in "${CANONICAL_FILES[@]}"; do
    target="$asset_root/${canonical##*/}"
    if [ "$check" = true ]; then
      if [ ! -e "$target" ]; then
        fail "missing $target"
      elif ! cmp -s "$canonical" "$target"; then
        fail "stale $target; run scripts/sync_offline_assets.sh"
      fi
      continue
    fi
    mkdir -p "$asset_root"
    cp "$canonical" "$target"
  done
done

for asset_root in "${PACKAGE_ASSET_ROOTS[@]}"; do
  [ -d "$asset_root" ] || continue
  for found in "$asset_root"/*.json; do
    [ -e "$found" ] || continue
    expected=false
    for canonical in "${CANONICAL_FILES[@]}"; do
      if [ "${found##*/}" = "${canonical##*/}" ]; then
        expected=true
      fi
    done
    if [ "$expected" = false ]; then
      fail "orphan $found; it is no longer a canonical contract"
    fi
  done
done

if [ -n "$FAILURES" ]; then
  afdata_error offline_assets_out_of_sync "${FAILURES%$'\n'}" "Run scripts/sync_offline_assets.sh" || exit 1
fi
if [ "$check" = true ]; then
  afdata_result "offline assets in sync"
else
  afdata_result "offline assets synced"
fi
