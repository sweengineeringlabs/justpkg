use edge_domain::ServiceError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("packages.toml read error for {path}: {message}")]
    SpecRead { path: String, message: String },

    #[error("packages.toml parse error for {path}: {message}")]
    SpecParse { path: String, message: String },

    #[error("channel fetch failed for '{channel}': {message}")]
    ChannelFetch { channel: String, message: String },

    #[error("store-paths.xz decompression error for '{url}': {message}")]
    StorePathsDecompress { url: String, message: String },

    #[error("package '{name}' not found in '{channel}' store-paths (store-paths.xz covers only the channel closure)")]
    PackageNotFound { name: String, channel: String },

    #[error("disk cache write error for '{path}': {message}")]
    CacheWrite { path: String, message: String },

    #[error("manifest write error for '{path}': {message}")]
    ManifestWrite { path: String, message: String },
}

impl From<ResolveError> for ServiceError {
    fn from(e: ResolveError) -> Self {
        match e {
            ResolveError::SpecRead { path, message } => {
                ServiceError::InvalidRequest(format!("packages.toml read error for {path}: {message}"))
            }
            ResolveError::SpecParse { path, message } => {
                ServiceError::InvalidRequest(format!("packages.toml parse error for {path}: {message}"))
            }
            ResolveError::ChannelFetch { channel, message } => {
                ServiceError::Unavailable(format!("channel fetch failed for '{channel}': {message}"))
            }
            ResolveError::StorePathsDecompress { url, message } => {
                ServiceError::Internal(format!("store-paths.xz decompression error for '{url}': {message}"))
            }
            ResolveError::PackageNotFound { name, channel } => {
                ServiceError::NotFound(format!("package '{name}' not found in '{channel}' store-paths"))
            }
            ResolveError::CacheWrite { path, message } => {
                ServiceError::Internal(format!("disk cache write error for '{path}': {message}"))
            }
            ResolveError::ManifestWrite { path, message } => {
                ServiceError::Internal(format!("manifest write error for '{path}': {message}"))
            }
        }
    }
}
