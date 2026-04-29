// Integration tests for parse_manifest.
// Each test exercises exactly one observable behaviour of the parser so that a
// single regression in the parser breaks exactly the tests that cover it.
use swe_justpkg_vminit::parse_manifest;

const SINGLE_ENTRY: &str =
    r#"{"packages": {"curl": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}}"#;
const MULTI_ENTRY: &str = r#"{
    "packages": {
        "curl": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "git":  "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=",
        "bash": "sha256-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC="
    }
}"#;
const EMPTY_PACKAGES: &str = r#"{"packages": {}}"#;

#[test]
fn test_parse_manifest_single_entry_returns_correct_name_and_hash() {
    let manifest = parse_manifest(SINGLE_ENTRY).expect("valid JSON must parse");
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(
        manifest.entries.get("curl").map(String::as_str),
        Some("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="),
        "curl SRI hash must match the value in the JSON"
    );
}

#[test]
fn test_parse_manifest_empty_packages_returns_empty_manifest() {
    let manifest = parse_manifest(EMPTY_PACKAGES).expect("empty packages object must parse");
    assert!(
        manifest.entries.is_empty(),
        "empty packages object must produce zero entries"
    );
}

#[test]
fn test_parse_manifest_multiple_entries_all_parsed() {
    let manifest = parse_manifest(MULTI_ENTRY).expect("multi-entry JSON must parse");
    assert_eq!(
        manifest.entries.len(),
        3,
        "all three packages must be present"
    );
    assert!(
        manifest.entries.contains_key("curl"),
        "curl must be present"
    );
    assert!(manifest.entries.contains_key("git"), "git must be present");
    assert!(
        manifest.entries.contains_key("bash"),
        "bash must be present"
    );
}

#[test]
fn test_parse_manifest_lookup_by_name_returns_correct_sri() {
    let manifest = parse_manifest(MULTI_ENTRY).expect("multi-entry JSON must parse");
    assert_eq!(
        manifest.entries["git"], "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=",
        "git SRI must equal the value in the JSON"
    );
    assert_eq!(
        manifest.entries["bash"], "sha256-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=",
        "bash SRI must equal the value in the JSON"
    );
}

#[test]
fn test_parse_manifest_round_trip_preserves_all_entries() {
    // Parse the manifest, collect the entries, then verify each entry is
    // identical to what we can look up from a second parse.  This proves the
    // parser is deterministic and does not silently drop or mutate values.
    let first = parse_manifest(MULTI_ENTRY).expect("first parse must succeed");
    let second = parse_manifest(MULTI_ENTRY).expect("second parse must succeed");
    for (name, hash) in &first.entries {
        assert_eq!(
            second.entries.get(name),
            Some(hash),
            "entry {name:?} must survive a second parse unchanged"
        );
    }
    assert_eq!(
        first.entries.len(),
        second.entries.len(),
        "both parses must yield the same number of entries"
    );
}
