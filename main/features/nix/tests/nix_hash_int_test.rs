// Tests for Nix hash encoding conversions.
// Nix uses three distinct encodings (SRI base64, Nix base-32, hex) in different
// contexts. Getting them wrong causes silent 404s against cache.nixos.org.
use swe_justpkg_nix::{nix_base32_to_hex, sri_to_hex, NixFetchError};

// sha256-AAAA...= is 32 zero bytes encoded as standard base64
const ZERO_SRI: &str = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const ZERO_HEX: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn test_sri_to_hex_zero_hash_produces_correct_hex() {
    let result = sri_to_hex(ZERO_SRI).unwrap();
    assert_eq!(result, ZERO_HEX);
}

#[test]
fn test_sri_to_hex_output_is_64_chars_for_sha256() {
    let result = sri_to_hex(ZERO_SRI).unwrap();
    assert_eq!(result.len(), 64, "SHA-256 hex must be 64 characters");
}

#[test]
fn test_sri_to_hex_rejects_missing_algorithm_prefix() {
    let result = sri_to_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    assert!(
        result.is_err(),
        "bare base64 without 'sha256-' prefix must be rejected"
    );
    assert!(matches!(
        result.unwrap_err(),
        NixFetchError::InvalidNixHash(_)
    ));
}

#[test]
fn test_sri_to_hex_rejects_wrong_algorithm() {
    let result = sri_to_hex("sha512-AAAA");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NixFetchError::InvalidNixHash(_)
    ));
}

#[test]
fn test_sri_to_hex_rejects_invalid_base64_chars() {
    let result = sri_to_hex("sha256-!!!not-valid-base64!!!");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        NixFetchError::InvalidNixHash(_)
    ));
}

#[test]
fn test_nix_base32_to_hex_all_zeros_is_valid() {
    // 32 '0' chars in Nix base-32 alphabet — all zero bits
    let result = nix_base32_to_hex("00000000000000000000000000000000");
    assert!(result.is_ok(), "32 zero chars must decode: {:?}", result);
    let hex = result.unwrap();
    assert!(
        hex.chars().all(|c| c.is_ascii_hexdigit()),
        "result must be lowercase hex"
    );
}

#[test]
fn test_nix_base32_to_hex_rejects_invalid_char() {
    // 'e' is not in the Nix base-32 alphabet (alphabet skips 'e', 'o', 't', 'u')
    let result = nix_base32_to_hex("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    assert!(result.is_err(), "'e' is not a valid Nix base-32 character");
    assert!(matches!(
        result.unwrap_err(),
        NixFetchError::InvalidNixHash(_)
    ));
}
