use std::collections::BTreeMap;
use std::path::Path;

use justpkg_pkg::HttpClient;

use crate::api::error::ResolveError;
use crate::api::types::{ManifestMeta, PackagesSpec, ResolvedManifest};
use crate::spi::{manifest, store_paths};

/// Parse a `packages.toml` file from disk.
pub fn load_packages_spec(path: &Path) -> Result<PackagesSpec, ResolveError> {
    let text = std::fs::read_to_string(path).map_err(|e| ResolveError::SpecRead {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    toml::from_str(&text).map_err(|e| ResolveError::SpecParse {
        path: path.display().to_string(),
        message: e.to_string(),
    })
}

/// Resolve all packages in `spec` to Nix store paths and write `manifest.json` to `out`.
///
/// `channel_base` — base URL for channel index files, e.g. `"https://channels.nixos.org"`.
/// Loaded from `[nix] channel_base` in `application.toml`; default is the public NixOS mirror.
///
/// Resolution source: `<channel_base>/<channel>/store-paths.xz` — the complete set of
/// store paths shipped in the channel closure.  The store-paths list is cached in
/// `disk_cache_dir` keyed by channel + git revision, so repeated runs skip the ~5 MB download.
pub fn resolve(
    http: &dyn HttpClient,
    spec: &PackagesSpec,
    channel_base: &str,
    disk_cache_dir: &Path,
    out: &Path,
) -> Result<ResolvedManifest, ResolveError> {
    let rev = store_paths::fetch_git_revision(http, &spec.nixpkgs_channel, channel_base)?;
    let paths =
        store_paths::fetch_store_paths(http, &spec.nixpkgs_channel, channel_base, disk_cache_dir)?;

    let mut packages = BTreeMap::new();

    for entry in &spec.packages {
        let store_path = store_paths::find_store_path(&paths, &entry.name).ok_or_else(|| {
            ResolveError::PackageNotFound {
                name: entry.name.clone(),
                channel: spec.nixpkgs_channel.clone(),
            }
        })?;
        packages.insert(entry.name.clone(), store_path.to_string());
    }

    let resolved = ResolvedManifest {
        packages,
        meta: ManifestMeta {
            nixpkgs_rev: rev,
            channel: spec.nixpkgs_channel.clone(),
            resolved_at: manifest::now_rfc3339(),
        },
    };

    manifest::write_manifest(&resolved, out)?;
    Ok(resolved)
}
