use swe_justpkg_nix::{NarInfo, NixFetchError};

const VALID_NARINFO: &str = "\
StorePath: /nix/store/abc123-hello-2.12
URL: nar/abc123.nar.xz
Compression: xz
FileHash: sha256:deadbeef
FileSize: 1234
NarHash: sha256:cafebabe
NarSize: 5678
References: abc123-hello-2.12
";

#[test]
fn test_parse_valid_narinfo_succeeds() {
    let info = NarInfo::parse("abc123", VALID_NARINFO).unwrap();
    assert_eq!(info.url, "nar/abc123.nar.xz");
    assert_eq!(info.file_hash, "deadbeef");
    assert_eq!(info.nar_hash, "cafebabe");
    assert_eq!(info.file_size, 1234);
    assert_eq!(info.nar_size, 5678);
    assert_eq!(info.store_path, "/nix/store/abc123-hello-2.12");
}

#[test]
fn test_parse_missing_url_returns_err() {
    let text = "StorePath: /nix/store/abc123-hello-2.12\nCompression: xz\n";
    let result = NarInfo::parse("abc123", text);
    assert!(result.is_err(), "missing URL must be rejected");
    match result.unwrap_err() {
        NixFetchError::NarInfoParse { message, .. } => {
            assert!(message.contains("URL"), "error must mention missing field");
        }
        e => panic!("expected NarInfoParse, got {e:?}"),
    }
}

#[test]
fn test_parse_unknown_compression_returns_err() {
    let text = "StorePath: /nix/store/abc\nURL: nar/x.nar.lz4\nCompression: lz4\n";
    let result = NarInfo::parse("abc", text);
    assert!(result.is_err(), "unknown compression must be rejected");
    match result.unwrap_err() {
        NixFetchError::NarInfoParse { message, .. } => {
            assert!(message.contains("lz4"));
        }
        e => panic!("expected NarInfoParse, got {e:?}"),
    }
}

#[test]
fn test_parse_strips_sha256_prefix_from_hashes() {
    let info = NarInfo::parse("abc123", VALID_NARINFO).unwrap();
    assert!(
        !info.file_hash.starts_with("sha256:"),
        "sha256: prefix must be stripped"
    );
    assert!(
        !info.nar_hash.starts_with("sha256:"),
        "sha256: prefix must be stripped"
    );
}

#[test]
fn test_parse_empty_references_is_valid() {
    let text = "StorePath: /nix/store/abc\nURL: nar/x.nar.xz\nReferences: \n";
    let info = NarInfo::parse("abc", text).unwrap();
    assert!(
        info.references.is_empty(),
        "empty References field must produce empty vec"
    );
}
