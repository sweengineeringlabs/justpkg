use super::error::NixFetchError;

/// Parsed `.narinfo` file from `cache.nixos.org`.
#[derive(Debug, Clone)]
pub struct NarInfo {
    pub store_path: String,
    /// Relative URL of the compressed NAR, e.g. `nar/abc123.nar.xz`
    pub url: String,
    pub compression: Compression,
    /// SHA-256 of the compressed NAR file (used as CAS key)
    pub file_hash: String,
    pub file_size: u64,
    /// SHA-256 of the uncompressed NAR (verified after decompression)
    pub nar_hash: String,
    pub nar_size: u64,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compression {
    Xz,
    Zstd,
    Bzip2,
    None,
}

impl NarInfo {
    pub fn parse(hash: &str, text: &str) -> Result<Self, NixFetchError> {
        let mut store_path = String::new();
        let mut url = String::new();
        let mut compression = Compression::None;
        let mut file_hash = String::new();
        let mut file_size = 0u64;
        let mut nar_hash = String::new();
        let mut nar_size = 0u64;
        let mut references = Vec::new();

        for line in text.lines() {
            if let Some((key, val)) = line.split_once(": ") {
                match key {
                    "StorePath"   => store_path  = val.to_string(),
                    "URL"         => url         = val.to_string(),
                    "Compression" => compression = match val {
                        "xz"    => Compression::Xz,
                        "zstd"  => Compression::Zstd,
                        "bzip2" => Compression::Bzip2,
                        "none"  => Compression::None,
                        other   => return Err(NixFetchError::NarInfoParse {
                            hash: hash.to_string(),
                            message: format!("unknown compression: {other}"),
                        }),
                    },
                    "FileHash" => {
                        // format: "sha256:<hex-or-base32>"
                        file_hash = val.trim_start_matches("sha256:").to_string();
                    }
                    "FileSize"   => file_size = val.parse().unwrap_or(0),
                    "NarHash"    => {
                        nar_hash = val.trim_start_matches("sha256:").to_string();
                    }
                    "NarSize"    => nar_size = val.parse().unwrap_or(0),
                    "References" => {
                        references = val.split_whitespace()
                            .map(str::to_string)
                            .collect();
                    }
                    _ => {}
                }
            }
        }

        if url.is_empty() {
            return Err(NixFetchError::NarInfoParse {
                hash: hash.to_string(),
                message: "missing URL field".to_string(),
            });
        }

        Ok(NarInfo { store_path, url, compression, file_hash, file_size, nar_hash, nar_size, references })
    }
}
