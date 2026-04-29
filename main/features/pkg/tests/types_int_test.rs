use swe_justpkg_pkg::{PackageSpec, VersionConstraint};

#[test]
fn test_version_constraint_any_displays_as_star() {
    assert_eq!(VersionConstraint::Any.to_string(), "*");
}

#[test]
fn test_version_constraint_eq_displays_with_equals_prefix() {
    assert_eq!(VersionConstraint::Eq("1.2.3".to_string()).to_string(), "=1.2.3");
}

#[test]
fn test_version_constraint_gte_displays_correctly() {
    assert_eq!(VersionConstraint::Gte("2.0".to_string()).to_string(), ">=2.0");
}

#[test]
fn test_version_constraint_lt_displays_correctly() {
    assert_eq!(VersionConstraint::Lt("3.0".to_string()).to_string(), "<3.0");
}

#[test]
fn test_package_spec_equality_holds_for_identical_values() {
    let a = PackageSpec { name: "curl".to_string(), constraint: VersionConstraint::Any };
    let b = PackageSpec { name: "curl".to_string(), constraint: VersionConstraint::Any };
    assert_eq!(a, b);
}

#[test]
fn test_package_spec_inequality_when_names_differ() {
    let a = PackageSpec { name: "curl".to_string(), constraint: VersionConstraint::Any };
    let b = PackageSpec { name: "wget".to_string(), constraint: VersionConstraint::Any };
    assert_ne!(a, b);
}
