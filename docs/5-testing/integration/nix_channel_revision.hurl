# Contract test: NixOS channel git-revision endpoint
#
# What this validates:
#   pkg resolve fetches this endpoint to pin the nixpkgs revision in manifest.json.
#   If channels.nixos.org changes the URL scheme or response format, all pkg resolve
#   invocations would fail with a cryptic error.  This test catches that early.
#
# Run:
#   hurl --test nix_channel_revision.hurl
#
# Gate: network (requires channels.nixos.org reachability)

GET https://channels.nixos.org/nixos-24.11/git-revision
[Options]
location: true
HTTP 200

[Asserts]
# Response must be a 40-character lowercase hex git SHA.
# pkg validates length >= 7; the full 40-char check here is stricter.
body matches "^[0-9a-f]{40}\n?$"
