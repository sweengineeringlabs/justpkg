# Contract test: Attic returns a valid narinfo for a known-present store hash
#
# Flow:
#   1. pkg install: Attic is the first substituter; GET {attic_url}/{hash}.narinfo
#   2. Hash IS present → Attic returns 200 with a narinfo body
#   3. NarInfo::parse() processes it identically to a cache.nixos.org narinfo
#      (same parser, same required fields — Attic is a drop-in narinfo server)
#   4. URL field may be an Attic-served NAR path or a redirect to the upstream NAR
#   5. pkg downloads and verifies the NAR; install completes from Attic, not nixos cache
#   Pair with attic_substituter.hurl (404 path) to cover both substituter outcomes.
#
# Breaks if:
#   - Attic narinfo format diverges from cache.nixos.org → NarInfo::parse() fails
#     even though the package is cached → Attic cache is never used, all hits fall through
#   - Authorization header format changes → 401/403 → is_cache_miss() = false → abort
#   - Missing FileHash field → hash verification skipped → silent corruption risk
#
# Variables (injected by harness.sh):
#   attic_url   — Attic cache base URL, e.g. http://127.0.0.1:8080/swe-private
#   attic_token — Bearer token (empty string for unauthenticated caches)
#   attic_hash  — 32-char Nix base32 store hash known to be present in Attic
#                 (auto-detected from packages/fleet/manifest.json by harness.sh)
#
# Gate: local (requires a running Attic instance with at least one cached package)

GET {{attic_url}}/{{attic_hash}}.narinfo
Authorization: Bearer {{attic_token}}
HTTP 200

[Asserts]
# Attic must serve the Nix-specific MIME type — pkg uses this to detect narinfos
header "Content-Type" contains "text/x-nix-narinfo"

# Fields required by NarInfo::parse() in justpkg_nix
body contains "StorePath: /nix/store/"
body contains "URL:"
body contains "Compression:"
body contains "NarHash: sha256:"
body contains "NarSize:"
