# justpkg

Pure-Rust, cross-platform Nix NAR fetcher, verifier, and extractor.
No `nix` binary, no chroot, no Docker, no WSL.

Fetches packages from `cache.nixos.org` using a `flake.lock` as the
source of truth for the full transitive closure. Every blob is
content-addressed and verified against its SHA-256 digest before extraction.

## Usage

```bash
# Resolve, fetch, and extract all locked packages into ./out
justpkg build flake.lock ./out

# Fetch only (populate local CAS cache)
justpkg fetch flake.lock

# Extract from cache into ./out
justpkg extract flake.lock ./out
```

## Architecture

```
justpkg/
  main/features/
    core/   swe_justpkg_core   HTTP client trait, path sanitizer, error types
    nix/    swe_justpkg_nix    flake.lock parser, narinfo fetcher, NAR extractor
    cli/    swe_justpkg_cli    justpkg binary
```

CAS storage is provided by the `justcas` sibling crate (`swe_justcas_cas`).
Blobs are cached at `~/.cache/justpkg/` and only fetched from the CDN on
a cache miss.

## Deferred backends

- `justpkg-apk` — Alpine APK (implement when a workload requires it)
- `justpkg-apt` — Debian/Ubuntu APT (implement when a workload requires it)

See ADR-020 for the CAS-first package strategy rationale.
