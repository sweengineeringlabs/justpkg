# Contract test: narinfo fields required by the CAS-backed installer
#
# Three fields are critical to the CAS install path introduced in feat(nix):
#
#   FileHash  — SHA-256 of the compressed NAR (hex or Nix base-32).
#               file_hash_to_digest() decodes this to produce the CAS lookup
#               key. If the encoding or prefix changes, every CAS lookup fails
#               and falls back to re-downloading (correctness preserved but
#               performance regresses to pre-CAS behaviour).
#
#   FileSize  — compressed NAR size in bytes.
#               fetch_compressed_nar() uses this to decide whether to show
#               download progress (threshold: 10 MiB). If absent or zero,
#               large NARs (e.g. OpenSearch JDK) download silently with no
#               user feedback.
#
#   References — space-separated list of transitive dependency basenames.
#                build_with_closure() recurses into each reference to fetch
#                the full closure. If absent or format changes, only the
#                top-level package is installed — the VM fails to boot because
#                shared libraries are missing.
#
# Breaks if:
#   - FileHash encoding changes (e.g. sha256: prefix removed, or length changes)
#   - FileSize field is dropped or renamed
#   - References field format changes (space-separated basenames → something else)
#
# Run:
#   hurl --test nix_narinfo_cas_fields.hurl
#
# Gate: network (requires cache.nixos.org reachability)

# bash-5.2p37 (mjhcjikhxps97mq5z54j4gjjfzgmsir5) — stable reference package.
# Has transitive deps (glibc, ncurses, readline) so References is non-empty.
GET https://cache.nixos.org/mjhcjikhxps97mq5z54j4gjjfzgmsir5.narinfo
HTTP 200

[Asserts]
# FileHash must carry sha256: prefix and a non-empty hash (hex: 64 chars,
# Nix base-32: 52 chars).  file_hash_to_digest() branches on length after
# stripping the prefix — if the format drifts, every CAS key is computed wrong.
body matches "FileHash: sha256:[0-9a-z]{52,64}"

# FileSize must be a non-zero integer — zero or absent means the progress
# threshold check (>= 10 MiB) always produces the wrong answer.
body matches "FileSize: [1-9][0-9]+"

# References must be present.  bash depends on glibc, ncurses, readline —
# if the field is absent the installer treats bash as a leaf and installs
# a broken rootfs (no libc, no terminal support).
body contains "References:"
