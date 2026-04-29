use std::io::Read as _;
use std::path::{Path, PathBuf};

use justpkg_pkg::HttpClient;

use crate::api::error::ResolveError;
use crate::api::types::ChannelIndex;

const CHANNEL_BASE: &str = "https://channels.nixos.org";

/// URL of the brotli-compressed packages index for `channel`.
fn index_url(channel: &str) -> String {
    format!("{CHANNEL_BASE}/{channel}/packages.json.br")
}

/// URL of the plain-text git revision for `channel`.
fn git_revision_url(channel: &str) -> String {
    format!("{CHANNEL_BASE}/{channel}/git-revision")
}

/// Disk-cache path for a resolved channel index.
///
/// Keyed by channel name + nixpkgs rev so the same channel at different
/// revisions never collide, and the same rev is never re-downloaded.
fn cache_path(cache_dir: &Path, channel: &str, rev: &str) -> PathBuf {
    cache_dir.join(format!("channel-{channel}-{rev}.json"))
}

/// Return the `ChannelIndex` for `channel`, using a disk cache in `cache_dir`.
///
/// Fetch sequence:
///   1. GET `channels.nixos.org/<channel>/git-revision` → `rev` (tiny, always fresh).
///   2. Check `<cache_dir>/channel-<channel>-<rev>.json`.  On hit: parse + return.
///   3. On miss: GET `…/packages.json.br`, brotli-decompress, parse JSON, write cache.
///
/// `index.commit` is always populated with `rev` regardless of whether the data
/// came from cache or network (packages.json itself has no `commit` field).
pub fn fetch_channel_index(
    http: &dyn HttpClient,
    channel: &str,
    cache_dir: &Path,
) -> Result<ChannelIndex, ResolveError> {
    let rev = fetch_git_revision(http, channel)?;
    let path = cache_path(cache_dir, channel, &rev);

    if path.exists() {
        if let Ok(json) = std::fs::read(&path) {
            if let Ok(mut index) = parse_index(&json, channel) {
                index.commit = rev;
                return Ok(index);
            }
        }
    }

    let url = index_url(channel);
    let compressed = http.get_bytes(&url).map_err(|e| ResolveError::ChannelFetch {
        channel: channel.to_string(),
        message: e.to_string(),
    })?;

    let json = decompress_brotli(&compressed, &url)?;
    let mut index = parse_index(&json, channel)?;
    index.commit = rev;

    std::fs::write(&path, &json).map_err(|e| ResolveError::CacheWrite {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;

    Ok(index)
}

/// Fetch the current nixpkgs git revision for `channel` from `git-revision`.
fn fetch_git_revision(http: &dyn HttpClient, channel: &str) -> Result<String, ResolveError> {
    let url = git_revision_url(channel);
    let bytes = http.get_bytes(&url).map_err(|e| ResolveError::ChannelFetch {
        channel: channel.to_string(),
        message: e.to_string(),
    })?;
    let rev = String::from_utf8_lossy(&bytes).trim().to_string();
    if rev.len() < 7 {
        return Err(ResolveError::ChannelFetch {
            channel: channel.to_string(),
            message: format!("git-revision response too short: {rev:?}"),
        });
    }
    Ok(rev)
}

/// Look up the `out` store path for `attr` in `index`.
/// Returns `None` if the attribute is absent, has no `out` output, or the path is null.
pub fn lookup_store_path<'a>(index: &'a ChannelIndex, attr: &str) -> Option<&'a str> {
    index
        .packages
        .get(attr)
        .and_then(|pkg| pkg.outputs.get("out"))
        .and_then(|opt| opt.as_deref())
}

fn decompress_brotli(compressed: &[u8], url: &str) -> Result<Vec<u8>, ResolveError> {
    let mut decoder = brotli::Decompressor::new(compressed, 4096);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).map_err(|e| ResolveError::BrotliDecompress {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    Ok(out)
}

fn parse_index(json: &[u8], channel: &str) -> Result<ChannelIndex, ResolveError> {
    serde_json::from_slice(json).map_err(|e| ResolveError::ChannelParse {
        channel: channel.to_string(),
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real packages.json format: `version` + `packages`, no `commit`.
    const FIXTURE: &str = r#"{
        "version": 2,
        "packages": {
            "pkgsMusl.postgresql_16": {
                "outputs": {
                    "out": "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-postgresql-16.6",
                    "lib": "/nix/store/llllmmmmnnnn0000ppppqqqqrrrr1111-postgresql-16.6-lib"
                }
            },
            "pkgsMusl.su-exec": {
                "outputs": {
                    "out": "/nix/store/ssssttttuuuuvvvvwwwwxxxxyyyyzzzz-su-exec-0.2"
                }
            },
            "pkgsMusl.broken": {
                "outputs": {
                    "out": null
                }
            }
        }
    }"#;

    #[test]
    fn test_parse_index_commit_defaults_empty() {
        let index = parse_index(FIXTURE.as_bytes(), "nixos-24.11").unwrap();
        assert_eq!(index.commit, "", "commit must default to empty when absent from JSON");
    }

    #[test]
    fn test_lookup_store_path_returns_out_output() {
        let index = parse_index(FIXTURE.as_bytes(), "nixos-24.11").unwrap();
        let path = lookup_store_path(&index, "pkgsMusl.postgresql_16").unwrap();
        assert_eq!(path, "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-postgresql-16.6");
    }

    #[test]
    fn test_lookup_store_path_returns_none_for_null_output() {
        let index = parse_index(FIXTURE.as_bytes(), "nixos-24.11").unwrap();
        assert!(
            lookup_store_path(&index, "pkgsMusl.broken").is_none(),
            "null output path must be treated as missing"
        );
    }

    #[test]
    fn test_lookup_store_path_returns_none_for_unknown_attr() {
        let index = parse_index(FIXTURE.as_bytes(), "nixos-24.11").unwrap();
        assert!(lookup_store_path(&index, "pkgsMusl.nonexistent").is_none());
    }

    #[test]
    fn test_parse_index_fails_on_malformed_json() {
        let err = parse_index(b"not json", "nixos-24.11");
        assert!(err.is_err());
    }
}
