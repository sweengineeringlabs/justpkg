// Tests for NAR extraction. Constructs minimal valid NAR byte sequences
// in-process — no fixture files needed.
use swe_justpkg_nix::extract_nar;

/// Writes a length-prefixed, 8-byte-aligned NAR string field.
fn write_nar_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    let pad = (8 - (s.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

/// Builds a minimal NAR containing one regular file at the root.
///
/// Closing paren accounting (extractor's `read_regular` consumes ")" from the
/// stream to exit its loop, then `read_nar_node` consumes a second ")"):
///   ")" [A]  — read_regular exits
///   ")" [B]  — read_nar_node closes the node
fn build_single_file_nar(content: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "regular");
    write_nar_str(&mut buf, "contents");
    let len = content.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(content);
    let pad = (8 - (content.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
    write_nar_str(&mut buf, ")"); // [A] read_regular exits
    write_nar_str(&mut buf, ")"); // [B] read_nar_node closes
    buf
}

#[test]
fn test_extract_nar_single_file_writes_content() {
    let dir = tempfile::tempdir().unwrap();
    // dest must not be an existing directory — the regular node writes a file AT dest
    let dest = dir.path().join("output_file");
    let nar = build_single_file_nar(b"hello world");
    extract_nar(std::io::Cursor::new(nar), &dest).unwrap();
    let content = std::fs::read(&dest).unwrap();
    assert_eq!(content, b"hello world");
}

#[test]
fn test_extract_nar_rejects_wrong_magic() {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "not-a-nar");
    let dir = tempfile::tempdir().unwrap();
    let result = extract_nar(std::io::Cursor::new(buf), dir.path());
    assert!(result.is_err(), "wrong magic must be rejected");
}

#[test]
fn test_extract_nar_rejects_unknown_node_type() {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "hardlink"); // not a valid NAR node type
    write_nar_str(&mut buf, ")");
    write_nar_str(&mut buf, ")");
    let dir = tempfile::tempdir().unwrap();
    let result = extract_nar(std::io::Cursor::new(buf), dir.path());
    assert!(result.is_err(), "unknown node type must be rejected");
}
