use thiserror::Error;
use justpkg_core::JustpkgError;

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

    #[error(transparent)]
    Core(#[from] JustpkgError),
}
