# Contributing to justpkg

## Branch model

Six-branch flow: `dev` → `test` → `int` → `uat` → `prd` → `main`. PRs target `dev`. Feature branches branch from `dev`.

## Before opening a PR

```bash
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

All three must pass. CI enforces `RUSTFLAGS=-D warnings`.

## Test conventions

Every test must follow `test_<action>_<condition>_<expectation>` naming. Tests must be able to fail — no tautological assertions. See `core/tests/safe_path_join_test.rs` for examples.

## Adding a new package backend

1. Create a new crate under `main/features/<backend>/` (e.g., `apk`, `apt`)
2. Implement the `HttpClient` + `Extractor` traits from `justpkg-core`
3. Add integration tests that hit a real local package index (no mocks)
4. Wire the new backend into `justpkg-cli` behind a subcommand flag

## Reporting bugs

Open a GitHub issue. For unimplemented v0 features, use the `v0-gap` label.

## License

By contributing you agree your contributions will be licensed under [Apache-2.0](LICENSE).
