use thiserror::Error;

#[derive(Debug, Error)]
pub enum JustpkgError {
    #[error("HTTP error fetching {url}: status {status}")]
    Http { url: String, status: u16 },

    #[error("SHA-256 mismatch for {url}: expected {expected}, got {actual}")]
    HashMismatch { url: String, expected: String, actual: String },

    #[error("package not found: {name} ({constraint})")]
    PackageNotFound { name: String, constraint: String },

    #[error("dependency conflict: {package} required at '{a}' and '{b}'")]
    DependencyConflict { package: String, a: String, b: String },

    #[error("unsafe archive path rejected: {path}")]
    UnsafeArchivePath { path: String },

    #[error("parse error in {context}: {message}")]
    Parse { context: String, message: String },

    #[error("CAS error: {0}")]
    Cas(#[from] cas::CasError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
