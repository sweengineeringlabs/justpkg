use swe_justpkg_nix::NixFetchError;

#[test]
fn test_nix_fetch_error_flake_lock_parse_includes_message() {
    let e = NixFetchError::FlakeLockParse("unexpected token at line 3".to_string());
    assert!(e.to_string().contains("unexpected token at line 3"));
}

#[test]
fn test_nix_fetch_error_invalid_hash_includes_input() {
    let e = NixFetchError::InvalidNixHash("not-a-nix-hash".to_string());
    assert!(e.to_string().contains("not-a-nix-hash"));
}

#[test]
fn test_nix_fetch_error_nar_extract_includes_detail() {
    let e = NixFetchError::NarExtract("unexpected EOF at offset 42".to_string());
    assert!(e.to_string().contains("unexpected EOF at offset 42"));
}

#[test]
fn test_nix_fetch_error_narinfo_parse_includes_hash_and_message() {
    let e = NixFetchError::NarInfoParse {
        hash: "abc123def456".to_string(),
        message: "missing URL field".to_string(),
    };
    let msg = e.to_string();
    assert!(msg.contains("abc123def456"));
    assert!(msg.contains("missing URL field"));
}
