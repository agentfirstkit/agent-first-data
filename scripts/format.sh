#!/bin/bash
# Format source files for languages with configured formatters

set -euo pipefail
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Formatting agent-first-data..."
echo ""

echo "[1/4] Rust - cargo fmt"
(cd "$ROOTPATH/rust" && cargo fmt --all)

echo ""
echo "[2/4] Go - gofmt"
(
  cd "$ROOTPATH/go"
  find . -name '*.go' -type f -print0 | xargs -0 gofmt -w
)

echo ""
echo "[3/4] Python - no formatter configured"

echo ""
echo "[4/4] TypeScript - no formatter configured"

echo ""
echo "Format complete!"
