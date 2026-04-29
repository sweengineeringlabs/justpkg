use swe_justpkg_pkg::JustpkgError;

#[test]
fn test_justpkg_error_http_includes_url_and_status() {
    let e = JustpkgError::Http { url: "https://cache.nixos.org/x.narinfo".to_string(), status: 404 };
    let msg = e.to_string();
    assert!(msg.contains("https://cache.nixos.org/x.narinfo"), "error must include URL");
    assert!(msg.contains("404"), "error must include status code");
}

#[test]
fn test_justpkg_error_unsafe_archive_path_includes_path() {
    let e = JustpkgError::UnsafeArchivePath { path: "../etc/passwd".to_string() };
    assert!(e.to_string().contains("../etc/passwd"), "error must include the rejected path");
}

#[test]
fn test_justpkg_error_parse_includes_context_and_message() {
    let e = JustpkgError::Parse { context: "narinfo".to_string(), message: "missing URL".to_string() };
    let msg = e.to_string();
    assert!(msg.contains("narinfo"));
    assert!(msg.contains("missing URL"));
}

#[test]
fn test_justpkg_error_hash_mismatch_includes_expected_and_actual() {
    let e = JustpkgError::HashMismatch {
        url: "https://example.com/x.nar".to_string(),
        expected: "deadbeef".to_string(),
        actual: "cafebabe".to_string(),
    };
    let msg = e.to_string();
    assert!(msg.contains("deadbeef"));
    assert!(msg.contains("cafebabe"));
}
