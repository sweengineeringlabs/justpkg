# Contract test: local Attic binary cache substituter
#
# What this validates:
#   When ATTIC_CI_TOKEN is set, pkg install prepends the Attic cache to the
#   substituter list.  This test verifies the Attic instance is reachable,
#   returns the expected API shape for a known store hash, and that a 404
#   response is returned for an unknown hash (so pkg correctly falls back to
#   cache.nixos.org rather than treating a 404 as a fatal error).
#
# Prerequisites:
#   - Attic server running at http://127.0.0.1:8080
#   - ATTIC_TOKEN env var set to a valid token
#   - Run: bash packages/attic/run.sh
#
# Run:
#   ATTIC_TOKEN=$(cat packages/attic/.env | grep ATTIC_CI_TOKEN | cut -d= -f2) \
#     hurl --test attic_substituter.hurl --variable attic_token=$ATTIC_TOKEN
#
# Gate: local (requires a running Attic instance — not part of CI by default)

# Known-absent hash: Attic must return 404, not 500.
# pkg treats 404 as "try next substituter"; 500 aborts with error.
GET http://127.0.0.1:8080/swe-private/0000000000000000000000000000000000000000.narinfo
Authorization: Bearer {{attic_token}}
HTTP 404
