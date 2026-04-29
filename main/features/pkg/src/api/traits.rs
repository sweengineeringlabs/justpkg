use std::path::Path;

use super::error::JustpkgError;

pub trait HttpClient: Send + Sync {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError>;
    fn get_stream(&self, url: &str, dest: &mut dyn std::io::Write) -> Result<u64, JustpkgError>;
}

/// Verifies that `path` is safely joinable onto `base` — no `..`,
/// no absolute components, no Windows drive prefixes, no null bytes.
///
/// All archive extractors must call this for every archive entry.
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
