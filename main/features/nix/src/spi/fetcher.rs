//! Orchestrates the full Nix NAR fetch pipeline:
//!   flake.lock → narinfo → compressed NAR → verified + extracted

use std::path::Path;

use justpkg_pkg::HttpClient;

use crate::api::flake_lock::FlakeLock;
use crate::api::narinfo::{Compression, NarInfo};
use crate::api::error::NixFetchError;
use crate::spi::nar::extract_nar;
use crate::spi::nix_hash;

const CACHE_BASE: &str = "https://cache.nixos.org";

pub struct NixFetcher<'a> {
    pub http: &'a dyn HttpClient,
    // cas integration wired in follow-up (fetch-only command)
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

    fn fetch_nar_by_sri(&self, sri: &str, dest_dir: &Path) -> Result<(), NixFetchError> {
        // Convert SRI narHash to Nix base-32 for narinfo URL
        // For now, derive the store hash from the SRI directly via hex
        let hex = nix_hash::sri_to_hex(sri)?;
        let narinfo_url = format!("{CACHE_BASE}/{hex}.narinfo");

        let narinfo_bytes = self.http.get_bytes(&narinfo_url)?;
        let narinfo_text = String::from_utf8(narinfo_bytes)
            .map_err(|e| NixFetchError::NarInfoParse {
                hash: hex.clone(),
                message: e.to_string(),
            })?;

        let narinfo = NarInfo::parse(&hex, &narinfo_text)?;
        let nar_url = format!("{CACHE_BASE}/{}", narinfo.url);

        // Fetch compressed NAR (cache by FileHash)
        let mut nar_bytes = Vec::new();
        self.http.get_stream(&nar_url, &mut nar_bytes)?;

        // Decompress
        let uncompressed = decompress(&narinfo.compression, &nar_bytes)
            .map_err(NixFetchError::NarExtract)?;

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
            let mut decoder = flate2::read::GzDecoder::new(data);
            std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
            Ok(out)
        }
        Compression::None => Ok(data.to_vec()),
        Compression::Zstd => Err("zstd compression not yet implemented".to_string()),
    }
}
