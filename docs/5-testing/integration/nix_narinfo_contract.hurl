# Contract test: narinfo format served by cache.nixos.org
#
# Flow:
#   1. pkg install reads manifest.json → gets a 32-char store hash per package
#   2. NixFetcher::fetch_narinfo() builds URL: {cache_base}/{hash}.narinfo
#   3. GET {cache}/{hash}.narinfo → text/x-nix-narinfo response
#   4. NarInfo::parse() extracts: StorePath, URL, Compression, FileHash, NarHash, NarSize
#   5. URL (e.g. "nar/abc...xz") is joined with cache_base → full NAR download URL
#   6. Compression selects the decompressor: xz → xz2, zstd → zstd, bzip2 → bzip2
#   7. NAR is downloaded, decompressed, and FileHash / NarHash verified
#   8. NAR is extracted into the dest directory under its full Nix store path
#
# Breaks if:
#   - cache.nixos.org changes Content-Type → NarInfo::parse() rejects the response
#   - Any required field renamed or removed → parse() errors; install fails for all packages
#   - URL field format changes → NAR download URL malformed → install fails
#
# Run:
#   hurl --test nix_narinfo_contract.hurl
#
# Gate: network (requires cache.nixos.org reachability)

# bash-5.2p37 (mjhcjikhxps97mq5z54j4gjjfzgmsir5) — stable, well-known store hash,
# referenced in every vmisolate workload manifest; unlikely to be evicted.
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
