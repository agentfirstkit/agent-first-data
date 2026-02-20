#!/bin/bash
# Run tests for all languages

set -e
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Testing agent-first-data..."
echo ""

echo "[1/4] Rust (policy + tests)"
(cd "$ROOTPATH/rust" && cargo clippy --lib -- -D warnings)
(cd "$ROOTPATH/rust" && cargo test)
(cd "$ROOTPATH/rust" && cargo test --examples)

echo ""
echo "[2/4] Go"
(cd "$ROOTPATH/go" && go test -v ./...)

echo ""
echo "[3/4] Python"
(cd "$ROOTPATH/python" && PYTHONPATH=. python3 -m pytest tests/ examples/agent_cli.py -v)

echo ""
echo "[4/4] TypeScript"
(cd "$ROOTPATH/typescript" && npx tsx --test src/*.test.ts)
(cd "$ROOTPATH/typescript" && npx tsx --test examples/agent_cli.ts)

echo ""
echo "All tests passed!"
