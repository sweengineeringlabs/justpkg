# Contract test: NixOS channel store-paths.xz endpoint
#
# Flow:
#   1. pkg resolve reads packages.toml → determines channel and [[package]] names
#   2. GET {channel_base}/{channel}/store-paths.xz (follows redirect) → ~5 MB xz archive
#   3. xz2 decompressor streams the archive line by line
#   4. Each line is a full store path: /nix/store/{32-char-hash}-{name}-{version}
#   5. Resolver matches each [[package]] name against store path names
#      (longest suffix match, -info packages deprioritised — see fix 8dc308a)
#   6. Best-matching store path written to manifest.json per package
#
# Breaks if:
#   - Endpoint moves → ChannelFetch error on every pkg resolve invocation
#   - Compression changes from xz → xz2 decompressor produces garbage; parse fails
#   - An error page is served with 200 status → magic byte check here catches it
#   - Line format changes → name matching produces wrong or empty store paths
#
# Run:
#   hurl --test nix_channel_store_paths.hurl
#
# Gate: network (requires channels.nixos.org reachability)
# Note: response body is ~5 MB — this test only checks the xz magic bytes, not content.

GET https://channels.nixos.org/nixos-24.11/store-paths.xz
[Options]
location: true
HTTP 200

[Asserts]
# xz magic bytes: 0xFD 0x37 0x7A 0x58 0x5A 0x00
# Hurl checks the raw response body bytes to confirm the file is xz-compressed,
# not a redirect page or error HTML served with a 200 status code.
bytes startsWith hex,fd377a585a00;
