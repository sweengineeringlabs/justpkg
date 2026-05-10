//! Orchestrates the full Nix NAR fetch pipeline:
//!   flake.lock → narinfo → compressed NAR → verified + extracted

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use cas::{Algorithm, Cas, Digest};
use justpkg_pkg::HttpClient;

use crate::api::error::NixFetchError;
use crate::api::flake_lock::FlakeLock;
use crate::api::narinfo::{Compression, NarInfo};
use crate::spi::nar::extract_nar;
use crate::spi::nix_hash;

pub struct NixFetcher<'a> {
    pub http: &'a dyn HttpClient,
    /// Nix binary cache base URL (e.g. `"https://cache.nixos.org"`).
    /// Loaded from `application.toml` by the caller; use
    /// `swe_justpkg_nix::DEFAULT_CACHE_BASE` when no config is available.
    pub cache_base: &'a str,
    /// Bearer token for authenticated caches (Attic, Cachix private).
    /// When `Some`, requests carry `Authorization: Bearer <token>`.
    pub token: Option<&'a str>,
}

impl<'a> NixFetcher<'a> {
    /// Fetch and extract all locked nodes from a `flake.lock` into `dest_dir`,
    /// recursively resolving transitive dependencies (the closure).
    pub fn build(&self, lock: &FlakeLock, dest_dir: &Path) -> Result<(), NixFetchError> {
        let mut visited = std::collections::HashSet::new();
        for (name, node) in lock.locked_nodes() {
            let sri = &node.locked.nar_hash;
            eprintln!("  fetch {name}");
            let store_hash = nix_hash::nar_hash_to_store_path_hash(sri)?;
            self.build_with_closure(&store_hash, dest_dir, &mut visited)?;
        }
        Ok(())
    }

    /// Download narinfo + compressed NAR for every locked node and store the
    /// compressed bytes in `cas` keyed by their SHA-256.
    ///
    /// Returns a map of node name → `Digest` so the caller can pass it to
    /// `extract_from_cas` to complete the pipeline in a separate step.
    pub fn fetch_to_cas(
        &self,
        lock: &FlakeLock,
        cas: &dyn Cas,
    ) -> Result<HashMap<String, Digest>, NixFetchError> {
        let mut result = HashMap::new();
        for (name, node) in lock.locked_nodes() {
            let sri = &node.locked.nar_hash;
            eprintln!("  fetch {name}");
            let narinfo = self.fetch_narinfo(sri)?;
            let nar_url = format!("{}/{}", self.cache_base, narinfo.url);

            let mut compressed_bytes = Vec::new();
            self.http
                .get_stream_auth(&nar_url, self.token, &mut compressed_bytes)?;

            let digest = cas
                .put(&compressed_bytes)
                .map_err(|e| NixFetchError::NarExtract(e.to_string()))?;

            result.insert(name.clone(), digest);
        }
        Ok(result)
    }

    /// Read each locked node's compressed NAR from `cas`, decompress, verify
    /// integrity, and extract to the Nix store path layout under `dest_dir`.
    ///
    /// The `cas` must already contain all blobs populated by `fetch_to_cas`.
    /// Each blob is identified by the SHA-256 digest of its compressed bytes —
    /// the same key that `fetch_to_cas` produced when calling `cas.put`.
    ///
    /// A missing blob is surfaced as `NixFetchError::NarExtract`.
    pub fn extract_from_cas(
        &self,
        lock: &FlakeLock,
        cas: &dyn Cas,
        dest_dir: &Path,
    ) -> Result<(), NixFetchError> {
        for (name, node) in lock.locked_nodes() {
            let sri = &node.locked.nar_hash;
            eprintln!("  extract {name}");
            let narinfo = self.fetch_narinfo(sri)?;

            // Derive the CAS lookup key from narinfo.file_hash (Nix base-32 or
            // hex SHA-256 of the compressed file) — no need to re-download the
            // blob just to reconstruct the digest.
            let file_hash_bytes = if narinfo.file_hash.len() == 52 {
                nix_hash::nix_base32_decode(&narinfo.file_hash).map_err(|e| {
                    NixFetchError::NarExtract(format!("invalid FileHash base-32: {e}"))
                })?
            } else if narinfo.file_hash.len() == 64 {
                hex::decode(&narinfo.file_hash)
                    .map_err(|e| NixFetchError::NarExtract(format!("invalid FileHash hex: {e}")))?
            } else {
                return Err(NixFetchError::NarExtract(format!(
                    "unexpected FileHash length {} for {name}: expected 52 (Nix base-32) or 64 (hex)",
                    narinfo.file_hash.len()
                )));
            };

            let digest = Digest::from_hash_output(Algorithm::Sha256, &file_hash_bytes);
            let stored = cas
                .get(&digest)
                .map_err(|e| NixFetchError::NarExtract(e.to_string()))?;

            let uncompressed =
                decompress(&narinfo.compression, &stored).map_err(NixFetchError::NarExtract)?;

            // Issue #3: verify NAR integrity before extraction
            verify_nar_hash(&uncompressed, &narinfo.nar_hash)?;

            let store_basename = narinfo
                .store_path
                .strip_prefix("/nix/store/")
                .unwrap_or(&narinfo.store_path);
            let extract_path = dest_dir.join("nix").join("store").join(store_basename);
            // Create the parent so extract_nar can write the store node.
            // The NAR extractor handles creation of the node itself.
            let parent = extract_path
                .parent()
                .ok_or_else(|| NixFetchError::NarExtract("store path has no parent".to_string()))?;
            std::fs::create_dir_all(parent).map_err(|e| {
                NixFetchError::NarExtract(format!("failed to create store parent dir: {e}"))
            })?;
            extract_nar(Cursor::new(uncompressed), &extract_path)?;
        }
        Ok(())
    }

    fn fetch_narinfo_by_store_hash(&self, store_hash: &str) -> Result<NarInfo, NixFetchError> {
        let narinfo_url = format!("{}/{store_hash}.narinfo", self.cache_base);
        let narinfo_bytes = self
            .http
            .get_bytes_auth(&narinfo_url, self.token)
            .map_err(|e| {
                // Translate HTTP 404 to NotFound so callers can implement substituter fallback.
                if let justpkg_pkg::JustpkgError::Http { status: 404, .. } = &e {
                    return NixFetchError::NotFound {
                        cache: self.cache_base.to_string(),
                        store_hash: store_hash.to_string(),
                    };
                }
                NixFetchError::Core(e)
            })?;
        let narinfo_text =
            String::from_utf8(narinfo_bytes).map_err(|e| NixFetchError::NarInfoParse {
                hash: store_hash.to_string(),
                message: e.to_string(),
            })?;
        NarInfo::parse(store_hash, &narinfo_text)
    }

    /// Fetch and extract a single Nix store path and its full transitive closure
    /// into `dest_dir`.
    ///
    /// `store_path` must be an absolute path under `/nix/store/` with the standard
    /// layout `/<32-char-hash>-<name>-<version>`.  The store hash is extracted and
    /// used to fetch the `.narinfo` from `cache.nixos.org`.
    pub fn build_store_path(&self, store_path: &str, dest_dir: &Path) -> Result<(), NixFetchError> {
        let basename = store_path.strip_prefix("/nix/store/").ok_or_else(|| {
            NixFetchError::NarExtract(format!("invalid store path: {store_path:?}"))
        })?;
        let store_hash = basename.split_once('-').map(|(h, _)| h).ok_or_else(|| {
            NixFetchError::NarExtract(format!("store path missing hash separator: {store_path:?}"))
        })?;
        let mut visited = std::collections::HashSet::new();
        self.build_with_closure(store_hash, dest_dir, &mut visited)
    }

    fn build_with_closure(
        &self,
        store_hash: &str,
        dest_dir: &Path,
        visited: &mut std::collections::HashSet<String>,
    ) -> Result<(), NixFetchError> {
        if !visited.insert(store_hash.to_string()) {
            return Ok(()); // already fetched — handles cycles and shared deps
        }

        let narinfo = self.fetch_narinfo_by_store_hash(store_hash)?;

        // Extract to Nix store path layout: <dest_dir>/nix/store/<basename>
        let store_basename = narinfo
            .store_path
            .strip_prefix("/nix/store/")
            .unwrap_or(&narinfo.store_path);
        let extract_path = dest_dir.join("nix").join("store").join(store_basename);

        if !extract_path.exists() {
            let nar_url = format!("{}/{}", self.cache_base, narinfo.url);
            let mut compressed = Vec::new();
            self.http
                .get_stream_auth(&nar_url, self.token, &mut compressed)?;
            let uncompressed =
                decompress(&narinfo.compression, &compressed).map_err(NixFetchError::NarExtract)?;
            verify_nar_hash(&uncompressed, &narinfo.nar_hash)?;

            // Create the parent so extract_nar can write the store node.
            // The NAR extractor handles creation of the node itself.
            let parent = extract_path
                .parent()
                .ok_or_else(|| NixFetchError::NarExtract("store path has no parent".to_string()))?;
            std::fs::create_dir_all(parent).map_err(|e| {
                NixFetchError::NarExtract(format!("failed to create store parent dir: {e}"))
            })?;
            extract_nar(std::io::Cursor::new(uncompressed), &extract_path)?;
        }

        // Recursively fetch all transitive dependencies.
        // References are bare basenames ("hash-name-ver"), not full /nix/store/ paths.
        for dep in &narinfo.references {
            let basename = dep.strip_prefix("/nix/store/").unwrap_or(dep);
            if let Some((dep_hash, _)) = basename.split_once('-') {
                self.build_with_closure(dep_hash, dest_dir, visited)?;
            }
        }

        Ok(())
    }

    fn fetch_narinfo(&self, sri: &str) -> Result<NarInfo, NixFetchError> {
        // Derive the Nix store path hash (Nix base-32 of truncated SHA-256 of
        // the source fingerprint) — this is the 32-char prefix in the .narinfo URL.
        // Using sri_to_hex directly as the URL was wrong (issue #80).
        let store_hash = nix_hash::nar_hash_to_store_path_hash(sri)?;
        let narinfo_url = format!("{}/{store_hash}.narinfo", self.cache_base);
        let narinfo_bytes = self.http.get_bytes_auth(&narinfo_url, self.token)?;
        let narinfo_text =
            String::from_utf8(narinfo_bytes).map_err(|e| NixFetchError::NarInfoParse {
                hash: store_hash.clone(),
                message: e.to_string(),
            })?;
        NarInfo::parse(&store_hash, &narinfo_text)
    }
}

fn verify_nar_hash(nar_bytes: &[u8], nar_hash_nix_base32: &str) -> Result<(), NixFetchError> {
    let expected = nix_hash::nix_base32_decode(nar_hash_nix_base32)
        .map_err(|e| NixFetchError::NarExtract(format!("invalid NarHash encoding: {e}")))?;
    let actual = {
        use sha2::Digest;
        sha2::Sha256::digest(nar_bytes).to_vec()
    };
    if actual != expected {
        return Err(NixFetchError::NarExtract(format!(
            "NAR integrity check failed: expected {} but got {}",
            hex::encode(&expected),
            hex::encode(&actual),
        )));
    }
    Ok(())
}

fn decompress(compression: &Compression, data: &[u8]) -> Result<Vec<u8>, String> {
    match compression {
        Compression::Xz => {
            let mut out = Vec::new();
            let mut decoder = xz2::read::XzDecoder::new(data);
            std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        Compression::Bzip2 => {
            let mut out = Vec::new();
            let mut decoder = bzip2::read::BzDecoder::new(data);
            std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        Compression::None => Ok(data.to_vec()),
        Compression::Zstd => {
            let mut out = Vec::new();
            let mut decoder = zstd::Decoder::new(data).map_err(|e| e.to_string())?;
            std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_nar_hash_accepts_matching_hash() {
        // Verifies that verify_nar_hash returns Ok when sha256(bytes) matches the
        // Nix base-32 encoded hash. Would fail if the SHA-256 computation or
        // Nix base-32 decode is wrong.
        let data = b"test NAR bytes";
        let hash_bytes = {
            use sha2::Digest;
            sha2::Sha256::digest(data).to_vec()
        };
        let encoded = nix_hash::nix_base32_encode(&hash_bytes);
        let result = verify_nar_hash(data, &encoded);
        assert!(
            result.is_ok(),
            "verify_nar_hash must return Ok when hash matches, got: {result:?}"
        );
    }

    #[test]
    fn test_verify_nar_hash_rejects_mismatched_hash() {
        // Verifies that verify_nar_hash returns NarExtract(…) when the bytes don't
        // match the declared hash — the core integrity check.
        let data = b"real NAR bytes";
        let wrong_data = b"different bytes";
        let hash_bytes = {
            use sha2::Digest;
            sha2::Sha256::digest(wrong_data).to_vec()
        };
        let encoded = nix_hash::nix_base32_encode(&hash_bytes);
        let result = verify_nar_hash(data, &encoded);
        assert!(
            matches!(result, Err(NixFetchError::NarExtract(_))),
            "verify_nar_hash must return NarExtract when bytes do not match hash"
        );
    }

    #[test]
    fn test_verify_nar_hash_rejects_invalid_base32_encoding() {
        // Verifies that malformed hash strings produce an error rather than panicking.
        let result = verify_nar_hash(b"data", "not-valid-base32!!!");
        assert!(
            matches!(result, Err(NixFetchError::NarExtract(_))),
            "verify_nar_hash must return NarExtract for invalid base-32 input"
        );
    }
}

#[cfg(test)]
mod tests_build_store_path {
    use super::*;
    use justpkg_pkg::JustpkgError;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    struct PanicClient;
    impl HttpClient for PanicClient {
        fn get_bytes(&self, _url: &str) -> Result<Vec<u8>, JustpkgError> {
            panic!("PanicClient: no HTTP request should be made for invalid store paths");
        }
        fn get_stream(
            &self,
            _url: &str,
            _out: &mut dyn std::io::Write,
        ) -> Result<u64, JustpkgError> {
            panic!("PanicClient: no HTTP request should be made for invalid store paths");
        }
    }

    #[test]
    fn test_build_store_path_rejects_path_without_nix_store_prefix() {
        let fetcher = NixFetcher {
            http: &PanicClient,
            cache_base: "https://cache.nixos.org",
            token: None,
        };
        let err = fetcher
            .build_store_path("/usr/local/abc123-curl-8.0", std::path::Path::new("/dest"))
            .unwrap_err();
        assert!(
            matches!(err, NixFetchError::NarExtract(_)),
            "must reject paths not under /nix/store/, got: {err:?}"
        );
    }

    #[test]
    fn test_build_store_path_rejects_path_without_hash_separator() {
        let fetcher = NixFetcher {
            http: &PanicClient,
            cache_base: "https://cache.nixos.org",
            token: None,
        };
        let err = fetcher
            .build_store_path("/nix/store/nohyphennamehere", std::path::Path::new("/dest"))
            .unwrap_err();
        assert!(
            matches!(err, NixFetchError::NarExtract(_)),
            "must reject paths missing the hash-name hyphen separator, got: {err:?}"
        );
    }

    // Two-node graph: "redis" (aaaa…) references "glibc" (bbbb…).
    // The client returns narinfo for each hash and a trivial 1-byte NAR for the
    // actual download.  build_with_closure must visit BOTH hashes — the bug was
    // that References were bare basenames so strip_prefix("/nix/store/") returned
    // None and the dep loop was a no-op.
    struct ClosureClient {
        fetched: Arc<Mutex<HashSet<String>>>,
    }
    impl ClosureClient {
        fn new() -> Self {
            Self {
                fetched: Arc::new(Mutex::new(HashSet::new())),
            }
        }
    }

    // A minimal valid NAR: "(" type regular contents "" ")"
    // Length-prefixed strings, 8-byte LE, padded to 8 bytes.
    fn minimal_nar() -> Vec<u8> {
        fn encode(s: &str) -> Vec<u8> {
            let len = s.len() as u64;
            let padded = (len as usize + 7) & !7;
            let mut v = len.to_le_bytes().to_vec();
            v.extend_from_slice(s.as_bytes());
            v.resize(8 + padded, 0);
            v
        }
        let magic = "nix-archive-1";
        let mut out = Vec::new();
        out.extend(encode(magic));
        out.extend(encode("("));
        out.extend(encode("type"));
        out.extend(encode("regular"));
        // contents: empty
        out.extend(encode("contents"));
        out.extend(0u64.to_le_bytes()); // 0-byte body
        out.extend(encode(")"));
        out
    }

    impl HttpClient for ClosureClient {
        fn get_bytes(&self, url: &str) -> Result<Vec<u8>, JustpkgError> {
            // narinfo requests: /<hash>.narinfo
            let hash = url
                .split('/')
                .last()
                .unwrap()
                .trim_end_matches(".narinfo")
                .to_string();
            self.fetched
                .lock()
                .unwrap()
                .insert(format!("narinfo:{hash}"));

            let (store_basename, references) = if hash.starts_with('a') {
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-redis-7.2.7",
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-glibc-2.40-66",
                )
            } else {
                ("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-glibc-2.40-66", "")
            };

            // Build a real compressed NAR so decompression + hash verification don't fail.
            // We use Compression: none to skip decompression complexity.
            let nar_bytes = minimal_nar();
            let nar_hash_bytes: Vec<u8> = {
                use sha2::Digest as _;
                sha2::Sha256::digest(&nar_bytes).to_vec()
            };
            use crate::spi::nix_hash::nix_base32_encode;
            let nar_hash = nix_base32_encode(&nar_hash_bytes);
            let file_hash = hex::encode(&nar_hash_bytes);

            let narinfo = format!(
                "StorePath: /nix/store/{store_basename}\n\
                 URL: nar/{hash}.nar\n\
                 Compression: none\n\
                 FileHash: sha256:{file_hash}\n\
                 FileSize: {file_size}\n\
                 NarHash: sha256:{nar_hash}\n\
                 NarSize: {nar_size}\n\
                 References: {references}\n",
                file_size = nar_bytes.len(),
                nar_size = nar_bytes.len(),
            );
            Ok(narinfo.into_bytes())
        }

        fn get_stream(&self, url: &str, out: &mut dyn std::io::Write) -> Result<u64, JustpkgError> {
            // NAR download requests: /nar/<hash>.nar
            let hash = url
                .split('/')
                .last()
                .unwrap()
                .trim_end_matches(".nar")
                .to_string();
            self.fetched.lock().unwrap().insert(format!("nar:{hash}"));
            let nar = minimal_nar();
            let n = nar.len() as u64;
            out.write_all(&nar).map_err(JustpkgError::Io)?;
            Ok(n)
        }
    }

    #[test]
    fn test_build_with_closure_fetches_transitive_deps_via_bare_basename_references() {
        // Regression test: build_with_closure previously silently skipped References
        // because they are bare basenames ("hash-name") and the code did
        // strip_prefix("/nix/store/") which always returned None.
        let client = ClosureClient::new();
        let dest = tempfile::TempDir::new().unwrap();
        let fetcher = NixFetcher {
            http: &client,
            cache_base: "http://cache",
            token: None,
        };

        fetcher
            .build_store_path(
                "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-redis-7.2.7",
                dest.path(),
            )
            .expect("build_store_path must succeed");

        let fetched = client.fetched.lock().unwrap().clone();
        assert!(
            fetched
                .iter()
                .any(|s| s.contains("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")),
            "glibc (transitive dep via bare basename reference) must be fetched; got: {fetched:?}"
        );
    }
}
