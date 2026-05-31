# Contract test: Attic returns a valid narinfo for a known-present store hash
#
# What this validates:
#   The 404 test proves Attic handles cache misses correctly so pkg falls back.
#   This test proves the happy path: when a hash IS in Attic, the response shape
#   is a valid narinfo that justpkg_nix::NarInfo::parse() can consume.
#   If Attic changed its narinfo format, pkg install would silently fail to
#   parse packages even though the cache was reachable — this test catches that.
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
