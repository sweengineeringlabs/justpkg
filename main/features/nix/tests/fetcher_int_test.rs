// Integration tests for NixFetcher.
// Tests marked #[ignore] require network. Run with: cargo test -- --include-ignored
//
// The narinfo URL derivation (sri → hex → narinfo URL) is currently a known gap —
// see issue #80. These tests exercise the components that DO work.
use cas::{Cas, MemCas};
use justpkg_pkg::UreqClient;
use swe_justpkg_nix::{FlakeLock, NixFetchError, NixFetcher};

// ── NAR builder helpers (mirrored from nar_int_test) ─────────────────────────

fn write_nar_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    let pad = (8 - (s.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

/// Builds a minimal NAR that contains one regular file at the root with the
/// given content bytes.
fn build_single_file_nar(content: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    write_nar_str(&mut buf, "nix-archive-1");
    write_nar_str(&mut buf, "(");
    write_nar_str(&mut buf, "type");
    write_nar_str(&mut buf, "regular");
    write_nar_str(&mut buf, "contents");
    let len = content.len() as u64;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(content);
    let pad = (8 - (content.len() % 8)) % 8;
    buf.extend(std::iter::repeat(0u8).take(pad));
    write_nar_str(&mut buf, ")"); // read_regular exits
    write_nar_str(&mut buf, ")"); // read_nar_node closes
    buf
}

/// Compute Nix base-32 encoded SHA-256 of `data` — the format used in narinfo NarHash fields.
fn nix_base32_sha256(data: &[u8]) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(data).to_vec();
    nix_base32_encode(&hash)
}

/// Encode raw bytes in Nix's custom base-32 alphabet (mirrors nix_hash::nix_base32_encode).
fn nix_base32_encode(bytes: &[u8]) -> String {
    const NIX_BASE32_CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let out_len = (bytes.len() * 8 + 4) / 5;
    let mut out = Vec::with_capacity(out_len);
    for n in (0..out_len).rev() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;
        let c0 = bytes[i] as u32;
        let c1 = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        let c = ((c0 >> j) | (c1 << (8 - j))) & 0x1f;
        out.push(NIX_BASE32_CHARS[c as usize]);
    }
    String::from_utf8(out).expect("nix_base32_encode produced non-UTF-8")
}

const MINIMAL_FLAKE_LOCK: &str = r#"{
  "nodes": {
    "root": { "inputs": { "nixpkgs": "nixpkgs" } },
    "nixpkgs": {
      "locked": {
        "lastModified": 1700000000,
        "narHash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "NixOS", "repo": "nixpkgs",
        "rev": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "type": "github"
      },
      "inputs": {}
    }
  },
  "root": "root",
  "version": 7
}"#;

struct FailingClient;

impl justpkg_pkg::HttpClient for FailingClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 0,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 0,
        })
    }
}

#[test]
fn test_build_fails_on_unreachable_cache() {
    // Verifies the fetcher propagates HTTP errors rather than silently succeeding.
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let client = FailingClient;
    let fetcher = NixFetcher { http: &client };
    let result = fetcher.build(&lock, dir.path());
    assert!(result.is_err(), "HTTP failure must propagate as error");
    assert!(matches!(result.unwrap_err(), NixFetchError::Core(_)));
}

#[test]
#[ignore = "requires network"]
fn test_build_real_flake_lock_fetches_nar() {
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let http = UreqClient;
    let fetcher = NixFetcher { http: &http };
    fetcher.build(&lock, dir.path()).unwrap();
}

// ── Zstd decompression integration test ──────────────────────────────────────

/// Mock HTTP client that serves a narinfo with `Compression: zstd` and the
/// corresponding zstd-compressed NAR bytes.
///
/// The flake.lock in MINIMAL_FLAKE_LOCK uses the SRI
/// `sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=`.
/// `sri_to_hex` decodes the base-64 payload (32 zero bytes) to the 64-zero
/// hex string, so the narinfo URL the fetcher will request is:
///   https://cache.nixos.org/0000…0000.narinfo   (64 zeros)
///
/// This mock serves that narinfo, then serves zstd-compressed NAR bytes for
/// the NAR URL referenced inside it. This exercises the full Zstd decompression
/// path through NixFetcher without any network access.
struct ZstdMockClient {
    narinfo_text: String,
    nar_bytes: Vec<u8>,
    nar_url_suffix: String,
}

impl ZstdMockClient {
    fn new(file_content: &[u8]) -> Self {
        let raw_nar = build_single_file_nar(file_content);

        let mut compressed = Vec::new();
        let mut encoder =
            zstd::Encoder::new(&mut compressed, 0).expect("zstd encoder creation failed");
        std::io::copy(&mut std::io::Cursor::new(&raw_nar), &mut encoder)
            .expect("zstd encode failed");
        encoder.finish().expect("zstd finish failed");

        let nar_url_suffix = "nar/test.nar.zst".to_string();

        // NarHash must be the real SHA-256 of the uncompressed NAR in Nix base-32.
        let nar_hash = nix_base32_sha256(&raw_nar);

        let narinfo_text = format!(
            "StorePath: /nix/store/test-pkg\n\
             URL: {nar_url_suffix}\n\
             Compression: zstd\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {file_size}\n\
             NarHash: sha256:{nar_hash}\n\
             NarSize: {nar_size}\n\
             References:\n",
            file_size = compressed.len(),
            nar_size = raw_nar.len(),
        );

        ZstdMockClient {
            narinfo_text,
            nar_bytes: compressed,
            nar_url_suffix,
        }
    }
}

impl justpkg_pkg::HttpClient for ZstdMockClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            return Ok(self.narinfo_text.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }

    fn get_stream(
        &self,
        url: &str,
        writer: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        if url.ends_with(&self.nar_url_suffix) {
            writer
                .write_all(&self.nar_bytes)
                .map_err(|_| justpkg_pkg::JustpkgError::Http {
                    url: url.to_string(),
                    status: 0,
                })?;
            return Ok(self.nar_bytes.len() as u64);
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }
}

#[test]
fn test_build_zstd_nar_decompresses_and_extracts_correctly() {
    // @covers: decompress::Zstd arm (fetcher.rs) + store path layout (issue #2)
    //
    // After adding store path layout, the extracted file lands at
    // <dest>/nix/store/test-pkg, not at <dest> directly. The narinfo StorePath
    // is /nix/store/test-pkg so the basename is "test-pkg".
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("output");

    let expected_content = b"zstd test payload";
    let client = ZstdMockClient::new(expected_content);
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("zstd-compressed NAR fetch and extract must succeed");

    // Issue #2: file must be under nix/store/<basename>/, not flat at dest.
    let extracted = dest.join("nix").join("store").join("test-pkg");
    let actual = std::fs::read(&extracted).expect("extracted file must exist at nix/store/test-pkg");
    assert_eq!(
        actual, expected_content,
        "decompressed+extracted content must match original payload"
    );
}

// ── Bzip2 decompression integration test ─────────────────────────────────────

struct Bzip2MockClient {
    narinfo_text: String,
    nar_bytes: Vec<u8>,
    nar_url_suffix: String,
}

impl Bzip2MockClient {
    fn new(file_content: &[u8]) -> Self {
        let raw_nar = build_single_file_nar(file_content);

        let mut compressed = Vec::new();
        let mut encoder =
            bzip2::write::BzEncoder::new(&mut compressed, bzip2::Compression::best());
        std::io::copy(&mut std::io::Cursor::new(&raw_nar), &mut encoder)
            .expect("bzip2 encode failed");
        encoder.finish().expect("bzip2 finish failed");

        let nar_url_suffix = "nar/test.nar.bz2".to_string();

        // NarHash must be the real SHA-256 of the uncompressed NAR in Nix base-32.
        let nar_hash = nix_base32_sha256(&raw_nar);

        let narinfo_text = format!(
            "StorePath: /nix/store/test-pkg\n\
             URL: {nar_url_suffix}\n\
             Compression: bzip2\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {file_size}\n\
             NarHash: sha256:{nar_hash}\n\
             NarSize: {nar_size}\n\
             References:\n",
            file_size = compressed.len(),
            nar_size = raw_nar.len(),
        );

        Bzip2MockClient {
            narinfo_text,
            nar_bytes: compressed,
            nar_url_suffix,
        }
    }
}

impl justpkg_pkg::HttpClient for Bzip2MockClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            return Ok(self.narinfo_text.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }

    fn get_stream(
        &self,
        url: &str,
        writer: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        if url.ends_with(&self.nar_url_suffix) {
            writer
                .write_all(&self.nar_bytes)
                .map_err(|_| justpkg_pkg::JustpkgError::Http {
                    url: url.to_string(),
                    status: 0,
                })?;
            return Ok(self.nar_bytes.len() as u64);
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }
}

#[test]
fn test_build_bzip2_nar_decompresses_and_extracts_correctly() {
    // @covers: decompress::Bzip2 arm (fetcher.rs) + store path layout (issue #2)
    //
    // Regression guard: a previous bug used GzDecoder for Bzip2 compression,
    // which would silently produce a decompression error or wrong bytes.
    // This test would fail if BzDecoder were replaced with any other decoder.
    //
    // After adding store path layout, the extracted file lands at
    // <dest>/nix/store/test-pkg, not at <dest> directly.
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("output");

    let expected_content = b"bzip2 test payload";
    let client = Bzip2MockClient::new(expected_content);
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("bzip2-compressed NAR fetch and extract must succeed");

    // Issue #2: file must be under nix/store/<basename>/, not flat at dest.
    let extracted = dest.join("nix").join("store").join("test-pkg");
    let actual = std::fs::read(&extracted).expect("extracted file must exist at nix/store/test-pkg");
    assert_eq!(
        actual, expected_content,
        "decompressed+extracted content must match original bzip2 payload"
    );
}

// ── HTTP stub for fetch_to_cas / extract_from_cas tests ──────────────────────

/// Stub that serves a canned narinfo with `Compression: none` and returns the
/// given raw NAR bytes for the NAR URL it advertises.
///
/// Using `Compression: none` avoids pulling in a compression library in the
/// test and keeps the round-trip simple: bytes stored by fetch_to_cas are
/// identical to the bytes extract_from_cas decompresses.
struct NoneCompressionStubClient {
    narinfo_text: String,
    nar_bytes: Vec<u8>,
}

impl NoneCompressionStubClient {
    fn new(nar_bytes: Vec<u8>) -> Self {
        // extract_from_cas derives the CAS key from narinfo.FileHash — use the
        // real SHA-256 so CAS lookups match.
        let file_hash_hex = cas::Digest::from_bytes(cas::Algorithm::Sha256, &nar_bytes)
            .hex()
            .to_string();
        // NarHash must be the real SHA-256 of the NAR bytes in Nix base-32 so
        // verify_nar_hash (issue #3) accepts them.
        let nar_hash = nix_base32_sha256(&nar_bytes);
        let narinfo_text = format!(
            "StorePath: /nix/store/stub-pkg\n\
             URL: nar/stub.nar\n\
             Compression: none\n\
             FileHash: sha256:{file_hash_hex}\n\
             FileSize: {sz}\n\
             NarHash: sha256:{nar_hash}\n\
             NarSize: {sz}\n\
             References:\n",
            sz = nar_bytes.len(),
        );
        NoneCompressionStubClient {
            narinfo_text,
            nar_bytes,
        }
    }
}

impl justpkg_pkg::HttpClient for NoneCompressionStubClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            return Ok(self.narinfo_text.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }

    fn get_stream(
        &self,
        url: &str,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        if url.ends_with("nar/stub.nar") {
            dest.write_all(&self.nar_bytes)
                .map_err(|_| justpkg_pkg::JustpkgError::Http {
                    url: url.to_string(),
                    status: 0,
                })?;
            return Ok(self.nar_bytes.len() as u64);
        }
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }
}

// ── Tests for fetch_to_cas ────────────────────────────────────────────────────

#[test]
fn test_fetch_to_cas_stores_compressed_nar_in_cas() {
    // @covers: NixFetcher::fetch_to_cas
    //
    // When the HTTP stub serves a valid narinfo + NAR, fetch_to_cas must put
    // the bytes into the CAS and return a digest map with one entry for the
    // one locked node. This test would fail if fetch_to_cas skips cas.put,
    // returns an empty map, or stores different bytes than what was served.
    let nar = build_single_file_nar(b"fetch-to-cas test payload");
    let http = NoneCompressionStubClient::new(nar.clone());
    let cas = MemCas::new();
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &http };

    let map = fetcher
        .fetch_to_cas(&lock, &cas)
        .expect("fetch_to_cas must succeed when HTTP stub serves valid data");

    assert_eq!(
        map.len(),
        1,
        "one locked node must produce exactly one digest map entry"
    );
    let digest = map.values().next().expect("map must contain a digest");
    let stored = cas
        .get(digest)
        .expect("CAS must contain the NAR bytes under the returned digest");
    assert_eq!(
        stored, nar,
        "bytes stored in CAS must be identical to the bytes served by the stub"
    );
}

#[test]
fn test_fetch_to_cas_propagates_http_error() {
    // @covers: NixFetcher::fetch_to_cas
    //
    // When the HTTP client returns an error, fetch_to_cas must propagate it.
    // This test would fail if fetch_to_cas ignored the error or returned Ok.
    let cas = MemCas::new();
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher {
        http: &FailingClient,
    };

    let result = fetcher.fetch_to_cas(&lock, &cas);

    assert!(
        result.is_err(),
        "HTTP transport failure must cause fetch_to_cas to return Err"
    );
    assert!(
        matches!(result.unwrap_err(), NixFetchError::Core(_)),
        "error must be the Core (HTTP) variant"
    );
}

#[test]
fn test_fetch_to_cas_returned_digest_matches_sha256_of_stored_bytes() {
    // @covers: NixFetcher::fetch_to_cas
    //
    // The digest returned for a node must equal SHA-256(compressed NAR bytes).
    // This test would fail if fetch_to_cas returned a fabricated or wrong digest.
    let nar = build_single_file_nar(b"digest-identity payload");
    let expected_digest = cas::Digest::from_bytes(cas::Algorithm::Sha256, &nar);
    let http = NoneCompressionStubClient::new(nar);
    let cas_store = MemCas::new();
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &http };

    let map = fetcher.fetch_to_cas(&lock, &cas_store).unwrap();
    let actual_digest = map.values().next().unwrap().clone();

    assert_eq!(
        actual_digest, expected_digest,
        "returned digest must be the SHA-256 of the exact bytes stored in the CAS"
    );
}

// ── Tests for extract_from_cas ────────────────────────────────────────────────

#[test]
fn test_extract_from_cas_extracts_file_to_dest_dir() {
    // @covers: NixFetcher::extract_from_cas + store path layout (issue #2)
    //
    // Given a MemCas pre-populated with a valid (uncompressed) NAR and an HTTP
    // stub that can serve the narinfo, extract_from_cas must produce the expected
    // file under dest_dir/nix/store/<basename>. This test would fail if extraction
    // is skipped, the wrong CAS blob is read, or the NAR parser is broken.
    //
    // The stub's StorePath is /nix/store/stub-pkg so the file lands at
    // dest_dir/nix/store/stub-pkg.
    let nar = build_single_file_nar(b"extract-from-cas content");
    let http = NoneCompressionStubClient::new(nar.clone());

    // Pre-populate the CAS exactly as fetch_to_cas would have done.
    let cas_store = MemCas::new();
    cas_store.put(&nar).expect("pre-populate CAS");

    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &http };
    let parent = tempfile::tempdir().unwrap();
    let dest_dir = parent.path().join("root");

    fetcher
        .extract_from_cas(&lock, &cas_store, &dest_dir)
        .expect("extract_from_cas must succeed when CAS contains the blob");

    // Issue #2: file is at dest_dir/nix/store/stub-pkg, not dest_dir itself.
    let extracted = dest_dir.join("nix").join("store").join("stub-pkg");
    assert!(
        extracted.exists(),
        "extract_from_cas must produce a file at nix/store/stub-pkg under dest_dir"
    );
    let content = std::fs::read(&extracted).unwrap();
    assert_eq!(
        content, b"extract-from-cas content",
        "extracted file content must match the original NAR payload"
    );
}

#[test]
fn test_extract_from_cas_fails_when_blob_absent_from_cas() {
    // @covers: NixFetcher::extract_from_cas
    //
    // If the CAS does not contain the expected blob (e.g. the user ran extract
    // without first running fetch), extract_from_cas must return an error.
    // This test would fail if extract_from_cas silently succeeded on a missing blob.
    let nar = build_single_file_nar(b"some payload");
    let http = NoneCompressionStubClient::new(nar);
    let empty_cas = MemCas::new(); // nothing stored — simulates skipped fetch step
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &http };
    let parent = tempfile::tempdir().unwrap();

    let result = fetcher.extract_from_cas(&lock, &empty_cas, parent.path());

    assert!(
        result.is_err(),
        "extract_from_cas must return Err when the CAS blob is absent"
    );
    assert!(
        matches!(result.unwrap_err(), NixFetchError::NarExtract(_)),
        "error must be NarExtract when the CAS lookup fails"
    );
}

// ── Issue #2: store path layout tests ────────────────────────────────────────

#[test]
fn test_build_extracts_to_nix_store_path_subdir() {
    // @covers: issue #2 — NAR extraction must land at <dest>/nix/store/<basename>/
    //
    // The stub narinfo declares StorePath: /nix/store/abc123-curl-8.4.0.
    // After build(), the content must be at dest/nix/store/abc123-curl-8.4.0,
    // NOT flat at dest. This test fails if the fetcher extracts to dest directly.
    let raw_nar = build_single_file_nar(b"store path layout test");
    let nar_hash = nix_base32_sha256(&raw_nar);

    let narinfo_text = format!(
        "StorePath: /nix/store/abc123-curl-8.4.0\n\
         URL: nar/abc123.nar\n\
         Compression: none\n\
         FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
         FileSize: {sz}\n\
         NarHash: sha256:{nar_hash}\n\
         NarSize: {sz}\n\
         References:\n",
        sz = raw_nar.len(),
    );
    let nar_bytes = raw_nar.clone();

    struct StorePathStub {
        narinfo_text: String,
        nar_bytes: Vec<u8>,
    }

    impl justpkg_pkg::HttpClient for StorePathStub {
        fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
            if url.ends_with(".narinfo") {
                return Ok(self.narinfo_text.as_bytes().to_vec());
            }
            Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
        }
        fn get_stream(
            &self,
            url: &str,
            dest: &mut dyn std::io::Write,
        ) -> Result<u64, justpkg_pkg::JustpkgError> {
            if url.ends_with("nar/abc123.nar") {
                dest.write_all(&self.nar_bytes)
                    .map_err(|_| justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })?;
                return Ok(self.nar_bytes.len() as u64);
            }
            Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
        }
    }

    let client = StorePathStub { narinfo_text, nar_bytes };
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("out");
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("build must succeed with valid stub data");

    // The NAR root-regular node is written AT the extract_path, so the file
    // is dest/nix/store/abc123-curl-8.4.0.
    let expected_path = dest.join("nix").join("store").join("abc123-curl-8.4.0");
    assert!(
        expected_path.exists(),
        "extracted file must exist at dest/nix/store/abc123-curl-8.4.0, not flat at dest"
    );
    assert!(
        !dest.join("nix-archive-1").exists(),
        "no NAR raw file must be written flat at dest"
    );
    let content = std::fs::read(&expected_path).unwrap();
    assert_eq!(
        content, b"store path layout test",
        "extracted content must match the original NAR payload"
    );
}

// ── Issue #3: NAR integrity verification tests ────────────────────────────────

#[test]
fn test_build_rejects_tampered_nar_bytes() {
    // @covers: issue #3 — sha256(uncompressed NAR) must match narinfo NarHash.
    //
    // The stub advertises a correct NarHash for the original NAR but serves
    // a corrupted version (one byte flipped). build() must return NarExtract.
    // This test fails if verify_nar_hash is missing or not called.
    let raw_nar = build_single_file_nar(b"integrity check payload");
    // Compute hash of the REAL NAR — this is what the narinfo claims.
    let real_nar_hash = nix_base32_sha256(&raw_nar);

    // Corrupt the NAR bytes — flip the last byte.
    let mut corrupted_nar = raw_nar.clone();
    let last = corrupted_nar.len() - 1;
    corrupted_nar[last] ^= 0xff;

    let narinfo_text = format!(
        "StorePath: /nix/store/tampered-pkg\n\
         URL: nar/tampered.nar\n\
         Compression: none\n\
         FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
         FileSize: {sz}\n\
         NarHash: sha256:{real_nar_hash}\n\
         NarSize: {sz}\n\
         References:\n",
        sz = corrupted_nar.len(),
    );

    struct TamperedNarStub {
        narinfo_text: String,
        corrupted_nar: Vec<u8>,
    }

    impl justpkg_pkg::HttpClient for TamperedNarStub {
        fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
            if url.ends_with(".narinfo") {
                return Ok(self.narinfo_text.as_bytes().to_vec());
            }
            Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
        }
        fn get_stream(
            &self,
            url: &str,
            dest: &mut dyn std::io::Write,
        ) -> Result<u64, justpkg_pkg::JustpkgError> {
            if url.ends_with("nar/tampered.nar") {
                dest.write_all(&self.corrupted_nar)
                    .map_err(|_| justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })?;
                return Ok(self.corrupted_nar.len() as u64);
            }
            Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
        }
    }

    let client = TamperedNarStub { narinfo_text, corrupted_nar };
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let fetcher = NixFetcher { http: &client };

    let result = fetcher.build(&lock, parent.path());

    assert!(
        result.is_err(),
        "build must reject a NAR whose bytes do not match the declared NarHash"
    );
    assert!(
        matches!(result.unwrap_err(), NixFetchError::NarExtract(_)),
        "error variant must be NarExtract when integrity check fails"
    );
}

// ── Closure resolution tests (justpkg#1) ─────────────────────────────────────

/// Mock that serves two narinfos and two NARs:
///   - top-level: matches any `.narinfo` URL not containing the dep hash
///   - dep:       `dep0000000000000000000000000000`, no further references
struct ClosureMockClient {
    top_narinfo: String,
    dep_narinfo: String,
    top_nar: Vec<u8>,
    dep_nar: Vec<u8>,
}

impl ClosureMockClient {
    fn new() -> Self {
        let top_nar = build_single_file_nar(b"top-level content");
        let dep_nar = build_single_file_nar(b"dep content");
        let top_nar_hash = nix_base32_sha256(&top_nar);
        let dep_nar_hash = nix_base32_sha256(&dep_nar);

        let top_narinfo = format!(
            "StorePath: /nix/store/aaaa0000000000000000000000000000-top-1.0\n\
             URL: nar/top.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {sz}\n\
             NarHash: sha256:{top_nar_hash}\n\
             NarSize: {sz}\n\
             References: /nix/store/dep0000000000000000000000000000-dep-1.0\n",
            sz = top_nar.len(),
        );
        let dep_narinfo = format!(
            "StorePath: /nix/store/dep0000000000000000000000000000-dep-1.0\n\
             URL: nar/dep.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {sz}\n\
             NarHash: sha256:{dep_nar_hash}\n\
             NarSize: {sz}\n\
             References:\n",
            sz = dep_nar.len(),
        );
        ClosureMockClient { top_narinfo, dep_narinfo, top_nar, dep_nar }
    }
}

impl justpkg_pkg::HttpClient for ClosureMockClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            if url.contains("dep0000000000000000000000000000") {
                return Ok(self.dep_narinfo.as_bytes().to_vec());
            }
            return Ok(self.top_narinfo.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
    }

    fn get_stream(
        &self,
        url: &str,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        let bytes = if url.ends_with("nar/dep.nar") {
            &self.dep_nar
        } else if url.ends_with("nar/top.nar") {
            &self.top_nar
        } else {
            return Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 });
        };
        dest.write_all(bytes)
            .map_err(|_| justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })?;
        Ok(bytes.len() as u64)
    }
}

#[test]
fn test_build_fetches_transitive_dependency() {
    // @covers: NixFetcher::build_with_closure recursive dep resolution (justpkg#1)
    //
    // The top-level narinfo declares References: /nix/store/dep…-dep-1.0.
    // After build(), both the top-level store path and the dep store path must
    // exist under dest/nix/store/. This test fails if build() only fetches the
    // top-level node and ignores the References field.
    let client = ClosureMockClient::new();
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("root");
    let fetcher = NixFetcher { http: &client };

    fetcher.build(&lock, &dest).expect("build must succeed");

    let top_path = dest
        .join("nix")
        .join("store")
        .join("aaaa0000000000000000000000000000-top-1.0");
    let dep_path = dest
        .join("nix")
        .join("store")
        .join("dep0000000000000000000000000000-dep-1.0");

    assert!(
        top_path.exists(),
        "top-level store path must be present after build"
    );
    assert!(
        dep_path.exists(),
        "transitive dep store path must be present after build — closure resolution failed"
    );
}

/// Mock that serves narinfos for two packages (top + dep) but panics if the
/// NAR URL for the pre-existing top-level node is requested via get_stream.
struct NoRefetchMockClient {
    top_narinfo: String,
    dep_nar: Vec<u8>,
    dep_narinfo: String,
}

impl NoRefetchMockClient {
    fn new(pre_existing_basename: &str) -> Self {
        let dep_nar = build_single_file_nar(b"dep content");
        let dep_nar_hash = nix_base32_sha256(&dep_nar);

        let top_narinfo = format!(
            "StorePath: /nix/store/{pre_existing_basename}\n\
             URL: nar/top-should-not-be-fetched.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: 1\n\
             NarHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             NarSize: 1\n\
             References: /nix/store/dep0000000000000000000000000000-dep-1.0\n",
        );
        let dep_narinfo = format!(
            "StorePath: /nix/store/dep0000000000000000000000000000-dep-1.0\n\
             URL: nar/dep.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {sz}\n\
             NarHash: sha256:{dep_nar_hash}\n\
             NarSize: {sz}\n\
             References:\n",
            sz = dep_nar.len(),
        );
        NoRefetchMockClient { top_narinfo, dep_nar, dep_narinfo }
    }
}

impl justpkg_pkg::HttpClient for NoRefetchMockClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            if url.contains("dep0000000000000000000000000000") {
                return Ok(self.dep_narinfo.as_bytes().to_vec());
            }
            return Ok(self.top_narinfo.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
    }

    fn get_stream(
        &self,
        url: &str,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        if url.ends_with("nar/dep.nar") {
            dest.write_all(&self.dep_nar)
                .map_err(|_| justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })?;
            return Ok(self.dep_nar.len() as u64);
        }
        // The top-level store path already existed — its NAR must NOT be requested.
        panic!(
            "get_stream called for pre-existing store path — must be skipped: {url}"
        );
    }
}

#[test]
fn test_build_does_not_refetch_already_present_store_path() {
    // @covers: NixFetcher::build_with_closure skip-if-exists guard (justpkg#1)
    //
    // If the store path already exists on disk, build() must skip the NAR
    // download for that node. The mock panics if the NAR for the pre-existing
    // node is requested. This test fails if build() re-fetches unconditionally.
    let pre_existing_basename = "aaaa0000000000000000000000000000-top-1.0";
    let client = NoRefetchMockClient::new(pre_existing_basename);
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("root");

    // Pre-create the top-level store path so the fetcher must skip its NAR.
    let pre_existing_path = dest
        .join("nix")
        .join("store")
        .join(pre_existing_basename);
    std::fs::create_dir_all(&pre_existing_path)
        .expect("pre-creating store path directory must succeed");

    let fetcher = NixFetcher { http: &client };
    fetcher
        .build(&lock, &dest)
        .expect("build must succeed when top-level store path already exists");

    // The dep must still be fetched (it did not pre-exist).
    let dep_path = dest
        .join("nix")
        .join("store")
        .join("dep0000000000000000000000000000-dep-1.0");
    assert!(
        dep_path.exists(),
        "dep store path must be present — closure resolution must fetch absent deps"
    );
}

/// Mock that counts dep NAR fetches across two top-level iterations.
///
/// Both top-level nodes receive the same narinfo (same StorePath, same URL).
/// The second top-level's NAR is skipped because its store path already exists
/// after the first. Both reference the same dep, which must be fetched once.
struct SharedDepMockClient {
    top_narinfo: String,
    top_nar: Vec<u8>,
    dep_narinfo: String,
    dep_nar: Vec<u8>,
    dep_nar_fetch_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl SharedDepMockClient {
    fn new(counter: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        let top_nar = build_single_file_nar(b"shared top content");
        let dep_nar = build_single_file_nar(b"shared dep content");
        let top_hash = nix_base32_sha256(&top_nar);
        let dep_hash = nix_base32_sha256(&dep_nar);
        let shared_ref = "/nix/store/dep0000000000000000000000000000-dep-1.0";

        // Both top-a and top-b will receive this narinfo since the mock does not
        // distinguish their computed store hashes (which we don't know at test
        // compile time). After top-a is fetched, top-b finds its store path
        // already present and skips the NAR; both then reach the dep reference.
        let top_narinfo = format!(
            "StorePath: /nix/store/aaaa0000000000000000000000000000-top-shared\n\
             URL: nar/top.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {sz}\n\
             NarHash: sha256:{top_hash}\n\
             NarSize: {sz}\n\
             References: {shared_ref}\n",
            sz = top_nar.len(),
        );
        let dep_narinfo = format!(
            "StorePath: /nix/store/dep0000000000000000000000000000-dep-1.0\n\
             URL: nar/dep.nar\n\
             Compression: none\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {sz}\n\
             NarHash: sha256:{dep_hash}\n\
             NarSize: {sz}\n\
             References:\n",
            sz = dep_nar.len(),
        );
        SharedDepMockClient {
            top_narinfo,
            dep_narinfo,
            top_nar,
            dep_nar,
            dep_nar_fetch_count: counter,
        }
    }
}

impl justpkg_pkg::HttpClient for SharedDepMockClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        if url.ends_with(".narinfo") {
            if url.contains("dep0000000000000000000000000000") {
                return Ok(self.dep_narinfo.as_bytes().to_vec());
            }
            // Any other narinfo URL → serve the shared top-level narinfo.
            return Ok(self.top_narinfo.as_bytes().to_vec());
        }
        Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 })
    }

    fn get_stream(
        &self,
        url: &str,
        dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        let bytes = if url.ends_with("nar/dep.nar") {
            self.dep_nar_fetch_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            &self.dep_nar
        } else if url.ends_with("nar/top.nar") {
            &self.top_nar
        } else {
            return Err(justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 404 });
        };
        dest.write_all(bytes)
            .map_err(|_| justpkg_pkg::JustpkgError::Http { url: url.to_string(), status: 0 })?;
        Ok(bytes.len() as u64)
    }
}

const TWO_NODE_FLAKE_LOCK: &str = r#"{
  "nodes": {
    "root": { "inputs": { "top-a": "top-a", "top-b": "top-b" } },
    "top-a": {
      "locked": {
        "lastModified": 1700000000,
        "narHash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "test", "repo": "top-a",
        "rev": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "type": "github"
      },
      "inputs": {}
    },
    "top-b": {
      "locked": {
        "lastModified": 1700000001,
        "narHash": "sha256-BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "owner": "test", "repo": "top-b",
        "rev": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "type": "github"
      },
      "inputs": {}
    }
  },
  "root": "root",
  "version": 7
}"#;

#[test]
fn test_build_handles_shared_dependency_without_duplicate_fetch() {
    // @covers: NixFetcher::build_with_closure visited-set dedup (justpkg#1)
    //
    // Two top-level nodes that both reference the same dep store hash must result
    // in that dep's NAR being fetched exactly once. This test fails if the
    // visited-set is not used or is not shared across top-level loop iterations.
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let client = SharedDepMockClient::new(counter.clone());

    // TWO_NODE_FLAKE_LOCK has two locked nodes, each with a different SRI so
    // they derive different store hashes. The mock maps each hash to its narinfo
    // by URL fragment. Both narinfos reference the same dep hash.
    let lock = FlakeLock::from_json(TWO_NODE_FLAKE_LOCK)
        .expect("TWO_NODE_FLAKE_LOCK must parse successfully");
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("root");
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("build must succeed for two-node lock with shared dep");

    let dep_fetches = counter.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        dep_fetches, 1,
        "shared dep NAR must be fetched exactly once, but was fetched {dep_fetches} times"
    );
}
