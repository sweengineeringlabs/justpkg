// Integration tests for NixFetcher.
// Tests marked #[ignore] require network. Run with: cargo test -- --include-ignored
//
// The narinfo URL derivation (sri → hex → narinfo URL) is currently a known gap —
// see issue #80. These tests exercise the components that DO work.
use swe_justpkg_nix::{FlakeLock, NixFetcher, NixFetchError};
use justpkg_pkg::UreqClient;

const MINIMAL_FLAKE_LOCK: &str = r#"{
  "nodes": {
    "root": { "inputs": { "nixpkgs": "nixpkgs" } },
    "nixpkgs": {
      "locked": {
        "lastModified": 1700000000,
        "narHash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "NixOS", "repo": "nixpkgs",
        "rev": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "type": "github"
      },
      "inputs": {}
    }
  },
  "root": "root",
  "version": 7
}"#;

struct FailingClient;

impl justpkg_pkg::HttpClient for FailingClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })
    }
    fn get_stream(&self, url: &str, _: &mut dyn std::io::Write) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })
    }
}

#[test]
fn test_build_fails_on_unreachable_cache() {
    // Verifies the fetcher propagates HTTP errors rather than silently succeeding.
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let client = FailingClient;
    let fetcher = NixFetcher { http: &client };
    let result = fetcher.build(&lock, dir.path());
    assert!(result.is_err(), "HTTP failure must propagate as error");
    assert!(matches!(result.unwrap_err(), NixFetchError::Core(_)));
}

#[test]
#[ignore = "requires network — known to fail until narinfo URL derivation is fixed (issue #80)"]
fn test_build_real_flake_lock_fetches_nar() {
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let http = UreqClient;
    let fetcher = NixFetcher { http: &http };
    fetcher.build(&lock, dir.path()).unwrap();
}
