#!/usr/bin/env bash
# Run all justpkg Hurl HTTP contract tests.
#
# Prerequisites:
#   hurl >= 4.0 on PATH  (https://hurl.dev/docs/installation.html)
#   Network access to cache.nixos.org and channels.nixos.org
#
# Usage:
#   bash docs/5-testing/integration/run.sh
#
# Exit code: 0 if all tests pass, non-zero otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

command -v hurl >/dev/null 2>&1 \
    || { echo "error: hurl not found — install from https://hurl.dev/docs/installation.html" >&2; exit 1; }

echo "==> Running justpkg HTTP contract tests..."
echo ""

PASS=0
FAIL=0

run_test() {
    local file="$1"
    local name
    name="$(basename "$file" .hurl)"
    if hurl --test "$file" 2>/dev/null; then
        echo "  ok  $name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL $name"
        FAIL=$((FAIL + 1))
    fi
}

# Network contract tests — require internet access
for f in \
    "$SCRIPT_DIR/nix_channel_revision.hurl" \
    "$SCRIPT_DIR/nix_channel_store_paths.hurl" \
    "$SCRIPT_DIR/nix_narinfo_contract.hurl"; do
    run_test "$f"
done

# Attic test only if a local instance is running
if curl -sf http://127.0.0.1:8080 >/dev/null 2>&1; then
    run_test "$SCRIPT_DIR/attic_substituter.hurl"
else
    echo "  skip attic_substituter (Attic not running on 127.0.0.1:8080)"
fi

echo ""
echo "Results: $PASS passed, $FAIL failed"

[[ $FAIL -eq 0 ]] || exit 1
