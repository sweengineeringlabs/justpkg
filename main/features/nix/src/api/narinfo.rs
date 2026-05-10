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
                    "StorePath" => store_path = val.to_string(),
                    "URL" => url = val.to_string(),
                    "Compression" => {
                        compression = match val {
                            "xz" => Compression::Xz,
                            "zstd" => Compression::Zstd,
                            "bzip2" => Compression::Bzip2,
                            "none" => Compression::None,
                            other => {
                                return Err(NixFetchError::NarInfoParse {
                                    hash: hash.to_string(),
                                    message: format!("unknown compression: {other}"),
                                })
                            }
                        }
                    }
                    "FileHash" => {
                        // format: "sha256:<hex-or-base32>"
                        file_hash = val.trim_start_matches("sha256:").to_string();
                    }
                    "FileSize" => file_size = val.parse().unwrap_or(0),
                    "NarHash" => {
                        nar_hash = val.trim_start_matches("sha256:").to_string();
                    }
                    "NarSize" => nar_size = val.parse().unwrap_or(0),
                    "References" => {
                        references = val.split_whitespace().map(str::to_string).collect();
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

        Ok(NarInfo {
            store_path,
            url,
            compression,
            file_hash,
            file_size,
            nar_hash,
            nar_size,
            references,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_NARINFO: &str = "\
StorePath: /nix/store/9ap0znk8zci1j8cp06wysciy253yxk7c-redis-7.2.7
URL: nar/0v7r3pkgfzakwasv63g7rqni9n51bdq3qz9qhpgd4c2y3wpgp2j3.nar.xz
Compression: xz
FileHash: sha256:0v7r3pkgfzakwasv63g7rqni9n51bdq3qz9qhpgd4c2y3wpgp2j3
FileSize: 1234567
NarHash: sha256:1r4lm8a0g1w5kq1mnxcp3qiw15ynx2r3zzp1bqxvv6bwb2k7h20q
NarSize: 9876543
References: 5m9amsvvh2z8sl7jrnc87hzy21glw6k1-glibc-2.40-66 9ap0znk8zci1j8cp06wysciy253yxk7c-redis-7.2.7
";

    #[test]
    fn test_parse_narinfo_references_are_bare_basenames() {
        // narinfo References: field is space-separated basenames without /nix/store/ prefix.
        // Callers that strip_prefix("/nix/store/") will get None and silently skip the dep
        // unless they also handle the bare-basename case. This test captures that contract.
        let ni = NarInfo::parse("9ap0znk8zci1j8cp06wysciy253yxk7c", SAMPLE_NARINFO).unwrap();
        assert_eq!(ni.references.len(), 2);
        assert_eq!(
            ni.references[0],
            "5m9amsvvh2z8sl7jrnc87hzy21glw6k1-glibc-2.40-66"
        );
        assert_eq!(
            ni.references[1],
            "9ap0znk8zci1j8cp06wysciy253yxk7c-redis-7.2.7"
        );
        // Must NOT have /nix/store/ prefix
        assert!(
            !ni.references[0].starts_with("/nix/store/"),
            "References must be bare basenames, not full store paths"
        );
    }

    #[test]
    fn test_parse_narinfo_empty_references() {
        let text = "StorePath: /nix/store/abc-pkg\nURL: nar/x.nar.xz\nCompression: xz\n\
                    FileHash: sha256:abc\nFileSize: 1\nNarHash: sha256:def\nNarSize: 2\nReferences: \n";
        let ni = NarInfo::parse("abc", text).unwrap();
        assert!(
            ni.references.is_empty(),
            "empty References: line must produce empty Vec"
        );
    }
}
