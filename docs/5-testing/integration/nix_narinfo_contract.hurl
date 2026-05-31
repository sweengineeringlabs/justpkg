# Contract test: narinfo format served by cache.nixos.org
#
# What this validates:
#   The binary cache returns a valid narinfo response that justpkg_nix::NarInfo::parse()
#   can consume.  If the cache changes the field names, encoding, or Content-Type, pkg
#   would silently fail to fetch packages — this test makes that failure explicit.
#
# Run:
#   hurl --test nix_narinfo_contract.hurl
#
# Gate: network (requires cache.nixos.org reachability)

# bash-5.2p37 — stable, well-known store hash, unlikely to be evicted
GET https://cache.nixos.org/mjhcjikhxps97mq5z54j4gjjfzgmsir5.narinfo
HTTP 200

[Asserts]
# Content-Type must be the Nix-specific MIME type that pkg uses to detect narinfo
header "Content-Type" contains "text/x-nix-narinfo"

# StorePath must be present and point to the Nix store
body contains "StorePath: /nix/store/"

# URL must be present — pkg uses it to build the NAR download URL
body contains "URL: nar/"

# Compression field must be present — pkg selects the decompressor from this
body contains "Compression:"

# Integrity hashes must be present for pkg to verify downloads
body contains "FileHash: sha256:"
body contains "NarHash: sha256:"

# NarSize must be present — pkg uses it for progress reporting
body contains "NarSize:"
