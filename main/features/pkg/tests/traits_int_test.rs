use std::path::Path;
use swe_justpkg_pkg::safe_path_join;

#[test]
fn test_safe_path_join_normal_path_resolves() {
    let base = Path::new("/tmp/out");
    let result = safe_path_join(base, "usr/lib/libfoo.so").unwrap();
    assert_eq!(result, base.join("usr/lib/libfoo.so"));
}

#[test]
fn test_safe_path_join_leading_dot_slash_stripped() {
    let base = Path::new("/tmp/out");
    let result = safe_path_join(base, "./usr/bin/sh").unwrap();
    assert_eq!(result, base.join("usr/bin/sh"));
}

#[test]
fn test_safe_path_join_rejects_dotdot_traversal() {
    let base = Path::new("/tmp/out");
    assert!(safe_path_join(base, "../etc/passwd").is_err());
}

#[test]
fn test_safe_path_join_rejects_absolute_path() {
    let base = Path::new("/tmp/out");
    assert!(safe_path_join(base, "/etc/passwd").is_err());
}

#[test]
fn test_safe_path_join_rejects_windows_drive_prefix() {
    let base = Path::new("/tmp/out");
    assert!(safe_path_join(base, "C:/windows/system32").is_err());
}

#[test]
fn test_safe_path_join_rejects_null_byte() {
    let base = Path::new("/tmp/out");
    assert!(safe_path_join(base, "usr/lib/foo\0bar").is_err());
}

#[test]
fn test_safe_path_join_empty_components_skipped() {
    let base = Path::new("/tmp/out");
    let result = safe_path_join(base, "usr//lib///libfoo.so").unwrap();
    assert_eq!(result, base.join("usr/lib/libfoo.so"));
}
