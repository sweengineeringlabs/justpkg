# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| v0 (current) | ✓ |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Report vulnerabilities by email to: phdsystemz@gmail.com

Include a description of the vulnerability, steps to reproduce, potential impact, and any suggested fix. You will receive an acknowledgement within 72 hours. We aim to release a fix within 14 days for confirmed vulnerabilities.

## Scope

justpkg fetches blobs from `cache.nixos.org` and extracts them onto the host filesystem. Security concerns relevant to this project include:

- **Path traversal** in NAR extraction (`safe_path_join` guards every archive entry; see `core/src/api/traits.rs`)
- **Hash mismatch** — blob content must match the SHA-256 in the narinfo before extraction
- **Untrusted archive data** — malformed NAR entries, integer overflow in length fields
- **MITM / CDN compromise** — narinfo and NAR bytes are fetched over HTTPS; hash verification is the final gate

Out of scope: Nix store daemon, `nix` binary, kernel-level vulnerabilities, issues in `cache.nixos.org` infrastructure.
