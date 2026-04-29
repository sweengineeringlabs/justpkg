use std::collections::HashMap;

/// Maps plain package names to Nix NAR SRI hashes.
///
/// Loaded from `/packages/.manifest.json` in the initrd.
/// Format: `{"packages": {"curl": "sha256-xxx", "git": "sha256-yyy"}}`
#[derive(Debug)]
pub struct PackageManifest {
    /// Maps package name → NAR hash (SRI format, e.g. `sha256-AAAA...=`).
    pub entries: HashMap<String, String>,
}
