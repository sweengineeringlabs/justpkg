# Contract test: NixOS channel git-revision endpoint
#
# Flow:
#   1. pkg resolve reads packages.toml → determines channel (e.g. nixos-24.11)
#   2. GET {channel_base}/{channel}/git-revision (follows redirect)
#   3. Response is a 40-char lowercase hex git SHA (the nixpkgs commit pinned to this channel)
#   4. SHA is stored in manifest.json as meta.nixpkgs_rev for reproducibility
#   5. All subsequent resolution in this run uses the pinned revision
#
# Breaks if:
#   - channels.nixos.org changes URL scheme → ResolveError::ChannelFetch on every pkg resolve
#   - Response format changes (not a bare hex SHA) → revision pinning silently writes garbage
#   - Redirect chain changes and follow fails → pkg resolve returns connection error
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
