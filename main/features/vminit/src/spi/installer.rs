/// Fetch and extract each listed package using the manifest and `NixFetcher`.
///
/// For each `name` in `names`:
/// 1. Look up the SRI hash in `manifest.entries` — returns [`VminitInstallError::PackageNotFound`] if absent.
/// 2. Construct a synthetic [`FlakeLock`] whose single locked node carries that SRI.
/// 3. Delegate to `NixFetcher::build` — maps errors to [`VminitInstallError::FetchFailed`].
use std::path::Path;

use justpkg_nix::{FlakeLock, NixFetcher};
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
        let sri =
            manifest
                .entries
                .get(name)
                .ok_or_else(|| VminitInstallError::PackageNotFound {
                    name: name.to_string(),
                })?;

        let lock = build_synthetic_lock(name, sri);
        let fetcher = NixFetcher { http };
        fetcher
            .build(&lock, dest_dir)
            .map_err(|source| VminitInstallError::FetchFailed {
                name: name.to_string(),
                source,
            })?;
    }
    Ok(())
}

/// Construct a minimal [`FlakeLock`] (version 7) containing a single locked node
/// for `name` with the given `nar_hash` SRI.  All optional fields are omitted.
fn build_synthetic_lock(name: &str, nar_hash: &str) -> FlakeLock {
    // Build as JSON so we reuse the existing serde Deserialize path on FlakeLock
    // and stay in sync with any future schema changes.
    let name_escaped = serde_json::Value::String(name.to_string()).to_string();
    let hash_escaped = serde_json::Value::String(nar_hash.to_string()).to_string();
    let json = format!(
        r#"{{
            "nodes": {{
                "root": {{ "inputs": {{ "pkg": {name_escaped} }} }},
                {name_escaped}: {{
                    "locked": {{
                        "narHash": {hash_escaped},
                        "type": "tarball"
                    }},
                    "inputs": {{}}
                }}
            }},
            "root": "root",
            "version": 7
        }}"#
    );
    FlakeLock::from_json(&json).expect("synthetic FlakeLock JSON is always well-formed")
}
