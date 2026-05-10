use thiserror::Error;

#[derive(Debug, Error)]
pub enum VminitInstallError {
    #[error("manifest parse error: {0}")]
    ManifestParse(String),

    #[error("package not found in manifest: {name:?}")]
    PackageNotFound { name: String },

    #[error("fetch failed for {name:?}: {source}")]
    FetchFailed {
        name: String,
        source: justpkg_nix::NixFetchError,
    },

    /// All substituters returned HTTP 404 for this store path.
    #[error(
        "'{name}' ({store_path}) not found in any cache: {caches:?}\n\
         hint: verify the nixpkgs revision in manifest.json is reachable from these caches, \
         or add a substituter with `--substituter <url>`"
    )]
    NotInAnyCache {
        name: String,
        store_path: String,
        caches: Vec<String>,
    },

    #[error("root layout generation failed: {reason}: {source}")]
    RootLayoutFailed {
        reason: String,
        source: std::io::Error,
    },
}
