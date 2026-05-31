use edge_domain::ServiceError;
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

impl From<VminitInstallError> for ServiceError {
    fn from(e: VminitInstallError) -> Self {
        match e {
            VminitInstallError::ManifestParse(msg) => {
                ServiceError::InvalidRequest(format!("manifest parse error: {msg}"))
            }
            VminitInstallError::PackageNotFound { name } => {
                ServiceError::NotFound(format!("package not found in manifest: {name:?}"))
            }
            VminitInstallError::FetchFailed { name, source } => {
                ServiceError::Unavailable(format!("fetch failed for {name:?}: {source}"))
            }
            VminitInstallError::NotInAnyCache { name, store_path, caches } => {
                ServiceError::NotFound(format!(
                    "'{name}' ({store_path}) not found in any cache: {caches:?}"
                ))
            }
            VminitInstallError::RootLayoutFailed { reason, source } => {
                ServiceError::Internal(format!("root layout generation failed: {reason}: {source}"))
            }
        }
    }
}
