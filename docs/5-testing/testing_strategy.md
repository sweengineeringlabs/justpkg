# justpkg Testing Strategy

> **TLDR:** Three-layer test pyramid — Rust unit/integration/security tests (every commit), Hurl HTTP contract tests (network gate), manual scaffold smoke tests.

---

## Test Layers

| Layer | Location | Tool | Gate | What it catches |
|---|---|---|---|---|
| **Unit** | `src/` inline `#[cfg(test)]` | `cargo test` | every commit | Logic bugs in parsing, hashing, path sanitisation |
| **Integration** | `tests/*_int_test.rs` | `cargo test` | every commit | Cross-module wiring, substituter fallback, manifest round-trips |
| **Security** | `tests/*_security_test.rs` | `cargo test` | every commit | Path traversal, null bytes, adversarial input |
| **HTTP contract** | `docs/5-testing/integration/*.hurl` | `hurl` | `network` | External API shape changes (narinfo format, channel endpoints) |
| **Scaffold smoke** | Manual | `pkg new` + `pkg rootfs build` | pre-release | Generated packages.toml produces a bootable ext4 image |

---

## Running Tests

```bash
# All Rust tests (every-commit layers)
cargo test

# Single crate
cargo test -p swe_justpkg_nix

# HTTP contract tests (requires network + hurl)
bash docs/5-testing/integration/harness.sh

# Specific test only
bash docs/5-testing/integration/harness.sh --filter nix_narinfo

# Verbose output on failure
bash docs/5-testing/integration/harness.sh --verbose

# With local Attic token
bash docs/5-testing/integration/harness.sh --attic-token "$MY_TOKEN"
```

The harness auto-installs missing prerequisites (hurl, jq, curl) on first run.
Pass `--no-install` to abort instead of auto-installing.

---

## What the Hurl Tests Cover

The `pkg` binary makes three kinds of HTTP requests that can break silently
if an external service changes its API shape:

| Request | URL pattern | Hurl file |
|---|---|---|
| Narinfo fetch | `<cache>/<hash>.narinfo` | `nix_narinfo_contract.hurl` |
| NAR download | `<cache>/<URL from narinfo>` | `nix_nar_download.hurl` |
| Channel revision | `<channel_base>/<channel>/git-revision` | `nix_channel_revision.hurl` |
| Store paths index | `<channel_base>/<channel>/store-paths.xz` | `nix_channel_store_paths.hurl` |
| Attic cache miss | `<attic_url>/<absent-hash>.narinfo` | `attic_substituter.hurl` |
| Attic cache hit | `<attic_url>/<present-hash>.narinfo` | `attic_narinfo_positive.hurl` |

`nix_nar_download.hurl` is a two-step test: it captures the `URL` field from a live
narinfo response, then downloads and validates the actual NAR. This exercises the exact
URL-joining logic pkg uses — a malformed join would produce a 404 here before any
real install is attempted.

These are **read-only contract tests** — they assert response shape, not
content. They run against the live cache and are skipped in CI unless the
`network` label is set on the workflow.

---

## Per-Crate Coverage

| Crate | Unit | Integration | Security | Notes |
|---|---|---|---|---|
| `pkg` | ✅ path sanitiser, error types | ✅ HTTP client retry | ✅ path traversal | `UreqClient` network tests ignored in CI |
| `nix` | ✅ NAR parser, hash encoding | ✅ fetch pipeline, substituter fallback | ✅ NAR path escape, hash tamper | Fetcher tests use in-process stubs |
| `resolve` | ✅ store path matching | ✅ manifest round-trip | — | Network fetch mocked via `HttpClient` trait |
| `vminit` | — | ✅ install packages, root layout | ✅ adversarial package names | |
| `config` | ✅ XDG loader, TOML parsing | — | — | |
| `cli` | ✅ scaffold generation | — | — | `pkg new` tested via tempdir |
| `rootfs` (planned #6) | TOML schema parsing | rootfs build end-to-end | path traversal in file entries | pending `pkg rootfs build` implementation |

---

## Fake-Test Policy

A test that cannot fail is deleted, not kept.

Before writing any test, answer: **what bug would this catch?**

- Every test must exercise the feature it claims to test
- Negative tests are mandatory for security features (path sanitisation, hash verification)
- If a test passes without the implementation existing, delete it

See global CLAUDE.md §1 for the full policy.

---

## Adding New Tests

### Rust test
Follow the existing `test_<action>_<condition>_<expectation>` naming convention
and register the file in the crate's `Cargo.toml` as a `[[test]]` entry if it
lives outside `src/`.

### Hurl test
Add a `.hurl` file to `docs/5-testing/integration/` and register it in `harness.sh`.
Each file must open with a `# Flow:` block that narrates the exact sequence of HTTP
calls `pkg` makes that this test validates, followed by a `# Breaks if:` block listing
what real failures the test would catch. Without both blocks the test is documentation-free
and will be rejected in review.

---

## CI Integration

```
Every commit:   cargo test (unit + integration + security)
network gate:   bash docs/5-testing/integration/harness.sh
pre-release:    pkg new <name> && pkg rootfs build packages/<name>/packages.toml
```
