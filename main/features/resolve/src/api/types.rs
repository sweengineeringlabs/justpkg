use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Input: packages.toml ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PackagesSpec {
    /// NixOS channel name, e.g. `"nixos-24.11"`.
    pub nixpkgs_channel: String,

    /// Target system tuple (informational — store-paths.xz is per-channel, not per-system).
    #[serde(default = "default_system")]
    pub system: String,

    /// List of packages to resolve. TOML key is `[[package]]`.
    #[serde(rename = "package")]
    pub packages: Vec<PackageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageEntry {
    /// Short name used as the manifest key and for store-path lookup.
    /// Used to search store-paths.xz: `*-{name.replace('_','-')}-*`.
    pub name: String,

    /// Nix attribute path (informational; not used in current resolver).
    #[serde(default)]
    pub attr: String,
}

fn default_system() -> String {
    "x86_64-linux".to_string()
}

// ── Output: manifest.json ────────────────────────────────────────────────────

/// Serialises to the format consumed by `vminit::parse_manifest`:
/// `{"packages": {"name": "/nix/store/<hash>-<name>-<version>"}, "meta": {…}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedManifest {
    /// name → Nix store path; consumed by `vminit::install_packages`.
    pub packages: BTreeMap<String, String>,
    pub meta: ManifestMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMeta {
    /// Nixpkgs git revision from the channel's git-revision file.
    pub nixpkgs_rev: String,
    /// Channel that was resolved against, e.g. `"nixos-24.11"`.
    pub channel: String,
    /// RFC 3339 UTC timestamp of when `justpkg resolve` ran.
    pub resolved_at: String,
}
