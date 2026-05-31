use edge_domain::ServiceError;
use justpkg_pkg::PkgError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NixFetchError {
    #[error("flake.lock parse error: {0}")]
    FlakeLockParse(String),

    #[error("narinfo parse error for {hash}: {message}")]
    NarInfoParse { hash: String, message: String },

    #[error("invalid Nix hash encoding '{0}'")]
    InvalidNixHash(String),

    #[error("NAR extraction error: {0}")]
    NarExtract(String),

    /// The store path is not present in the cache (HTTP 404).
    /// Callers that implement substituter fallback should catch this variant
    /// and retry against the next substituter before surfacing an error.
    #[error("store path {store_hash} not found in cache {cache}")]
    NotFound { cache: String, store_hash: String },

    #[error(transparent)]
    Core(#[from] PkgError),
}

/// Returns `true` if `e` indicates the store path was absent from the cache.
pub fn is_not_found(e: &NixFetchError) -> bool {
    matches!(e, NixFetchError::NotFound { .. })
}

impl From<NixFetchError> for ServiceError {
    fn from(e: NixFetchError) -> Self {
        match e {
            NixFetchError::FlakeLockParse(msg) => ServiceError::Internal(msg),
            NixFetchError::NarInfoParse { hash, message } => {
                ServiceError::Internal(format!("narinfo parse error for {hash}: {message}"))
            }
            NixFetchError::InvalidNixHash(s) => {
                ServiceError::InvalidRequest(format!("invalid Nix hash encoding '{s}'"))
            }
            NixFetchError::NarExtract(msg) => ServiceError::Internal(msg),
            NixFetchError::NotFound { cache, store_hash } => {
                ServiceError::NotFound(format!("store path {store_hash} not found in {cache}"))
            }
            NixFetchError::Core(e) => ServiceError::from(e),
        }
    }
}
