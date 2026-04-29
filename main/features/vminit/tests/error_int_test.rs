use swe_justpkg_vminit::VminitInstallError;

#[test]
fn test_manifest_parse_error_display_contains_message() {
    let e = VminitInstallError::ManifestParse("unexpected token".to_string());
    let s = e.to_string();
    assert!(
        s.contains("unexpected token"),
        "display must include the error text: {s}"
    );
}

#[test]
fn test_package_not_found_error_display_contains_name() {
    let e = VminitInstallError::PackageNotFound { name: "curl".to_string() };
    let s = e.to_string();
    assert!(s.contains("curl"), "display must include the package name: {s}");
}

#[test]
fn test_manifest_parse_error_is_debug_formattable() {
    let e = VminitInstallError::ManifestParse("oops".to_string());
    let _ = format!("{e:?}");
}

#[test]
fn test_package_not_found_error_is_debug_formattable() {
    let e = VminitInstallError::PackageNotFound { name: "git".to_string() };
    let _ = format!("{e:?}");
}
