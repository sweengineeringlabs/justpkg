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

const CACHE_BASE: &str = "https://cache.nixos.org";

pub struct NixFetcher<'a> {
    pub http: &'a dyn HttpClient,
}

impl<'a> NixFetcher<'a> {
    /// Fetch and extract all locked nodes from a `flake.lock` into `dest_dir`.
    pub fn build(&self, lock: &FlakeLock, dest_dir: &Path) -> Result<(), NixFetchError> {
        for (name, node) in lock.locked_nodes() {
            let sri = &node.locked.nar_hash;
            eprintln!("  fetch {name}");
            self.fetch_nar_by_sri(sri, dest_dir)?;
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
            let nar_url = format!("{CACHE_BASE}/{}", narinfo.url);

            let mut compressed_bytes = Vec::new();
            self.http.get_stream(&nar_url, &mut compressed_bytes)?;

            let digest = cas
                .put(&compressed_bytes)
                .map_err(|e| NixFetchError::NarExtract(e.to_string()))?;

            result.insert(name.clone(), digest);
        }
        Ok(result)
    }

    /// Read each locked node's compressed NAR from `cas`, decompress, and
    /// extract to `dest_dir`.
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
                hex::decode(&narinfo.file_hash).map_err(|e| {
                    NixFetchError::NarExtract(format!("invalid FileHash hex: {e}"))
                })?
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

            extract_nar(Cursor::new(uncompressed), dest_dir)?;
        }
        Ok(())
    }

    fn fetch_narinfo(&self, sri: &str) -> Result<NarInfo, NixFetchError> {
        // Derive the Nix store path hash (Nix base-32 of truncated SHA-256 of
        // the source fingerprint) — this is the 32-char prefix in the .narinfo URL.
        // Using sri_to_hex directly as the URL was wrong (issue #80).
        let store_hash = nix_hash::nar_hash_to_store_path_hash(sri)?;
        let narinfo_url = format!("{CACHE_BASE}/{store_hash}.narinfo");
        let narinfo_bytes = self.http.get_bytes(&narinfo_url)?;
        let narinfo_text =
            String::from_utf8(narinfo_bytes).map_err(|e| NixFetchError::NarInfoParse {
                hash: store_hash.clone(),
                message: e.to_string(),
            })?;
        NarInfo::parse(&store_hash, &narinfo_text)
    }

    fn fetch_nar_by_sri(&self, sri: &str, dest_dir: &Path) -> Result<(), NixFetchError> {
        let narinfo = self.fetch_narinfo(sri)?;
        let nar_url = format!("{CACHE_BASE}/{}", narinfo.url);

        // Fetch compressed NAR (cache by FileHash)
        let mut nar_bytes = Vec::new();
        self.http.get_stream(&nar_url, &mut nar_bytes)?;

        // Decompress
        let uncompressed =
            decompress(&narinfo.compression, &nar_bytes).map_err(NixFetchError::NarExtract)?;

        // Extract
        extract_nar(std::io::Cursor::new(uncompressed), dest_dir)?;

        Ok(())
    }
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
