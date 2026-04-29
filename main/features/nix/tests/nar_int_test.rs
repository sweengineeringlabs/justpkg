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
    write_nar_str(&mut buf, ")"); // close node
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

/// Builds a directory NAR whose root contains one regular file entry.
fn build_dir_nar(entry_name: &str, content: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "directory");
    write_nar_str(&mut buf, "entry");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "name");
    write_nar_str(&mut buf, entry_name);
    write_nar_str(&mut buf, "node");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "regular");
    write_nar_str(&mut buf, "contents");
    let len = content.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(content);
    let pad = (8 - (content.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
    write_nar_str(&mut buf, ")"); // close regular node
    write_nar_str(&mut buf, ")"); // close entry group
    write_nar_str(&mut buf, ")"); // close directory node
    buf
}

#[test]
fn test_extract_nar_directory_root_writes_file_inside_dest() {
    // Real nixpkgs packages have a directory as the NAR root. This test
    // would fail before the fix because the extractor double-consumed ")".
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("pkg");
    let nar = build_dir_nar("bin/psql", b"pg content");
    extract_nar(std::io::Cursor::new(nar), &dest).unwrap();
    let expected = dest.join("bin").join("psql");
    assert!(expected.exists(), "file must be at dest/bin/psql");
    assert_eq!(std::fs::read(&expected).unwrap(), b"pg content");
}

#[test]
fn test_extract_nar_directory_with_multiple_entries() {
    // Verify the entry loop in read_directory handles multiple files correctly.
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "directory");
    for (name, content) in [("a.txt", b"aaa" as &[u8]), ("b.txt", b"bbb")] {
        write_nar_str(&mut buf, "entry");
        write_nar_str(&mut buf, "(");
        write_nar_str(&mut buf, "name");
        write_nar_str(&mut buf, name);
        write_nar_str(&mut buf, "node");
        write_nar_str(&mut buf, "(");
        write_nar_str(&mut buf, "type");
        write_nar_str(&mut buf, "regular");
        write_nar_str(&mut buf, "contents");
        let len = content.len() as u64;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(content);
        let pad = (8 - (content.len() % 8)) % 8;
        buf.extend(std::iter::repeat(0u8).take(pad));
        write_nar_str(&mut buf, ")"); // close regular node
        write_nar_str(&mut buf, ")"); // close entry group
    }
    write_nar_str(&mut buf, ")"); // close directory node

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("pkg");
    extract_nar(std::io::Cursor::new(buf), &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"aaa");
    assert_eq!(std::fs::read(dest.join("b.txt")).unwrap(), b"bbb");
}

#[cfg(unix)]
#[test]
fn test_extract_nar_symlink_node_creates_symlink() {
    // Symlinks are ubiquitous in nixpkgs closures (bin/, lib/ entries).
    // This test would fail if read_symlink doesn't consume ")".
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "symlink");
    write_nar_str(&mut buf, "target");
    write_nar_str(&mut buf, "../lib/libpq.so.5");
    write_nar_str(&mut buf, ")"); // close symlink node

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("libpq.so");
    extract_nar(std::io::Cursor::new(buf), &dest).unwrap();
    let target = std::fs::read_link(&dest).unwrap();
    assert_eq!(target.to_str().unwrap(), "../lib/libpq.so.5");
}

#[test]
fn test_extract_nar_rejects_unknown_node_type() {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "hardlink"); // not a valid NAR node type
    write_nar_str(&mut buf, ")");
    let dir = tempfile::tempdir().unwrap();
    let result = extract_nar(std::io::Cursor::new(buf), dir.path());
    assert!(result.is_err(), "unknown node type must be rejected");
}
