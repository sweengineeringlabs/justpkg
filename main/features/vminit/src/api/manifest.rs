use std::collections::HashMap;

/// Maps plain package names to Nix store paths.
///
/// Loaded from `/packages/.manifest.json` in the initrd.
/// Format: `{"packages": {"curl": "/nix/store/<hash>-curl-8.x", …}}`
#[derive(Debug)]
pub struct PackageManifest {
    /// Maps package name → absolute Nix store path, e.g. `/nix/store/<32-char-hash>-curl-8.x`.
    pub entries: HashMap<String, String>,
}
