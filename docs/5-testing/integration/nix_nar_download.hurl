# Contract test: narinfo URL field leads to a valid downloadable NAR
#
# Flow:
#   1. pkg install calls fetch_narinfo() → GET {cache}/{hash}.narinfo
#   2. Extracts the URL field: e.g. "nar/0abc...def.nar.xz"
#   3. Joins URL with cache_base: https://cache.nixos.org/nar/0abc...def.nar.xz
#   4. GET the full NAR URL → downloads the compressed archive
#   5. Decompressor reads the first bytes to detect format (xz magic: fd 37 7a 58 5a 00)
#   6. NAR is extracted into dest tree; NarHash verified against narinfo
#   This test covers steps 1-5 end-to-end with a real narinfo + real NAR download.
#
# Breaks if:
#   - URL field in narinfo is relative but pkg joins it incorrectly → 404 on NAR fetch
#   - cache.nixos.org changes the NAR URL scheme → every install fails after narinfo succeeds
#   - NAR compression format changes → decompressor magic check fails; xz2 rejects the stream
#   - CDN serves a redirect page with 200 instead of the actual NAR bytes → magic check catches it
#
# Run:
#   hurl --test nix_nar_download.hurl
#
# Gate: network (requires cache.nixos.org reachability; downloads ~3 MB)

# Step 1: Fetch narinfo for bash-5.2p37, capture the NAR URL field.
# This is the same narinfo validated by nix_narinfo_contract.hurl — here we
# use it as a live fixture to drive the actual NAR download in step 2.
GET https://cache.nixos.org/mjhcjikhxps97mq5z54j4gjjfzgmsir5.narinfo
HTTP 200
[Captures]
nar_url: body regex "URL: (nar/[^\\n]+)"

# Step 2: Download the NAR using the captured URL.
# pkg constructs this URL as: {cache_base} + "/" + {URL field from narinfo}.
# The response must be a valid xz-compressed NAR (xz magic: fd 37 7a 58 5a 00).
GET https://cache.nixos.org/{{nar_url}}
HTTP 200

[Asserts]
# xz magic bytes — confirms the body is the actual compressed NAR, not an error page.
# If the URL field is wrong or the CDN substitutes an HTML error, this assertion fails.
bytes startsWith hex,fd377a585a00;
