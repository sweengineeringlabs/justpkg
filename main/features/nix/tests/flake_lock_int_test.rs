use swe_justpkg_nix::FlakeLock;

const MINIMAL_FLAKE_LOCK: &str = r#"{
  "nodes": {
    "root": {
      "inputs": { "nixpkgs": "nixpkgs" }
    },
    "nixpkgs": {
      "locked": {
        "lastModified": 1700000000,
        "narHash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "NixOS",
        "repo": "nixpkgs",
        "rev": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "type": "github"
      },
      "inputs": {}
    }
  },
  "root": "root",
  "version": 7
}"#;

#[test]
fn test_from_json_parses_valid_flake_lock() {
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    assert_eq!(lock.version, 7);
    assert_eq!(lock.root, "root");
    assert!(lock.nodes.contains_key("nixpkgs"));
}

#[test]
fn test_locked_nodes_excludes_root_node() {
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let names: Vec<&str> = lock.locked_nodes().map(|(k, _)| k.as_str()).collect();
    assert!(!names.contains(&"root"), "root node must not appear in locked_nodes()");
    assert!(names.contains(&"nixpkgs"), "nixpkgs locked node must appear");
}

#[test]
fn test_locked_nodes_nar_hash_matches_input() {
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let (_, node) = lock.locked_nodes().next().unwrap();
    assert_eq!(
        node.locked.nar_hash,
        "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    );
}

#[test]
fn test_from_json_rejects_malformed_json() {
    let result = FlakeLock::from_json("{not valid json");
    assert!(result.is_err(), "malformed JSON must be rejected");
}

#[test]
fn test_from_json_rejects_missing_required_fields() {
    // Missing "nodes" field
    let result = FlakeLock::from_json(r#"{"root": "root", "version": 7}"#);
    assert!(result.is_err(), "missing nodes field must be rejected");
}
