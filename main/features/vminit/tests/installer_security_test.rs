// Security tests for install_packages.
//
// Adversarial package names (path separators, null bytes, huge strings) must
// either be rejected via PackageNotFound (because they are not in the manifest)
// or handled without panicking.  HTTP errors must surface as FetchFailed, never
// silently swallowed.
use swe_justpkg_vminit::{install_packages, parse_manifest, VminitInstallError};

// ── Test stub ────────────────────────────────────────────────────────────────

struct ErrorHttpClient;

impl justpkg_pkg::HttpClient for ErrorHttpClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::PkgError> {
        Err(justpkg_pkg::PkgError::Http {
            url: url.to_string(),
            status: 500,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::PkgError> {
        Err(justpkg_pkg::PkgError::Http {
            url: url.to_string(),
            status: 500,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

const SINGLE_ENTRY_MANIFEST: &str =
    r#"{"packages": {"curl": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}}"#;
const EMPTY_MANIFEST: &str = r#"{"packages": {}}"#;

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_install_packages_name_with_path_separator_is_rejected_as_not_found() {
    // Names like "../etc/passwd" or "a/b" are not in the manifest, so the installer
    // must return PackageNotFound.  It must never attempt to use the separator as
    // a path component that could escape the destination directory.
    let http = ErrorHttpClient;
    let manifest = parse_manifest(SINGLE_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    for dangerous in &["../etc/passwd", "curl/../evil", "a/b/c", "/abs/path"] {
        let result = install_packages(&http, &manifest, &[dangerous], dir.path(), &[]);
        assert!(
            result.is_err(),
            "name {dangerous:?} must not succeed — it is not in the manifest"
        );
        assert!(
            matches!(
                result.unwrap_err(),
                VminitInstallError::PackageNotFound { .. }
            ),
            "adversarial name {dangerous:?} must yield PackageNotFound, not a panic or path escape"
        );
    }
}

#[test]
fn test_install_packages_http_error_propagates_as_fetch_failed() {
    // The HTTP stub always returns 500.  Once the manifest lookup succeeds,
    // the installer must surface the HTTP error as FetchFailed, not swallow it.
    let http = ErrorHttpClient;
    let manifest = parse_manifest(SINGLE_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl"], dir.path(), &[]);

    assert!(result.is_err(), "HTTP 500 must propagate as error");
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::FetchFailed { .. }),
        "HTTP error must surface as FetchFailed"
    );
}

#[test]
fn test_install_packages_empty_manifest_with_nonempty_names_returns_not_found() {
    // An empty manifest has no packages.  Any requested name must be PackageNotFound.
    let http = ErrorHttpClient;
    let manifest = parse_manifest(EMPTY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl"], dir.path(), &[]);

    assert!(result.is_err(), "name not in empty manifest must fail");
    assert!(
        matches!(
            result.unwrap_err(),
            VminitInstallError::PackageNotFound { name } if name == "curl"
        ),
        "error must be PackageNotFound for name not in empty manifest"
    );
}

#[test]
fn test_install_packages_very_long_name_does_not_panic() {
    // A very long package name must not cause a stack overflow or panic.
    // It is not in the manifest, so PackageNotFound is the expected outcome.
    let http = ErrorHttpClient;
    let manifest = parse_manifest(SINGLE_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let long_name = "a".repeat(65_536);

    let result = install_packages(&http, &manifest, &[long_name.as_str()], dir.path(), &[]);

    assert!(
        result.is_err(),
        "very long name must fail — it is not in the manifest"
    );
    assert!(
        matches!(
            result.unwrap_err(),
            VminitInstallError::PackageNotFound { .. }
        ),
        "very long name must yield PackageNotFound, not a panic"
    );
}

#[test]
fn test_install_packages_name_with_null_byte_is_not_in_manifest() {
    // Null bytes in a package name are not valid in any manifest entry, so the
    // lookup must return PackageNotFound without any attempt to use the name as
    // a filesystem path.
    let http = ErrorHttpClient;
    let manifest = parse_manifest(SINGLE_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    // Rust &str can contain embedded NUL bytes; the manifest HashMap uses plain
    // String keys so the lookup will simply miss.
    let name_with_null = "curl\x00evil";
    let result = install_packages(&http, &manifest, &[name_with_null], dir.path(), &[]);

    assert!(
        result.is_err(),
        "name with null byte must not be found in manifest"
    );
    assert!(
        matches!(
            result.unwrap_err(),
            VminitInstallError::PackageNotFound { .. }
        ),
        "null-byte name must yield PackageNotFound"
    );
}
