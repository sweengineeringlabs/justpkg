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

        let narinfo_text = format!(
            "StorePath: /nix/store/test-pkg\n\
             URL: {nar_url_suffix}\n\
             Compression: zstd\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {file_size}\n\
             NarHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
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
    // The dest for a root-regular-file NAR must be a non-existent path because
    // extract_nar writes the file at `dest_dir` itself (the root node IS the file).
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("output");

    let expected_content = b"zstd test payload";
    let client = ZstdMockClient::new(expected_content);
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("zstd-compressed NAR fetch and extract must succeed");

    let actual = std::fs::read(&dest).expect("extracted file must exist at dest path");
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

        let narinfo_text = format!(
            "StorePath: /nix/store/test-pkg\n\
             URL: {nar_url_suffix}\n\
             Compression: bzip2\n\
             FileHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
             FileSize: {file_size}\n\
             NarHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
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
    // @covers: decompress::Bzip2 arm (fetcher.rs)
    //
    // Regression guard: a previous bug used GzDecoder for Bzip2 compression,
    // which would silently produce a decompression error or wrong bytes.
    // This test would fail if BzDecoder were replaced with any other decoder.
    let parent = tempfile::tempdir().unwrap();
    let dest = parent.path().join("output");

    let expected_content = b"bzip2 test payload";
    let client = Bzip2MockClient::new(expected_content);
    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &client };

    fetcher
        .build(&lock, &dest)
        .expect("bzip2-compressed NAR fetch and extract must succeed");

    let actual = std::fs::read(&dest).expect("extracted file must exist at dest path");
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
        // extract_from_cas now derives the CAS key from narinfo.FileHash instead of
        // re-downloading the bytes. Use the real SHA-256 so CAS lookups match.
        let file_hash_hex = cas::Digest::from_bytes(cas::Algorithm::Sha256, &nar_bytes)
            .hex()
            .to_string();
        let narinfo_text = format!(
            "StorePath: /nix/store/stub-pkg\n\
             URL: nar/stub.nar\n\
             Compression: none\n\
             FileHash: sha256:{file_hash_hex}\n\
             FileSize: {sz}\n\
             NarHash: sha256:0000000000000000000000000000000000000000000000000000000000000000\n\
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
    // @covers: NixFetcher::extract_from_cas
    //
    // Given a MemCas pre-populated with a valid (uncompressed) NAR and an HTTP
    // stub that can serve the narinfo, extract_from_cas must produce the expected
    // file under dest_dir. This test would fail if extraction is skipped, the
    // wrong CAS blob is read, or the NAR parser is broken.
    let nar = build_single_file_nar(b"extract-from-cas content");
    let http = NoneCompressionStubClient::new(nar.clone());

    // Pre-populate the CAS exactly as fetch_to_cas would have done.
    let cas_store = MemCas::new();
    cas_store.put(&nar).expect("pre-populate CAS");

    let lock = FlakeLock::from_json(MINIMAL_FLAKE_LOCK).unwrap();
    let fetcher = NixFetcher { http: &http };
    // The single-file NAR writes the root node as a file at dest_dir itself.
    let parent = tempfile::tempdir().unwrap();
    let dest_file = parent.path().join("out");

    fetcher
        .extract_from_cas(&lock, &cas_store, &dest_file)
        .expect("extract_from_cas must succeed when CAS contains the blob");

    assert!(
        dest_file.exists(),
        "extract_from_cas must produce a file at the destination path"
    );
    let content = std::fs::read(&dest_file).unwrap();
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
