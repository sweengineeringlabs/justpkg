# Contract test: local Attic binary cache substituter — cache-miss path
#
# Flow:
#   1. pkg install reads the substituter list: Attic first, cache.nixos.org second
#      (from [[nix.substituters]] in application.toml, or --substituter CLI flag)
#   2. For each package: GET {attic_url}/{hash}.narinfo with Authorization: Bearer {token}
#   3. Hash absent in Attic → Attic returns 404
#   4. is_cache_miss() returns true for 404 → fetch_with_fallback() advances to next substituter
#   5. GET cache.nixos.org/{hash}.narinfo → found → install proceeds from public cache
#   Step 4 also triggers on status 0 (connection refused / timeout) — Attic being
#   down does not abort the install; it falls through to cache.nixos.org (fix 4fd93bf).
#
# Breaks if:
#   - Attic returns 500 instead of 404 for absent hash → is_cache_miss() = false
#     → fetch_with_fallback() aborts → install fails whenever Attic has partial cache
#   - Attic returns 200 with garbage body for absent hash → NarInfo::parse() fails
#     with a non-404 error → same abort path
#
# Prerequisites:
#   - Attic server running at http://127.0.0.1:8080 (bash packages/attic/run.sh)
#   - ATTIC_TOKEN env var set to a valid Bearer token (packages/attic/.env)
#
# Run:
#   ATTIC_TOKEN=$(grep ATTIC_CI_TOKEN packages/attic/.env | cut -d= -f2) \
#     hurl --test attic_substituter.hurl --variable attic_token=$ATTIC_TOKEN
#
# Gate: local (requires a running Attic instance — not part of CI by default)

# All-zero hash is guaranteed absent from any real Attic cache.
# Attic must return 404 (cache miss), not 500 (server error).
GET http://127.0.0.1:8080/swe-private/0000000000000000000000000000000000000000.narinfo
Authorization: Bearer {{attic_token}}
HTTP 404
