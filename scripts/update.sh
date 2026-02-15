#!/bin/bash
# Update lock files for all languages

set -e
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Updating agent-first-data dependencies..."
echo ""

echo "[1/4] Rust — cargo update"
(cd "$ROOTPATH/rust" && cargo update)

echo ""
echo "[2/4] Go — go mod tidy"
(cd "$ROOTPATH/go" && go mod tidy)

echo ""
echo "[3/4] TypeScript — npm update"
(cd "$ROOTPATH/typescript" && npm install && npm update)

echo ""
echo "[4/4] Python — no lock file"

echo ""
echo "Update complete!"
