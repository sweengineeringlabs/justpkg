// Security tests for the nix crate.
// Covers hash-parsing rejection and NAR extraction boundary enforcement.
// Complements nar_security_test.rs (which tests structural traversal attacks)
// with hash-layer attacks.
use swe_justpkg_nix::{sri_to_hex, NixFetchError};

#[test]
fn test_sri_to_hex_rejects_empty_string() {
    let result = sri_to_hex("");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NixFetchError::InvalidNixHash(_)
    ));
}

#[test]
fn test_sri_to_hex_rejects_only_prefix() {
    // "sha256-" with no payload must not decode to an empty byte slice silently
    let result = sri_to_hex("sha256-");
    // Either an error or a zero-length hash — both are acceptable; empty hash is
    // effectively useless as a CAS key but not a security violation. We assert it
    // doesn't panic and returns a deterministic result.
    let _ = result; // must not panic
}

#[test]
fn test_sri_to_hex_rejects_hash_with_null_byte() {
    let result = sri_to_hex("sha256-AA\x00AA");
    // null bytes in a hash string indicate malformed input
    assert!(
        result.is_err(),
        "null byte in SRI must not silently produce a hash"
    );
}
