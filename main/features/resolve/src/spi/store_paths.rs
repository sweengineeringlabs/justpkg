use std::io::Read as _;
use std::path::{Path, PathBuf};

use justpkg_pkg::HttpClient;

use crate::api::error::ResolveError;

fn git_revision_url(channel_base: &str, channel: &str) -> String {
    let base = channel_base.trim_end_matches('/');
    format!("{base}/{channel}/git-revision")
}

fn store_paths_url(channel_base: &str, channel: &str) -> String {
    let base = channel_base.trim_end_matches('/');
    format!("{base}/{channel}/store-paths.xz")
}

fn cache_path(cache_dir: &Path, channel: &str, rev: &str) -> PathBuf {
    cache_dir.join(format!("store-paths-{channel}-{rev}.txt"))
}

/// Return the store-paths list for `channel`, using a disk cache in `cache_dir`.
///
/// Fetch sequence:
///   1. GET `<channel_base>/<channel>/git-revision` → `rev`.
///   2. Check `<cache_dir>/store-paths-<channel>-<rev>.txt`.  On hit: read + return.
///   3. On miss: GET `…/<channel>/store-paths.xz`, xz-decompress, write cache, return.
pub fn fetch_store_paths(
    http: &dyn HttpClient,
    channel: &str,
    channel_base: &str,
    cache_dir: &Path,
) -> Result<Vec<String>, ResolveError> {
    let rev = fetch_git_revision(http, channel, channel_base)?;
    let path = cache_path(cache_dir, channel, &rev);

    if path.exists() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok(text.lines().map(|s| s.to_string()).collect());
        }
    }

    let url = store_paths_url(channel_base, channel);
    let compressed = http.get_bytes(&url).map_err(|e| ResolveError::ChannelFetch {
        channel: channel.to_string(),
        message: e.to_string(),
    })?;

    let text = decompress_xz(&compressed, &url, channel)?;
    std::fs::write(&path, &text).map_err(|e| ResolveError::CacheWrite {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    Ok(text.lines().map(|s| s.to_string()).collect())
}

/// Fetch the current nixpkgs git revision for `channel` — callers use it as `nixpkgs_rev` in manifests.
pub fn fetch_git_revision(
    http: &dyn HttpClient,
    channel: &str,
    channel_base: &str,
) -> Result<String, ResolveError> {
    let url = git_revision_url(channel_base, channel);
    let bytes = http.get_bytes(&url).map_err(|e| ResolveError::ChannelFetch {
        channel: channel.to_string(),
        message: e.to_string(),
    })?;
    let rev = String::from_utf8_lossy(&bytes).trim().to_string();
    if rev.len() < 7 {
        return Err(ResolveError::ChannelFetch {
            channel: channel.to_string(),
            message: format!("git-revision too short: {rev:?}"),
        });
    }
    Ok(rev)
}

/// Find the best-match store path for a package `name` in the store-paths list.
///
/// Matching is anchored to the derivation name portion of the store path:
///   `/nix/store/<32-char-hash>-<derivation-name>`
///                               ^ match starts HERE
///
/// A path matches if its derivation name starts with `{name_hyphen}-` or
/// `{name_hyphen}.` where `name_hyphen = name.replace('_', '-')`.  This prevents
/// `python3.11-tzdata` from matching a search for `tzdata`.
///
/// Disambiguation when multiple paths match:
///   - Paths with extension suffixes (`-lib`, `-dev`, `-man`, `-doc`, `-bin`,
///     `-data`, `-debug`, `-bin`) are deprioritised (secondary outputs).
///   - Paths containing test-related keywords are excluded.
///   - Among the remaining candidates, the lexicographically smallest store hash is chosen.
pub fn find_store_path<'a>(paths: &'a [String], name: &str) -> Option<&'a str> {
    let name_hyphen = name.replace('_', "-");
    // Derivation name starts immediately after `/nix/store/<32-char-hash>-`
    // i.e., at byte offset 44 (11 + 32 + 1).
    let drv_name_offset = "/nix/store/".len() + 32 + 1; // = 44

    let extension_suffixes = [
        "-lib", "-dev", "-man", "-doc", "-bin", "-data", "-debug",
        "-include", "-static", "-headers",
    ];
    let exclude_keywords = ["test-run-", "nixos-test", "test-driver", "-driver-"];

    let matches_name = |p: &str| -> bool {
        if p.len() <= drv_name_offset {
            return false;
        }
        let drv = &p[drv_name_offset..];
        // Exact name match: `{name}-<version>` where the first char of the version
        // is a digit or '.'.  This rejects `redis-schema-0.1` when searching for
        // `redis` — after `redis-`, 's' is a letter, meaning a different package.
        if let Some(rest) = drv.strip_prefix(&format!("{name_hyphen}-")) {
            rest.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        } else {
            drv.starts_with(&format!("{name_hyphen}."))
        }
    };

    let mut candidates: Vec<&str> = paths
        .iter()
        .filter(|p| {
            matches_name(p) && !exclude_keywords.iter().any(|kw| p.contains(kw))
        })
        .map(|s| s.as_str())
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort by derivation name (descending) to prefer lexicographically later versions
    // (e.g. "tzdata-2025b" before "tzdata-0.2.20240201.0").  Within the same derivation
    // name, fall back to store hash (ascending) for determinism.
    candidates.sort_unstable_by(|a, b| {
        let drv_a = if a.len() > drv_name_offset { &a[drv_name_offset..] } else { "" };
        let drv_b = if b.len() > drv_name_offset { &b[drv_name_offset..] } else { "" };
        drv_b.cmp(drv_a).then(a.cmp(b))
    });

    let primary: Vec<&str> = candidates
        .iter()
        .copied()
        .filter(|p| {
            !extension_suffixes.iter().any(|suf| p.ends_with(suf))
        })
        .collect();

    Some(if !primary.is_empty() { primary[0] } else { candidates[0] })
}

fn decompress_xz(data: &[u8], url: &str, channel: &str) -> Result<String, ResolveError> {
    let mut decoder = xz2::read::XzDecoder::new(data);
    let mut out = String::new();
    decoder
        .read_to_string(&mut out)
        .map_err(|e| ResolveError::StorePathsDecompress {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let _ = channel; // used in error context only
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(s: &[&str]) -> Vec<String> {
        s.iter().map(|&x| x.to_string()).collect()
    }

    #[test]
    fn test_find_store_path_matches_exact_name() {
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-su-exec-0.2",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-curl-8.10.1",
        ]);
        assert_eq!(
            find_store_path(&ps, "su-exec"),
            Some("/nix/store/aaaabbbbccccddddeeeeffffgggg0000-su-exec-0.2")
        );
    }

    #[test]
    fn test_find_store_path_handles_underscore_to_hyphen() {
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-postgresql-16.9",
        ]);
        assert_eq!(
            find_store_path(&ps, "postgresql_16"),
            Some("/nix/store/aaaabbbbccccddddeeeeffffgggg0000-postgresql-16.9")
        );
    }

    #[test]
    fn test_find_store_path_prefers_primary_over_lib() {
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-postgresql-16.9",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-postgresql-16.9-lib",
        ]);
        let result = find_store_path(&ps, "postgresql_16").unwrap();
        assert!(
            !result.ends_with("-lib"),
            "must prefer primary output over -lib: {result}"
        );
    }

    #[test]
    fn test_find_store_path_excludes_test_paths() {
        // vm-test-run-citus-postgresql-16.9 starts with 'vm-test-run', not 'postgresql-16',
        // so the anchored match rejects it automatically.
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-vm-test-run-citus-postgresql-16.9",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-postgresql-16.9",
        ]);
        let result = find_store_path(&ps, "postgresql_16").unwrap();
        assert!(
            result.contains("bbbb"),
            "must pick the direct postgresql-16.9 path, not the test driver: {result}"
        );
    }

    #[test]
    fn test_find_store_path_does_not_match_prefixed_package() {
        // python3.11-tzdata must NOT match when searching for tzdata.
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-python3.11-tzdata-2024.2",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-tzdata-2025b",
        ]);
        let result = find_store_path(&ps, "tzdata").unwrap();
        assert!(
            !result.contains("python"),
            "python3.11-tzdata must not match a search for tzdata: {result}"
        );
    }

    #[test]
    fn test_find_store_path_returns_none_for_unknown_package() {
        let ps = paths(&["/nix/store/aaaabbbbccccddddeeeeffffgggg0000-curl-8.10.1"]);
        assert!(find_store_path(&ps, "nonexistent").is_none());
    }

    #[test]
    fn test_find_store_path_rejects_letter_after_name_prefix() {
        // redis-schema-0.1.0 must NOT match a search for redis —
        // after `redis-`, the next char is 's' (letter), not a digit.
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-redis-schema-0.1.0",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-redis-7.4.1",
        ]);
        let result = find_store_path(&ps, "redis").unwrap();
        assert!(
            result.contains("redis-7"),
            "must pick redis-7.x.x, not redis-schema: {result}"
        );
    }

    #[test]
    fn test_find_store_path_deterministic_for_same_drv_name() {
        // Same derivation name, two hashes → deterministic (smaller hash wins as tiebreak).
        let ps = paths(&[
            "/nix/store/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz-postgresql-16.9",
            "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-postgresql-16.9",
        ]);
        let r1 = find_store_path(&ps, "postgresql_16").unwrap();
        let r2 = find_store_path(&ps, "postgresql_16").unwrap();
        assert_eq!(r1, r2, "must be deterministic");
        // Same drv name → hash tiebreak → smallest hash.
        assert!(r1.contains("aaaa"), "hash tiebreak must pick lexicographically smallest: {r1}");
    }

    #[test]
    fn test_find_store_path_prefers_later_version() {
        // tzdata-2025b > tzdata-0.2.20240201.0 lexicographically (2 > 0).
        let ps = paths(&[
            "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-tzdata-0.2.20240201.0",
            "/nix/store/bbbbccccddddeeeeffffgggg00001111-tzdata-2025b",
        ]);
        let result = find_store_path(&ps, "tzdata").unwrap();
        assert!(
            result.contains("2025b"),
            "must prefer later version (2025b over 0.2.20240201.0): {result}"
        );
    }
}
