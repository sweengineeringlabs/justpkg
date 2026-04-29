// Integration tests for UreqClient. Tests marked #[ignore] require network
// access and are skipped by default in CI. Run with: cargo test -- --include-ignored
use swe_justpkg_pkg::UreqClient;
use swe_justpkg_pkg::HttpClient;

#[test]
#[ignore = "requires network"]
fn test_get_bytes_real_url_returns_nonempty_body() {
    let client = UreqClient;
    let bytes = client.get_bytes("https://cache.nixos.org/nix-cache-info").unwrap();
    assert!(!bytes.is_empty(), "cache info should not be empty");
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("StoreDir"), "cache info must contain StoreDir field");
}

#[test]
#[ignore = "requires network"]
fn test_get_stream_real_url_writes_bytes_and_returns_count() {
    let client = UreqClient;
    let mut buf = Vec::new();
    let n = client.get_stream("https://cache.nixos.org/nix-cache-info", &mut buf).unwrap();
    assert!(n > 0, "byte count must be positive");
    assert_eq!(n as usize, buf.len(), "returned count must match bytes written");
}
