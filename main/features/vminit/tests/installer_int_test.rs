// Integration tests for install_packages.
//
// Network I/O is replaced by an in-process TestHttpClient that either fails
// immediately or records calls.  This lets us verify the logic of the installer
// (manifest lookup, error propagation, early-exit on first failure) without any
// real network dependency.
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use swe_justpkg_vminit::{install_packages, parse_manifest, VminitInstallError};

// ── Test stubs ──────────────────────────────────────────────────────────────

/// Always returns an HTTP error so NixFetcher never reaches NAR extraction.
struct FailingHttpClient;

impl justpkg_pkg::HttpClient for FailingHttpClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
}

/// Counts how many times `get_bytes` is called so we can assert that the
/// installer exits early on the first error and does not continue to the next
/// package.
struct CountingHttpClient {
    call_count: Arc<AtomicU32>,
}

impl justpkg_pkg::HttpClient for CountingHttpClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

const TWO_ENTRY_MANIFEST: &str = r#"{
    "packages": {
        "curl": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "git":  "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
    }
}"#;

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_install_packages_empty_names_list_succeeds_without_calling_http() {
    // An empty names slice must return Ok without ever touching the HTTP client.
    // This proves the installer does not speculatively fetch when there is nothing to do.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &[], dir.path());

    assert!(result.is_ok(), "empty names list must succeed");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "HTTP must not be called when names list is empty"
    );
}

#[test]
fn test_install_packages_name_not_in_manifest_returns_package_not_found() {
    // A name that is absent from the manifest must return PackageNotFound
    // *before* any HTTP call is made.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["unknown-pkg"], dir.path());

    assert!(result.is_err(), "missing name must return an error");
    assert!(
        matches!(
            result.unwrap_err(),
            VminitInstallError::PackageNotFound { name } if name == "unknown-pkg"
        ),
        "error variant must be PackageNotFound with the correct name"
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "HTTP must not be called when the manifest lookup fails"
    );
}

#[test]
fn test_install_packages_valid_entry_reaches_http_and_propagates_fetch_error() {
    // When the manifest lookup succeeds the installer must call the HTTP client.
    // Here the client always fails, so we get FetchFailed.  This proves the code
    // path from manifest lookup → NixFetcher → error propagation is wired correctly.
    let http = FailingHttpClient;
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl"], dir.path());

    assert!(result.is_err(), "HTTP failure must propagate as error");
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::FetchFailed { name, .. } if name == "curl"),
        "error variant must be FetchFailed with name=curl"
    );
}

#[test]
fn test_install_packages_stops_on_first_error() {
    // Given two valid packages, the installer must stop at the first HTTP failure
    // and not attempt the second package.  We count HTTP calls: must be exactly 1.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl", "git"], dir.path());

    assert!(
        result.is_err(),
        "first HTTP failure must abort the whole call"
    );
    // NixFetcher calls get_bytes once per package (for the narinfo).
    // Because we stop on first error, exactly 1 HTTP call must happen.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "installer must stop after first fetch failure, not continue to second package"
    );
}
