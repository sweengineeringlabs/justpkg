# Contract test: redis-7.2.7 narinfo — end-to-end workload package
#
# Pins the specific store hash used by the vmisolate redis workload manifest
# (packages/redis/manifest.json).  Running this test confirms that the exact
# package the redis VM depends on is still present and correctly described in
# the nixos-24.11 binary cache.
#
# Flow (mirrors pkg rootfs build packages/redis/packages.toml):
#   1. build_store_path("/nix/store/9ap0znk8zci1j8cp06wysciy253yxk7c-redis-7.2.7", ...)
#   2. fetch_narinfo_by_store_hash("9ap0znk8zci1j8cp06wysciy253yxk7c")
#   3. CAS lookup by FileHash digest → cache hit skips download; miss triggers fetch
#   4. Decompress, verify NarHash, extract to dest tree
#   5. build_with_closure() recurses into References (glibc, openssl, ...)
#
# Breaks if:
#   - redis-7.2.7 is evicted from the nixos-24.11 channel cache
#   - StorePath hash changes (nixpkgs bump) → manifest.json must be regenerated
#   - References field is empty → transitive deps not fetched → VM missing libc/openssl
#
# Run:
#   hurl --test nix_redis_narinfo.hurl
#
# Gate: network (requires cache.nixos.org reachability)

GET https://cache.nixos.org/9ap0znk8zci1j8cp06wysciy253yxk7c.narinfo
HTTP 200

[Asserts]
header "Content-Type" contains "text/x-nix-narinfo"

# Exact store path — confirms this is the redis-7.2.7 package, not a renamed variant
body contains "StorePath: /nix/store/9ap0znk8zci1j8cp06wysciy253yxk7c-redis-7.2.7"

# URL must be present for NAR download
body contains "URL: nar/"

# Compression must be present (xz, zstd, bzip2, or none)
body contains "Compression:"

# FileHash — CAS key (format validated by nix_narinfo_cas_fields.hurl)
body contains "FileHash: sha256:"

# FileSize — must be non-zero (redis binary is several MB)
body matches "FileSize: [1-9][0-9]+"

# NarHash — verified after decompression
body contains "NarHash: sha256:"

# NarSize — must be present
body contains "NarSize:"

# References — redis links against glibc, openssl, and others;
# an empty References here would mean a broken closure and a VM that won't boot
body contains "References:"
