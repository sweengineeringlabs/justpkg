/// Fetch and extract each listed package using the manifest and `NixFetcher`.
///
/// For each `name` in `names`:
/// 1. Look up the Nix store path in `manifest.entries` — returns
///    [`VminitInstallError::PackageNotFound`] if absent.
/// 2. Delegate to `NixFetcher::build_store_path` — maps errors to
///    [`VminitInstallError::FetchFailed`].  `build_store_path` resolves the full
///    transitive closure by following `References` in each narinfo.
use std::path::Path;

use justpkg_nix::{NixFetcher, DEFAULT_CACHE_BASE};
use justpkg_pkg::HttpClient;

use crate::api::error::VminitInstallError;
use crate::api::manifest::PackageManifest;

pub fn install_packages(
    http: &dyn HttpClient,
    manifest: &PackageManifest,
    names: &[&str],
    dest_dir: &Path,
) -> Result<(), VminitInstallError> {
    for &name in names {
        let store_path =
            manifest
                .entries
                .get(name)
                .ok_or_else(|| VminitInstallError::PackageNotFound {
                    name: name.to_string(),
                })?;

        let fetcher = NixFetcher { http, cache_base: DEFAULT_CACHE_BASE };
        fetcher
            .build_store_path(store_path, dest_dir)
            .map_err(|source| VminitInstallError::FetchFailed {
                name: name.to_string(),
                source,
            })?;
    }
    Ok(())
}
