#!/bin/bash
# Upgrade dependencies to latest versions for all languages

set -e
ROOTPATH="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Upgrading agent-first-data dependencies..."
echo ""

echo "[1/4] Rust — cargo upgrade"
(cd "$ROOTPATH/rust" && cargo upgrade --incompatible && cargo update)

echo ""
echo "[2/4] Go — go get -u"
(cd "$ROOTPATH/go" && go get -u ./... && go mod tidy)

echo ""
echo "[3/4] TypeScript — npm upgrade"
(cd "$ROOTPATH/typescript" && npm install && npm upgrade)

echo ""
echo "[4/4] Python — no pinned dependencies"

echo ""
echo "Upgrade complete!"
