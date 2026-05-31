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
| **Scaffold smoke** | Manual | `pkg new` + `bash` | pre-release | Generated scripts are correct and runnable |

---

## Running Tests

```bash
# All Rust tests (every-commit layers)
cargo test

# Single crate
cargo test -p swe_justpkg_nix

# HTTP contract tests (requires network + hurl)
bash docs/5-testing/integration/run.sh

# Single Hurl file
hurl --test docs/5-testing/integration/nix_narinfo_contract.hurl
```

Install Hurl: https://hurl.dev/docs/installation.html

---

## What the Hurl Tests Cover

The `pkg` binary makes three kinds of HTTP requests that can break silently
if an external service changes its API shape:

| Request | URL pattern | Hurl file |
|---|---|---|
| Narinfo fetch | `<cache>/<hash>.narinfo` | `nix_narinfo_contract.hurl` |
| Channel revision | `<channel_base>/<channel>/git-revision` | `nix_channel_revision.hurl` |
| Store paths index | `<channel_base>/<channel>/store-paths.xz` | `nix_channel_store_paths.hurl` |
| Attic cache | `<attic_url>/<hash>.narinfo` | `attic_substituter.hurl` |

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
Add a `.hurl` file to `docs/5-testing/integration/`. Register it in `run.sh`
so it is discovered automatically. Each file must have a comment header
explaining which external contract it validates and what would break if the
test failed.

---

## CI Integration

```
Every commit:   cargo test (unit + integration + security)
network gate:   hurl --test docs/5-testing/integration/*.hurl
pre-release:    pkg new smoke test + pkg resolve + bash build-rootfs.sh
```
