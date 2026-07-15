#!/bin/bash
# Run checks for all agent-first-data language implementations.

set -euo pipefail
# Force Python UTF-8 mode so scripts reading source files / fixtures / subprocess
# output decode as UTF-8 regardless of OS locale. Windows defaults to cp1252 and
# chokes on non-ASCII bytes (e.g. em dashes in Go/Rust doc comments).
export PYTHONUTF8=1
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOTPATH="$(cd "$SCRIPT_DIR/.." && pwd)"
MODE="${1:-all}"
PYTOOL=python3
AFDATA_NPM_CACHE="${AFDATA_NPM_CACHE:-${TMPDIR:-/tmp}/agent-first-data-npm-cache}"
export NPM_CONFIG_CACHE="${NPM_CONFIG_CACHE:-$AFDATA_NPM_CACHE}"
mkdir -p "$NPM_CONFIG_CACHE"

usage() {
  echo "Usage: $0 [static|unit|e2e|bench|all]" >&2
}

ensure_typescript_deps() {
  if [ ! -d "$ROOTPATH/typescript/node_modules" ]; then
    (cd "$ROOTPATH/typescript" && npm ci --ignore-scripts)
  fi
}

# pytest runs the tests; build + twine are installed too so release hooks can
# reuse this venv. Keep it inside the Python package tree so local Python
# tooling is colocated with pyproject.toml.
ensure_python_test_deps() {
  if python3 -c 'import pytest, build, twine, setuptools' >/dev/null 2>&1; then
    return
  fi
  local venv="${AFDATA_PYVENV:-$ROOTPATH/python/.venv}"
  # venv layout differs by platform: bin/python on Unix, Scripts/python.exe on
  # Windows. Resolve through venv_python instead of hardcoding bin/python, or the
  # provision path 127s on Windows CI.
  local py
  py="$(venv_python "$venv")"
  if ! "$py" -c 'import pytest, build, twine, setuptools' >/dev/null 2>&1; then
    echo "Provisioning Python venv at $venv (pytest, build, twine, setuptools)..."
    python3 -m venv "$venv"
    py="$(venv_python "$venv")"
    "$py" -m pip install -q --upgrade pip pytest build twine setuptools
  fi
  PYTOOL="$py"
}

venv_python() {
  local venv="$1"
  if [ -x "$venv/bin/python" ]; then
    printf '%s\n' "$venv/bin/python"
  else
    printf '%s\n' "$venv/Scripts/python.exe"
  fi
}

run_static() {
  echo "[1/5] Rust (fmt + clippy)"
  (cd "$ROOTPATH" && cargo fmt --all --check)
  (cd "$ROOTPATH" && cargo clippy --all-targets --all-features -- -D warnings)

  echo ""
  echo "[2/5] Spec registry"
  (cd "$ROOTPATH" && python3 scripts/validate_registry.py)
  (cd "$ROOTPATH" && python3 scripts/validate_protocol_docs.py)
  (cd "$ROOTPATH" && python3 scripts/validate_api_surface.py)
  (cd "$ROOTPATH" && python3 scripts/sync_offline_assets.py --check)

  echo ""
  echo "[3/5] Go (gofmt + compile)"
  unformatted_go="$(
    cd "$ROOTPATH/go" &&
      find . -name '*.go' -type f -print0 | xargs -0 gofmt -l
  )"
  if [ -n "$unformatted_go" ]; then
    echo "Go files need formatting:" >&2
    printf '%s\n' "$unformatted_go" >&2
    exit 1
  fi
  (cd "$ROOTPATH/go" && go test -run '^$' ./...)

  echo ""
  echo "[4/5] Python (syntax)"
  (cd "$ROOTPATH/python" && python3 -m compileall agent_first_data examples tests >/dev/null)

  echo ""
  echo "[5/5] TypeScript (typecheck)"
  ensure_typescript_deps
  (cd "$ROOTPATH/typescript" && npx tsc --noEmit)
}

run_unit() {
  echo "[1/4] Rust (tests)"
  (cd "$ROOTPATH" && cargo test --lib --tests)
  (cd "$ROOTPATH" && cargo test --lib --tests --features tracing)
  (cd "$ROOTPATH" && cargo test --all-features)
  (cd "$ROOTPATH" && cargo test --examples --features cli-help,cli-help-markdown)
  (cd "$ROOTPATH" && cargo test --examples --features cli-help,cli-help-markdown,skill-admin)

  echo ""
  echo "[2/4] Go"
  (cd "$ROOTPATH/go" && go test -v ./...)

  echo ""
  echo "[3/4] Python"
  (cd "$ROOTPATH/python" && PYTHONPATH=. "$PYTOOL" -m pytest tests/ examples/agent_cli.py -v)

  echo ""
  echo "[4/4] TypeScript"
  ensure_typescript_deps
  (cd "$ROOTPATH/typescript" && npx tsx --test src/*.test.ts)
  (cd "$ROOTPATH/typescript" && npx tsx --test examples/agent_cli.ts)
}

run_package() {
  echo ""
  echo "[package] Rust"
  rust_package_list="$(cd "$ROOTPATH" && cargo package --allow-dirty --no-verify --list)"
  for asset in spec/registry.json spec/protocol-v1.schema.json skills/agent-first-data/SKILL.md skills/agent-first-data/references/registry.json skills/agent-first-data/references/protocol-v1.schema.json; do
    if ! grep -qx "$asset" <<<"$rust_package_list"; then
      echo "Rust package missing offline asset: $asset" >&2
      exit 1
    fi
  done
  rust_smoke="$(mktemp -d)"
  (cd "$ROOTPATH" && cargo install --path . --root "$rust_smoke/cargo-root" --locked --features skill >/dev/null)
  if [ -x "$rust_smoke/cargo-root/bin/afdata.exe" ]; then
    "$rust_smoke/cargo-root/bin/afdata.exe" --version >/dev/null
    "$rust_smoke/cargo-root/bin/afdata.exe" skill validate "$ROOTPATH/skills/agent-first-data"
  else
    "$rust_smoke/cargo-root/bin/afdata" --version >/dev/null
    "$rust_smoke/cargo-root/bin/afdata" skill validate "$ROOTPATH/skills/agent-first-data"
  fi
  mkdir -p "$rust_smoke/lib/src"
  cat > "$rust_smoke/lib/Cargo.toml" <<EOF
[package]
name = "afdata-rust-smoke"
version = "0.0.0"
edition = "2021"

[dependencies]
agent-first-data = { path = "$ROOTPATH", default-features = false }
EOF
  cat > "$rust_smoke/lib/src/lib.rs" <<'RS'
pub fn smoke() {
    let _ = agent_first_data::build_cli_error("smoke", None);
}
RS
  (cd "$rust_smoke/lib" && cargo check --quiet)
  rm -rf "$rust_smoke"

  echo ""
  echo "[package] Python"
  py_out="$(mktemp -d)"
  had_py_build=0
  had_py_egg=0
  [ -d "$ROOTPATH/python/build" ] && had_py_build=1
  compgen -G "$ROOTPATH/python/*.egg-info" >/dev/null && had_py_egg=1
  (cd "$ROOTPATH/python" && "$PYTOOL" -m build --no-isolation --outdir "$py_out")
  "$PYTOOL" - "$py_out" <<'PY'
import sys
import zipfile
from pathlib import Path

wheel = next(Path(sys.argv[1]).glob("*.whl"))
required = {
    "agent_first_data/assets/registry.json",
    "agent_first_data/assets/protocol-v1.schema.json",
    "agent_first_data/assets/skills/agent-first-data/SKILL.md",
    "agent_first_data/assets/skills/agent-first-data/references/registry.json",
    "agent_first_data/assets/skills/agent-first-data/references/protocol-v1.schema.json",
}
with zipfile.ZipFile(wheel) as archive:
    names = set(archive.namelist())
missing = sorted(required - names)
if missing:
    raise SystemExit(f"Python wheel missing offline assets: {missing}")
PY
  py_smoke="$py_out/venv"
  python3 -m venv "$py_smoke"
  py_smoke_python="$(venv_python "$py_smoke")"
  "$py_smoke_python" -m pip install -q --upgrade pip
  "$py_smoke_python" -m pip install -q "$py_out"/*.whl
  "$py_smoke_python" - <<'PY'
from importlib.resources import files

from agent_first_data import build_cli_error

event = build_cli_error("smoke")
assert event["kind"] == "error"
assets = files("agent_first_data") / "assets"
assert (assets / "registry.json").is_file()
assert (assets / "protocol-v1.schema.json").is_file()
assert (assets / "skills" / "agent-first-data" / "SKILL.md").is_file()
PY
  rm -rf "$py_out"
  [ "$had_py_build" -eq 0 ] && rm -rf "$ROOTPATH/python/build"
  [ "$had_py_egg" -eq 0 ] && rm -rf "$ROOTPATH/python"/*.egg-info

  echo ""
  echo "[package] TypeScript"
  ensure_typescript_deps
  had_ts_dist=0
  [ -d "$ROOTPATH/typescript/dist" ] && had_ts_dist=1
  ts_pack_json="$(cd "$ROOTPATH/typescript" && npm run build >/dev/null && npm pack --dry-run --json)"
  # JavaScript source is intentionally single-quoted shell text.
  # shellcheck disable=SC2016
  node -e '
const pack = JSON.parse(process.argv[1])[0];
const files = new Set(pack.files.map((file) => file.path));
const required = [
  "assets/registry.json",
  "assets/protocol-v1.schema.json",
  "assets/skills/agent-first-data/SKILL.md",
  "assets/skills/agent-first-data/references/registry.json",
  "assets/skills/agent-first-data/references/protocol-v1.schema.json",
];
const missing = required.filter((file) => !files.has(file));
if (missing.length) {
  console.error(`TypeScript package missing offline assets: ${missing.join(", ")}`);
  process.exit(1);
}
' "$ts_pack_json"
  ts_smoke="$(mktemp -d)"
  ts_tarball="$(cd "$ROOTPATH/typescript" && npm pack --json --pack-destination "$ts_smoke" | node -e 'const fs = require("fs"); const pack = JSON.parse(fs.readFileSync(0, "utf8"))[0]; console.log(pack.filename);')"
  (cd "$ts_smoke" && npm init -y >/dev/null && npm install --ignore-scripts "./$ts_tarball" >/dev/null)
  (
    cd "$ts_smoke"
    node --input-type=module - <<'JS'
import { buildCliError } from "agent-first-data";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const event = buildCliError("smoke").toJSON();
if (event.kind !== "error") throw new Error("buildCliError smoke failed");
const distIndex = fileURLToPath(import.meta.resolve("agent-first-data"));
const root = dirname(dirname(distIndex));
for (const file of [
  "assets/registry.json",
  "assets/protocol-v1.schema.json",
  "assets/skills/agent-first-data/SKILL.md",
]) {
  if (!existsSync(join(root, file))) throw new Error(`missing ${file}`);
}
JS
  )
  rm -rf "$ts_smoke"
  [ "$had_ts_dist" -eq 0 ] && rm -rf "$ROOTPATH/typescript/dist"

  echo ""
  echo "[package] Go"
  go_smoke="$(mktemp -d)"
  mkdir -p "$go_smoke"
  (cd "$go_smoke" && go mod init afdata-go-smoke >/dev/null)
  (cd "$go_smoke" && go mod edit -replace github.com/agentfirstkit/agent-first-data/go="$ROOTPATH/go")
  (cd "$go_smoke" && go get github.com/agentfirstkit/agent-first-data/go@v0.0.0 >/dev/null)
  cat > "$go_smoke/afdata_smoke_test.go" <<'GO'
package smoke

import (
	"testing"

	afdata "github.com/agentfirstkit/agent-first-data/go"
)

func TestSmoke(t *testing.T) {
	event, err := afdata.BuildCLIError("smoke", "")
	if err != nil {
		t.Fatalf("BuildCLIError failed: %v", err)
	}
	if event.Value()["kind"] != "error" {
		t.Fatalf("unexpected event: %#v", event.Value())
	}
}
GO
  (cd "$go_smoke" && go test ./...)
  test -f "$ROOTPATH/go/assets/registry.json"
  test -f "$ROOTPATH/go/assets/protocol-v1.schema.json"
  test -f "$ROOTPATH/go/assets/skills/agent-first-data/SKILL.md"
  rm -rf "$go_smoke"
  return 0
}

run_e2e() {
  echo "[e2e] Four-language canonical CLI"
  ensure_typescript_deps
  (cd "$ROOTPATH" && AFDATA_PYTOOL="$PYTOOL" "$PYTOOL" tests/cli_e2e.py)
}

run_bench() {
  echo "[bench] Rust formatter/redaction baselines"
  (cd "$ROOTPATH" && cargo run --release --example afdata_bench --quiet)
}

if [ "$MODE" = "unit" ] || [ "$MODE" = "e2e" ] || [ "$MODE" = "all" ]; then
  ensure_python_test_deps
fi

case "$MODE" in
  static)
    run_static
    ;;
  unit)
    run_unit
    run_package
    ;;
  e2e)
    run_e2e
    ;;
  bench)
    run_bench
    ;;
  all)
    run_static
    echo ""
    run_unit
    run_package
    echo ""
    run_e2e
    ;;
  --help|-h|help)
    usage
    exit 0
    ;;
  *)
    usage
    exit 2
    ;;
esac

echo ""
echo "All checks passed for agent-first-data [$MODE]!"
