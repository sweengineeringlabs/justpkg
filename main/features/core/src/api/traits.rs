use std::path::Path;

use super::error::JustpkgError;

/// HTTP client abstraction — injected so tests never hit the network.
pub trait HttpClient: Send + Sync {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError>;
    fn get_stream(&self, url: &str, dest: &mut dyn std::io::Write) -> Result<u64, JustpkgError>;
}

/// Extracts a downloaded package archive into a destination directory.
///
/// Every path in the archive is passed through `safe_path_join` before
/// writing. Implementations must reject paths that escape `dest_dir`.
pub trait Extractor: Send + Sync {
    fn extract(&self, archive_path: &Path, dest_dir: &Path) -> Result<(), JustpkgError>;
}

/// Verifies that `path` is safely joinable onto `base` — no `..`,
/// no absolute components, no Windows drive prefixes, no null bytes.
///
/// All `Extractor` implementations must call this for every archive entry.
pub fn safe_path_join(base: &Path, entry: &str) -> Result<std::path::PathBuf, JustpkgError> {
    if entry.contains('\0') {
        return Err(JustpkgError::UnsafeArchivePath { path: entry.to_string() });
    }
    // Reject Unix absolute paths (leading '/') and native absolute paths.
    // On Windows, is_absolute() requires a drive letter, so check both.
    if entry.starts_with('/') || std::path::Path::new(entry).is_absolute() {
        return Err(JustpkgError::UnsafeArchivePath { path: entry.to_string() });
    }
    let mut result = base.to_path_buf();
    for component in entry.split('/') {
        match component {
            "" | "." => continue,
            ".." => return Err(JustpkgError::UnsafeArchivePath { path: entry.to_string() }),
            c if c.len() == 2 && c.ends_with(':') => {
                // Windows drive prefix e.g. "C:"
                return Err(JustpkgError::UnsafeArchivePath { path: entry.to_string() });
            }
            c => result.push(c),
        }
    }
    if !result.starts_with(base) {
        return Err(JustpkgError::UnsafeArchivePath { path: entry.to_string() });
    }
    Ok(result)
}
