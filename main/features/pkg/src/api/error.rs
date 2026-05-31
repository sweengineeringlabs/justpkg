use edge_domain::ServiceError;
use thiserror::Error;

/// Internal HTTP/IO/CAS error type for the `pkg` crate.
///
/// External callers convert to [`ServiceError`] via the [`From`] impl below.
/// This type is local so it satisfies Rust's orphan rules for the conversion.
#[derive(Debug, Error)]
pub enum PkgError {
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

impl From<PkgError> for ServiceError {
    fn from(e: PkgError) -> Self {
        match e {
            PkgError::Http { url, status } => {
                ServiceError::Unavailable(format!("HTTP {status} fetching {url}"))
            }
            PkgError::HashMismatch { url, expected, actual } => ServiceError::Internal(
                format!("SHA-256 mismatch for {url}: expected {expected}, got {actual}"),
            ),
            PkgError::PackageNotFound { name, constraint } => {
                ServiceError::NotFound(format!("package not found: {name} ({constraint})"))
            }
            PkgError::DependencyConflict { package, a, b } => ServiceError::RuleViolation(
                format!("dependency conflict: {package} required at '{a}' and '{b}'"),
            ),
            PkgError::UnsafeArchivePath { path } => {
                ServiceError::InvalidRequest(format!("unsafe archive path rejected: {path}"))
            }
            PkgError::Parse { context, message } => {
                ServiceError::Internal(format!("parse error in {context}: {message}"))
            }
            PkgError::Cas(e) => ServiceError::Internal(format!("CAS error: {e}")),
            PkgError::Io(e) => ServiceError::Internal(format!("IO error: {e}")),
        }
    }
}
