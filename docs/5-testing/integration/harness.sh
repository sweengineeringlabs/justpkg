#!/usr/bin/env bash
# Harness: justpkg Hurl HTTP contract tests.
#
# Locates the repo root, checks and installs prerequisites, wires environment
# variables, and runs the .hurl files in this directory against the live Nix
# binary cache.
#
# Usage:
#   bash docs/5-testing/integration/harness.sh [OPTIONS]
#
# Options:
#   --filter PATTERN        only run files whose name matches PATTERN (grep -E)
#   --attic-token TOK       Bearer token for the local Attic cache
#                           (default: read from packages/attic/.env)
#   --attic-url URL         Attic base URL (default: http://127.0.0.1:8080/swe-private)
#   --attic-known-hash HASH 32-char Nix base32 hash known to be in Attic (enables
#                           positive narinfo contract test); auto-detected from
#                           packages/fleet/manifest.json when Attic is running
#   --verbose               pass --verbose to hurl (shows request/response detail)
#   --keep-tmp              do not delete the temp report directory after the run
#   --no-install            abort instead of auto-installing missing prerequisites

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Defaults ──────────────────────────────────────────────────────────────────
FILTER=""
ATTIC_TOKEN=""
ATTIC_URL="http://127.0.0.1:8080/swe-private"
ATTIC_KNOWN_HASH=""
VERBOSE=false
KEEP_TMP=false
NO_INSTALL=false

# ── Argument parsing ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)           FILTER="$2";           shift 2 ;;
        --attic-token)      ATTIC_TOKEN="$2";      shift 2 ;;
        --attic-url)        ATTIC_URL="$2";        shift 2 ;;
        --attic-known-hash) ATTIC_KNOWN_HASH="$2"; shift 2 ;;
        --verbose)          VERBOSE=true;           shift   ;;
        --keep-tmp)         KEEP_TMP=true;          shift   ;;
        --no-install)       NO_INSTALL=true;        shift   ;;
        *) echo "error: unknown option: $1" >&2; exit 1 ;;
    esac
done

# ── Platform detection ────────────────────────────────────────────────────────
detect_platform() {
    case "$(uname -s)" in
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        Darwin)                echo "macos"   ;;
        Linux)                 echo "linux"   ;;
        *)                     echo "unknown" ;;
    esac
}
PLATFORM="$(detect_platform)"

# ── Prerequisite installer ────────────────────────────────────────────────────
# require_tool NAME [INSTALL_HINT]
# Checks if NAME is on PATH. If not, attempts auto-install based on platform.
# Exits with an error if --no-install was passed or installation fails.
require_tool() {
    local name="$1"
    local hint="${2:-}"

    if command -v "$name" &>/dev/null; then
        return 0
    fi

    if [[ "$NO_INSTALL" == "true" ]]; then
        echo "error: $name not found in PATH" >&2
        [[ -n "$hint" ]] && echo "  $hint" >&2
        exit 1
    fi

    echo "==> $name not found — attempting install..."

    case "$name" in
        hurl)
            case "$PLATFORM" in
                windows)
                    if command -v winget &>/dev/null; then
                        winget install --id=hurl.hurl -e --silent || true
                    elif command -v scoop &>/dev/null; then
                        scoop install hurl || true
                    else
                        echo "error: hurl not found; install manually: https://hurl.dev/docs/installation.html" >&2
                        exit 1
                    fi
                    ;;
                macos)
                    if command -v brew &>/dev/null; then
                        brew install hurl
                    else
                        echo "error: hurl not found; install: brew install hurl" >&2
                        exit 1
                    fi
                    ;;
                linux)
                    # Official install script — works on Debian/Ubuntu and RHEL-family.
                    curl -sSfL https://raw.githubusercontent.com/Orange-OpenSource/hurl/master/install.sh \
                        | bash -s -- --prefix /usr/local 2>/dev/null \
                        || apt-get install -y hurl 2>/dev/null \
                        || { echo "error: hurl install failed; see https://hurl.dev/docs/installation.html" >&2; exit 1; }
                    ;;
                *)
                    echo "error: hurl not found; install manually: https://hurl.dev/docs/installation.html" >&2
                    exit 1
                    ;;
            esac
            ;;
        jq)
            case "$PLATFORM" in
                windows)
                    if command -v winget &>/dev/null; then
                        winget install --id=jqlang.jq -e --silent || true
                    elif command -v choco &>/dev/null; then
                        choco install jq -y || true
                    else
                        echo "error: jq not found; install: winget install jqlang.jq" >&2
                        exit 1
                    fi
                    ;;
                macos)
                    command -v brew &>/dev/null && brew install jq \
                        || { echo "error: jq not found; install: brew install jq" >&2; exit 1; }
                    ;;
                linux)
                    apt-get install -y jq 2>/dev/null \
                        || yum install -y jq 2>/dev/null \
                        || { echo "error: jq install failed; install: apt-get install jq" >&2; exit 1; }
                    ;;
                *)
                    echo "error: jq not found" >&2; exit 1 ;;
            esac
            ;;
        curl)
            case "$PLATFORM" in
                linux)
                    apt-get install -y curl 2>/dev/null \
                        || { echo "error: curl not found; install: apt-get install curl" >&2; exit 1; }
                    ;;
                *)
                    echo "error: curl not found in PATH" >&2; exit 1 ;;
            esac
            ;;
        *)
            echo "error: $name not found in PATH" >&2
            [[ -n "$hint" ]] && echo "  $hint" >&2
            exit 1
            ;;
    esac

    # Verify the install succeeded.
    if ! command -v "$name" &>/dev/null; then
        echo "error: $name install appeared to succeed but is still not on PATH" >&2
        echo "  Try opening a new shell or adding the install directory to PATH" >&2
        exit 1
    fi

    echo "    $name installed: $(command -v "$name")"
}

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

# ── Check prerequisites ───────────────────────────────────────────────────────
echo "=== justpkg HTTP contract tests ==="
echo ""
echo "Repo root: $REPO_ROOT"
echo "Platform:  $PLATFORM"
echo ""
echo "Checking prerequisites..."

require_tool curl
require_tool jq
require_tool hurl

# Check that the pkg binary exists (built or on PATH).
PKG_BIN=""
for candidate in \
    "$(command -v pkg 2>/dev/null || true)" \
    "$(command -v pkg.exe 2>/dev/null || true)" \
    "$REPO_ROOT/target/release/pkg" \
    "$REPO_ROOT/target/release/pkg.exe"; do
    [[ -x "$candidate" ]] && PKG_BIN="$candidate" && break
done
if [[ -z "$PKG_BIN" ]]; then
    echo ""
    echo "error: pkg binary not found in PATH or target/release/" >&2
    echo "  Build it first:" >&2
    echo "    cargo build -p swe_justpkg_cli --release" >&2
    exit 1
fi

echo ""
echo "hurl:      $(hurl --version 2>&1 | head -1)"
echo "jq:        $(jq --version)"
echo "curl:      $(curl --version | head -1)"
echo "pkg:       $PKG_BIN"
echo ""
echo "Filter:    ${FILTER:-<all>}"
echo "Attic:     ${ATTIC_URL}"
echo ""

# ── Auto-load Attic token ─────────────────────────────────────────────────────
ENV_FILE="$REPO_ROOT/packages/attic/.env"
if [[ -z "$ATTIC_TOKEN" && -f "$ENV_FILE" ]]; then
    # shellcheck source=/dev/null
    source "$ENV_FILE"
    ATTIC_TOKEN="${ATTIC_CI_TOKEN:-}"
fi

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

# Bounded timeouts so a flaky upstream doesn't hang the whole run.
HURL_ARGS=(--test --connect-timeout 10 --max-time 30)
[[ "$VERBOSE" == "true" ]] && HURL_ARGS+=(--verbose)

run_test() {
    local file="$1"
    shift
    local extra_args=("$@")
    local name
    name="$(basename "$file" .hurl)"

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
        if [[ "$VERBOSE" == "false" ]]; then
            echo ""
            hurl --test --verbose --connect-timeout 10 --max-time 30 \
                "${extra_args[@]}" "$file" 2>&1 | sed 's/^/       /' || true
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
run_test "$SCRIPT_DIR/nix_narinfo_cas_fields.hurl"
run_test "$SCRIPT_DIR/nix_redis_narinfo.hurl"
run_test "$SCRIPT_DIR/nix_nar_download.hurl"
echo ""

# ── Attic substituter tests ───────────────────────────────────────────────────
echo "Attic substituter tests (local instance):"
ATTIC_HOST="$(echo "$ATTIC_URL" | cut -d/ -f1-3)"

if curl -sf --connect-timeout 3 "$ATTIC_HOST" >/dev/null 2>&1; then
    ATTIC_ARGS=(--variable "attic_token=${ATTIC_TOKEN:-}")

    run_test "$SCRIPT_DIR/attic_substituter.hurl" "${ATTIC_ARGS[@]}"

    if [[ -z "$ATTIC_KNOWN_HASH" ]]; then
        FLEET_MANIFEST="$REPO_ROOT/packages/fleet/manifest.json"
        if [[ -f "$FLEET_MANIFEST" ]]; then
            BASH_PATH=$(jq -r '.packages.bash // empty' "$FLEET_MANIFEST")
            if [[ -n "$BASH_PATH" ]]; then
                ATTIC_KNOWN_HASH=$(basename "$BASH_PATH" | cut -c1-32)
            fi
        fi
    fi

    if [[ -n "$ATTIC_KNOWN_HASH" ]]; then
        run_test "$SCRIPT_DIR/attic_narinfo_positive.hurl" \
            "${ATTIC_ARGS[@]}" \
            --variable "attic_url=$ATTIC_URL" \
            --variable "attic_hash=$ATTIC_KNOWN_HASH"
    else
        skip_test "attic_narinfo_positive" \
            "no known hash — pass --attic-known-hash or ensure packages/fleet/manifest.json exists"
    fi
else
    skip_test "attic_substituter"      "Attic not running at $ATTIC_HOST — start with: bash packages/attic/run.sh"
    skip_test "attic_narinfo_positive" "Attic not running at $ATTIC_HOST"
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
