# Contract test: NixOS channel store-paths.xz endpoint
#
# What this validates:
#   pkg resolve downloads store-paths.xz to map package names to store paths.
#   This test verifies the endpoint is reachable, returns a non-empty body,
#   and serves the xz-compressed content that justpkg_resolve::fetch_store_paths
#   passes to the xz2 decompressor.
#
#   If the endpoint moved, changed compression format, or started returning an
#   error page, pkg resolve would fail on every invocation.
#
# Run:
#   hurl --test nix_channel_store_paths.hurl
#
# Gate: network (requires channels.nixos.org reachability)
# Note: response body is ~5 MB — this test only checks headers, not content.

GET https://channels.nixos.org/nixos-24.11/store-paths.xz
[Options]
location: true
HTTP 200

[Asserts]
# xz magic bytes: 0xFD 0x37 0x7A 0x58 0x5A 0x00
# Hurl checks the raw response body bytes to confirm the file is xz-compressed,
# not a redirect page or error HTML served with a 200 status code.
bytes startsWith hex,fd377a585a00;
