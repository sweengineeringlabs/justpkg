use justpkg_pkg::JustpkgError;
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
    Core(#[from] JustpkgError),
}

/// Returns `true` if `e` indicates the store path was absent from the cache
/// (i.e. the cache returned HTTP 404).  Use this to drive substituter fallback.
pub fn is_not_found(e: &NixFetchError) -> bool {
    matches!(e, NixFetchError::NotFound { .. })
}
