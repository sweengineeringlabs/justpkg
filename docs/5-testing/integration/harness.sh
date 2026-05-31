#!/usr/bin/env bash
# Harness: justpkg Hurl HTTP contract tests.
#
# Locates the repo root, wires environment variables, and runs the
# .hurl files in this directory against the live Nix binary cache.
#
# Usage:
#   bash docs/5-testing/integration/harness.sh [OPTIONS]
#
# Options:
#   --filter PATTERN   only run files whose name matches PATTERN (grep -E)
#   --attic-token TOK  Bearer token for the local Attic cache
#                      (default: read from packages/attic/.env)
#   --attic-url URL    Attic base URL (default: http://127.0.0.1:8080/swe-private)
#   --verbose          pass --verbose to hurl (shows request/response detail)
#   --keep-tmp         do not delete the temp report directory after the run

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Defaults ──────────────────────────────────────────────────────────────────
FILTER=""
ATTIC_TOKEN=""
ATTIC_URL="http://127.0.0.1:8080/swe-private"
VERBOSE=false
KEEP_TMP=false

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)      FILTER="$2";      shift 2 ;;
        --attic-token) ATTIC_TOKEN="$2"; shift 2 ;;
        --attic-url)   ATTIC_URL="$2";   shift 2 ;;
        --verbose)     VERBOSE=true;     shift   ;;
        --keep-tmp)    KEEP_TMP=true;    shift   ;;
        *) echo "error: unknown option: $1" >&2; exit 1 ;;
    esac
done

# ── Locate repo root ──────────────────────────────────────────────────────────
REPO_ROOT=""
SEARCH="$SCRIPT_DIR"
for _ in $(seq 1 10); do
    if [[ -f "$SEARCH/Cargo.toml" ]] && grep -q 'name\s*=\s*"swe_justpkg_cli"' "$SEARCH/main/features/cli/Cargo.toml" 2>/dev/null; then
        REPO_ROOT="$SEARCH"
        break
    fi
    SEARCH="$(dirname "$SEARCH")"
done
if [[ -z "$REPO_ROOT" ]]; then
    echo "error: could not find repo root (looking for main/features/cli/Cargo.toml)" >&2
    exit 1
fi

# ── Auto-load Attic token ─────────────────────────────────────────────────────
ENV_FILE="$REPO_ROOT/packages/attic/.env"
if [[ -z "$ATTIC_TOKEN" && -f "$ENV_FILE" ]]; then
    # shellcheck source=/dev/null
    source "$ENV_FILE"
    ATTIC_TOKEN="${ATTIC_CI_TOKEN:-}"
fi

# ── Prereq checks ─────────────────────────────────────────────────────────────
echo "=== justpkg HTTP contract tests ==="
echo ""
echo "Repo root: $REPO_ROOT"
echo "Filter:    ${FILTER:-<all>}"
echo "Attic:     ${ATTIC_URL}"
echo ""

if ! command -v hurl &>/dev/null; then
    echo "error: hurl not found in PATH" >&2
    echo "  Install: https://hurl.dev/docs/installation.html" >&2
    exit 1
fi

HURL_VERSION=$(hurl --version 2>&1 | head -1)
echo "hurl:      $HURL_VERSION"
echo ""

# ── Temp directory for hurl reports ──────────────────────────────────────────
REPORT_DIR="$(mktemp -d /tmp/justpkg-hurl-XXXXXX)"

cleanup() {
    if [[ "$KEEP_TMP" == "false" ]]; then
        rm -rf "$REPORT_DIR"
    else
        echo ""
        echo "Reports preserved: $REPORT_DIR"
    fi
}
trap cleanup EXIT

# ── Test runner ───────────────────────────────────────────────────────────────
PASS=0
FAIL=0
SKIP=0

HURL_ARGS=(--test)
[[ "$VERBOSE" == "true" ]] && HURL_ARGS+=(--verbose)

run_test() {
    local file="$1"
    shift
    local extra_args=("$@")
    local name
    name="$(basename "$file" .hurl)"

    # Apply filter
    if [[ -n "$FILTER" ]] && ! echo "$name" | grep -qE "$FILTER"; then
        return
    fi

    local report="$REPORT_DIR/${name}.json"
    if hurl "${HURL_ARGS[@]}" "${extra_args[@]}" \
        --report-json "$report" \
        "$file" 2>/dev/null; then
        echo "  ok   $name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL $name"
        FAIL=$((FAIL + 1))
        # On failure, re-run verbosely so the caller can see what went wrong
        if [[ "$VERBOSE" == "false" ]]; then
            echo ""
            hurl --test --verbose "${extra_args[@]}" "$file" 2>&1 | sed 's/^/       /' || true
            echo ""
        fi
    fi
}

skip_test() {
    local name="$1"
    local reason="$2"
    if [[ -n "$FILTER" ]] && ! echo "$name" | grep -qE "$FILTER"; then
        return
    fi
    echo "  skip $name ($reason)"
    SKIP=$((SKIP + 1))
}

# ── Network contract tests ────────────────────────────────────────────────────
echo "Network contract tests (cache.nixos.org, channels.nixos.org):"
run_test "$SCRIPT_DIR/nix_channel_revision.hurl"
run_test "$SCRIPT_DIR/nix_channel_store_paths.hurl"
run_test "$SCRIPT_DIR/nix_narinfo_contract.hurl"
echo ""

# ── Attic substituter test ────────────────────────────────────────────────────
echo "Attic substituter test (local instance):"
ATTIC_HOST="$(echo "$ATTIC_URL" | cut -d/ -f1-3)"  # http://host:port
if curl -sf "$ATTIC_HOST" >/dev/null 2>&1; then
    ATTIC_ARGS=()
    [[ -n "$ATTIC_TOKEN" ]] && ATTIC_ARGS+=(--variable "attic_token=$ATTIC_TOKEN")
    run_test "$SCRIPT_DIR/attic_substituter.hurl" "${ATTIC_ARGS[@]}"
else
    skip_test "attic_substituter" "Attic not running at $ATTIC_HOST — start with: bash packages/attic/run.sh"
fi
echo ""

# ── Summary ───────────────────────────────────────────────────────────────────
TOTAL=$((PASS + FAIL + SKIP))
echo "Results: $PASS passed, $FAIL failed, $SKIP skipped / $TOTAL total"
echo ""

if [[ $FAIL -eq 0 ]]; then
    echo "PASS"
    exit 0
else
    echo "FAIL"
    exit 1
fi
