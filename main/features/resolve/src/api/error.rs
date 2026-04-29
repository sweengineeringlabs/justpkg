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
