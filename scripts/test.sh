#!/bin/bash
# Run tests for all languages

set -e
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Testing agent-first-data..."
echo ""

echo "[1/4] Rust"
(cd "$ROOTPATH/rust" && cargo test)

echo ""
echo "[2/4] Go"
(cd "$ROOTPATH/go" && go test -v ./...)

echo ""
echo "[3/4] Python"
(cd "$ROOTPATH/python" && python3 -m pytest tests/ -v)

echo ""
echo "[4/4] TypeScript"
(cd "$ROOTPATH/typescript" && npx tsx --test src/*.test.ts)

echo ""
echo "All tests passed!"
