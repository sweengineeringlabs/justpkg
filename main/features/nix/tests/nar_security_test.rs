// Security tests for NAR extraction.
// A malicious NAR could contain path traversal entries; extract_nar must reject them.
// These tests construct adversarial NAR byte sequences and verify rejection.
use swe_justpkg_nix::extract_nar;

fn write_nar_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    let pad = (8 - (s.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

/// Builds a directory NAR with one entry whose name is `entry_name`.
fn build_dir_nar_with_entry(entry_name: &str, content: &[u8]) -> Vec<u8> {
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
    write_nar_str(&mut buf, ")"); // [A] read_regular exits its loop
    write_nar_str(&mut buf, ")"); // [B] inner read_nar_node closes
    write_nar_str(&mut buf, ")"); // [C] read_directory entry group closes
    write_nar_str(&mut buf, ")"); // [D] read_directory loop exits
    write_nar_str(&mut buf, ")"); // [E] outer read_nar_node closes
    buf
}

#[test]
fn test_extract_nar_rejects_dotdot_traversal_entry() {
    let nar = build_dir_nar_with_entry("..", b"evil");
    let dir = tempfile::tempdir().unwrap();
    let result = extract_nar(std::io::Cursor::new(nar), dir.path());
    assert!(result.is_err(), "'..' directory entry must be rejected");
}

#[test]
fn test_extract_nar_rejects_absolute_path_entry() {
    let nar = build_dir_nar_with_entry("/etc/cron.d/evil", b"evil");
    let dir = tempfile::tempdir().unwrap();
    // The name contains '/' so safe_path_join splits it; the leading '/' produces
    // an empty first component which is skipped — but the leading slash check fires.
    let result = extract_nar(std::io::Cursor::new(nar), dir.path());
    assert!(result.is_err(), "absolute path entry must be rejected");
}

#[test]
fn test_extract_nar_normal_entry_does_not_escape_dest() {
    let nar = build_dir_nar_with_entry("subdir/file.txt", b"safe");
    let dir = tempfile::tempdir().unwrap();
    extract_nar(std::io::Cursor::new(nar), dir.path()).unwrap();
    let expected = dir.path().join("subdir").join("file.txt");
    assert!(expected.exists(), "normal entry must be written inside dest");
}
