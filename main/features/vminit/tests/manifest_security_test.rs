// Security tests for parse_manifest.
// Malformed, adversarial, and boundary inputs must never silently produce a
// usable manifest — they must return an explicit error.
use swe_justpkg_vminit::{parse_manifest, VminitInstallError};

#[test]
fn test_parse_manifest_malformed_json_returns_manifest_parse_error() {
    let result = parse_manifest("{not valid json at all");
    assert!(
        result.is_err(),
        "malformed JSON must not silently produce a manifest"
    );
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::ManifestParse(_)),
        "error variant must be ManifestParse"
    );
}

#[test]
fn test_parse_manifest_missing_packages_key_returns_manifest_parse_error() {
    // A valid JSON object that lacks the "packages" key is not a valid manifest.
    let result = parse_manifest(r#"{"version": 1, "other": {}}"#);
    assert!(
        result.is_err(),
        "JSON without \"packages\" key must be rejected"
    );
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::ManifestParse(_)),
        "error variant must be ManifestParse"
    );
}

#[test]
fn test_parse_manifest_empty_string_returns_error() {
    // An empty string is not valid JSON.
    let result = parse_manifest("");
    assert!(result.is_err(), "empty string must not produce a manifest");
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::ManifestParse(_)),
        "error variant must be ManifestParse"
    );
}

#[test]
fn test_parse_manifest_packages_value_not_object_returns_error() {
    // "packages" must be a JSON object; arrays and scalars must be rejected.
    for bad in &[
        r#"{"packages": ["curl", "git"]}"#,
        r#"{"packages": "curl"}"#,
        r#"{"packages": 42}"#,
        r#"{"packages": null}"#,
    ] {
        let result = parse_manifest(bad);
        assert!(
            result.is_err(),
            "packages={bad:?} must be rejected — only objects are valid"
        );
        assert!(
            matches!(result.unwrap_err(), VminitInstallError::ManifestParse(_)),
            "error variant must be ManifestParse for input {bad:?}"
        );
    }
}

#[test]
fn test_parse_manifest_entry_with_non_string_hash_returns_error() {
    // A package whose value is not a string (e.g. a number or nested object)
    // must not silently produce an entry with a garbage hash.
    let bad = r#"{"packages": {"curl": 12345}}"#;
    let result = parse_manifest(bad);
    assert!(result.is_err(), "non-string hash value must be rejected");
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::ManifestParse(_)),
        "error variant must be ManifestParse"
    );
}
