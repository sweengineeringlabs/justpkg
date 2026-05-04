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

    #[error("root layout generation failed: {reason}: {source}")]
    RootLayoutFailed {
        reason: String,
        source: std::io::Error,
    },
}
