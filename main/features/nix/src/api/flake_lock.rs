use serde::Deserialize;
use std::collections::HashMap;

/// Deserialised `flake.lock` — version 7 format.
///
/// The closure is pre-computed; no dependency resolution is needed.
/// Each locked node maps to one or more NARs to fetch from the binary cache.
#[derive(Debug, Deserialize)]
pub struct FlakeLock {
    pub nodes: HashMap<String, Node>,
    pub root: String,
    pub version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Locked(LockedNode),
    Root(RootNode),
}

#[derive(Debug, Deserialize)]
pub struct LockedNode {
    pub locked: LockedSource,
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct RootNode {
    #[serde(default)]
    pub inputs: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct LockedSource {
    #[serde(rename = "narHash")]
    pub nar_hash: String,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<u64>,
    pub rev: Option<String>,
    #[serde(rename = "type")]
    pub source_type: String,
    // github/gitlab specific
    pub owner: Option<String>,
    pub repo: Option<String>,
    // tarball specific
    pub url: Option<String>,
}

impl FlakeLock {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Returns all locked nodes (excludes the root node).
    pub fn locked_nodes(&self) -> impl Iterator<Item = (&String, &LockedNode)> {
        self.nodes.iter().filter_map(|(k, v)| {
            if let Node::Locked(n) = v {
                Some((k, n))
            } else {
                None
            }
        })
    }
}
