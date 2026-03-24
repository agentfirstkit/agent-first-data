#!/bin/bash
# Run checks for all agent-first-data language implementations

set -euo pipefail
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${1:-all}"

usage() {
  echo "Usage: $0 [static|unit|all]" >&2
}

ensure_typescript_deps() {
  if [ ! -d "$ROOTPATH/typescript/node_modules" ]; then
    (cd "$ROOTPATH/typescript" && npm install)
  fi
}

run_static() {
  echo "[1/4] Rust (fmt + clippy)"
  (cd "$ROOTPATH/rust" && cargo fmt --all --check)
  (cd "$ROOTPATH/rust" && cargo clippy --lib -- -D warnings)

  echo ""
  echo "[2/4] Go (gofmt + compile)"
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
  echo "[3/4] Python (syntax)"
  (cd "$ROOTPATH/python" && python3 -m compileall agent_first_data examples tests >/dev/null)

  echo ""
  echo "[4/4] TypeScript (typecheck)"
  ensure_typescript_deps
  (cd "$ROOTPATH/typescript" && npx tsc --noEmit)
}

run_unit() {
  echo "[1/4] Rust (tests)"
  (cd "$ROOTPATH/rust" && cargo test)
  (cd "$ROOTPATH/rust" && cargo test --features tracing)
  (cd "$ROOTPATH/rust" && cargo test --examples)

  echo ""
  echo "[2/4] Go"
  (cd "$ROOTPATH/go" && go test -v ./...)

  echo ""
  echo "[3/4] Python"
  (cd "$ROOTPATH/python" && PYTHONPATH=. python3 -m pytest tests/ examples/agent_cli.py -v)

  echo ""
  echo "[4/4] TypeScript"
  ensure_typescript_deps
  (cd "$ROOTPATH/typescript" && npx tsx --test src/*.test.ts)
  (cd "$ROOTPATH/typescript" && npx tsx --test examples/agent_cli.ts)
}

case "$MODE" in
  static)
    run_static
    ;;
  unit)
    run_unit
    ;;
  all)
    run_static
    echo ""
    run_unit
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
