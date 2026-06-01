use swe_justpkg_rootfs::validate_vfs_path;

// ── Path traversal ────────────────────────────────────────────────────────────

#[test]
fn test_validate_vfs_path_rejects_dotdot_traversal() {
    let result = validate_vfs_path("/var/lib/../../../etc/shadow");
    assert!(result.is_err(), "path traversal via '..' must be rejected");
    assert!(result.unwrap_err().contains(".."));
}

#[test]
fn test_validate_vfs_path_rejects_dotdot_at_root() {
    assert!(validate_vfs_path("/../etc/shadow").is_err());
}

#[test]
fn test_validate_vfs_path_rejects_dotdot_at_end() {
    assert!(validate_vfs_path("/var/lib/..").is_err());
}

// ── Relative paths ────────────────────────────────────────────────────────────

#[test]
fn test_validate_vfs_path_rejects_relative_path() {
    let result = validate_vfs_path("bin/start");
    assert!(result.is_err(), "relative paths must be rejected");
    assert!(result.unwrap_err().contains("absolute"));
}

// ── Null bytes ────────────────────────────────────────────────────────────────

#[test]
fn test_validate_vfs_path_rejects_null_byte() {
    let result = validate_vfs_path("/bin/start\x00/../etc/shadow");
    assert!(result.is_err(), "null bytes must be rejected");
    assert!(result.unwrap_err().contains("null"));
}

// ── Valid paths ───────────────────────────────────────────────────────────────

#[test]
fn test_validate_vfs_path_accepts_absolute_paths() {
    assert!(validate_vfs_path("/bin/start").is_ok());
    assert!(validate_vfs_path("/var/lib/opensearch").is_ok());
    assert!(validate_vfs_path("/etc/passwd").is_ok());
    assert!(validate_vfs_path("/").is_ok());
}

#[test]
fn test_validate_vfs_path_accepts_names_containing_dots_but_not_dotdot() {
    // "..hidden" is a valid filename; only the bare ".." component is rejected.
    assert!(validate_vfs_path("/var/lib/..hidden").is_ok());
    assert!(validate_vfs_path("/etc/file..cfg").is_ok());
}
