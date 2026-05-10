// Integration tests for install_packages and generate_root_layout.
//
// Network I/O is replaced by an in-process TestHttpClient that either fails
// immediately or records calls.  This lets us verify the logic of the installer
// (manifest lookup, error propagation, early-exit on first failure) without any
// real network dependency.
//
// generate_root_layout tests use real tempdir I/O to verify symlinks are created
// correctly without touching the network at all.
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[cfg(unix)]
use swe_justpkg_vminit::generate_root_layout;
use swe_justpkg_vminit::{install_packages, parse_manifest, VminitInstallError};

// ── Test stubs ──────────────────────────────────────────────────────────────

/// Always returns an HTTP error so NixFetcher never reaches NAR extraction.
struct FailingHttpClient;

impl justpkg_pkg::HttpClient for FailingHttpClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
}

/// Counts how many times `get_bytes` is called so we can assert that the
/// installer exits early on the first error and does not continue to the next
/// package.
struct CountingHttpClient {
    call_count: Arc<AtomicU32>,
}

impl justpkg_pkg::HttpClient for CountingHttpClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 503,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Store paths use real 32-char Nix-base32 hashes so `build_store_path`
/// proceeds past the validation step and actually calls the HTTP client.
const TWO_ENTRY_MANIFEST: &str = r#"{
    "packages": {
        "curl": "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-curl-8.10.1",
        "git":  "/nix/store/bbbbccccddddeeeeffffgggg00001111-git-2.46.0"
    }
}"#;

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_install_packages_empty_names_list_succeeds_without_calling_http() {
    // An empty names slice must return Ok without ever touching the HTTP client.
    // This proves the installer does not speculatively fetch when there is nothing to do.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &[], dir.path(), &[]);

    assert!(result.is_ok(), "empty names list must succeed");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "HTTP must not be called when names list is empty"
    );
}

#[test]
fn test_install_packages_name_not_in_manifest_returns_package_not_found() {
    // A name that is absent from the manifest must return PackageNotFound
    // *before* any HTTP call is made.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["unknown-pkg"], dir.path(), &[]);

    assert!(result.is_err(), "missing name must return an error");
    assert!(
        matches!(
            result.unwrap_err(),
            VminitInstallError::PackageNotFound { name } if name == "unknown-pkg"
        ),
        "error variant must be PackageNotFound with the correct name"
    );
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        0,
        "HTTP must not be called when the manifest lookup fails"
    );
}

#[test]
fn test_install_packages_valid_entry_reaches_http_and_propagates_fetch_error() {
    // When the manifest lookup succeeds the installer must call the HTTP client.
    // Here the client always fails, so we get FetchFailed.  This proves the code
    // path from manifest lookup → NixFetcher → error propagation is wired correctly.
    let http = FailingHttpClient;
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl"], dir.path(), &[]);

    assert!(result.is_err(), "HTTP failure must propagate as error");
    assert!(
        matches!(result.unwrap_err(), VminitInstallError::FetchFailed { name, .. } if name == "curl"),
        "error variant must be FetchFailed with name=curl"
    );
}

#[test]
fn test_install_packages_stops_on_first_error() {
    // Given two valid packages, the installer must stop at the first HTTP failure
    // and not attempt the second package.  We count HTTP calls: must be exactly 1.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let result = install_packages(&http, &manifest, &["curl", "git"], dir.path(), &[]);

    assert!(
        result.is_err(),
        "first HTTP failure must abort the whole call"
    );
    // NixFetcher calls get_bytes once per package (for the narinfo).
    // Because we stop on first error, exactly 1 HTTP call must happen.
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "installer must stop after first fetch failure, not continue to second package"
    );
}

// ── Substituter fallback tests ───────────────────────────────────────────────

/// Records which base URLs were tried and returns 404 for all of them.
struct RecordingNotFoundClient {
    tried_urls: Arc<std::sync::Mutex<Vec<String>>>,
}

impl justpkg_pkg::HttpClient for RecordingNotFoundClient {
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, justpkg_pkg::JustpkgError> {
        self.tried_urls.lock().unwrap().push(url.to_string());
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }
    fn get_stream(
        &self,
        url: &str,
        _dest: &mut dyn std::io::Write,
    ) -> Result<u64, justpkg_pkg::JustpkgError> {
        Err(justpkg_pkg::JustpkgError::Http {
            url: url.to_string(),
            status: 404,
        })
    }
}

#[test]
fn test_install_packages_skips_404_substituter_and_tries_next() {
    // When the first substituter returns 404 the installer must advance to the
    // second substituter.  Both return 404 here so we verify the URL of the
    // second narinfo request to confirm fallback happened.
    let tried = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let http = RecordingNotFoundClient {
        tried_urls: Arc::clone(&tried),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let subs = vec![
        justpkg_config::SubstituterConfig::new("https://private.cache.example"),
        justpkg_config::SubstituterConfig::new("https://cache.nixos.org"),
    ];

    let _ = install_packages(&http, &manifest, &["curl"], dir.path(), &subs);

    let urls = tried.lock().unwrap().clone();
    let tried_private = urls.iter().any(|u| u.contains("private.cache.example"));
    let tried_public = urls.iter().any(|u| u.contains("cache.nixos.org"));
    assert!(
        tried_private,
        "first substituter (private.cache.example) must be tried; got: {urls:?}"
    );
    assert!(
        tried_public,
        "second substituter (cache.nixos.org) must be tried after 404; got: {urls:?}"
    );
}

#[test]
fn test_install_packages_propagates_non_404_error_without_fallback() {
    // A 503 from the first substituter must NOT trigger fallback — only 404 does.
    // We count narinfo requests: if fallback happened the count would be 2.
    let call_count = Arc::new(AtomicU32::new(0));
    let http = CountingHttpClient {
        call_count: Arc::clone(&call_count),
    };
    let manifest = parse_manifest(TWO_ENTRY_MANIFEST).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let subs = vec![
        justpkg_config::SubstituterConfig::new("https://private.cache.example"),
        justpkg_config::SubstituterConfig::new("https://cache.nixos.org"),
    ];

    let result = install_packages(&http, &manifest, &["curl"], dir.path(), &subs);

    assert!(result.is_err(), "503 must propagate as error");
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "503 must abort immediately without trying the second substituter"
    );
}

// ── generate_root_layout tests ───────────────────────────────────────────────
//
// These tests exercise the root layout generator directly using real tempdir
// I/O. No network is involved. We manually create fake store path trees in a
// temp directory to simulate what NixFetcher::build_store_path would produce.

/// Helper: create a fake extracted store tree under `dest_dir` for `store_path`.
/// Creates `<dest_dir>/<store_path>/bin/<binary>` as a regular file.
#[cfg(unix)]
fn create_fake_store_bin(dest_dir: &std::path::Path, store_path: &str, binaries: &[&str]) {
    let bin_dir = dest_dir
        .join(store_path.trim_start_matches('/'))
        .join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    for &b in binaries {
        std::fs::write(bin_dir.join(b), b"#!/bin/sh\nexec true\n").unwrap();
    }
}

/// generate_root_layout creates dest_dir/bin/ and populates it with symlinks
/// pointing to the absolute nix store path.
///
/// Bug it catches: if the function uses the host path (dest_dir/nix/store/...)
/// as the symlink target instead of the absolute VM path (/nix/store/...), the
/// symlinks will resolve on the host but break inside the chroot. If it forgets
/// to create dest_dir/bin/ entirely, vminit's access("/rootfs/bin") check fails.
#[test]
#[cfg(unix)]
fn test_generate_root_layout_creates_bin_with_symlinks_to_store() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path();

    let store_path = "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-curl-8.10.1";
    create_fake_store_bin(dest, store_path, &["curl", "curl-config"]);

    let manifest_text =
        r#"{"packages": {"curl": "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-curl-8.10.1"}}"#;
    let manifest = parse_manifest(manifest_text).unwrap();

    generate_root_layout(&manifest, &["curl"], dest).unwrap();

    // bin/ must exist — this is what vminit checks.
    assert!(dest.join("bin").is_dir(), "bin/ must be created");

    // Each binary must appear as a symlink.
    for bin in &["curl", "curl-config"] {
        let link = dest.join("bin").join(bin);
        assert!(link.is_symlink(), "bin/{bin} must be a symlink");

        let target = std::fs::read_link(&link).unwrap();
        let expected = format!("{store_path}/bin/{bin}");
        assert_eq!(
            target.to_string_lossy(),
            expected,
            "bin/{bin} must point to the absolute VM nix store path"
        );
    }
}

/// Packages without a bin/ directory are silently skipped — root layout
/// generation only exposes binaries, not every package.
///
/// Bug it catches: if the function errors on a package that has no bin/ dir
/// (e.g. a data-only package like tzdata), the installer would fail for valid
/// manifests that mix executable and data packages.
#[test]
#[cfg(unix)]
fn test_generate_root_layout_skips_package_with_no_bin_dir() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path();

    // tzdata has no bin/ directory — just data files under share/.
    let store_path = "/nix/store/ccccddddeeeeffffgggg0000aaaabbbb-tzdata-2025b";
    let share_dir = dest.join(store_path.trim_start_matches('/')).join("share");
    std::fs::create_dir_all(&share_dir).unwrap();

    let manifest_text =
        r#"{"packages": {"tzdata": "/nix/store/ccccddddeeeeffffgggg0000aaaabbbb-tzdata-2025b"}}"#;
    let manifest = parse_manifest(manifest_text).unwrap();

    let result = generate_root_layout(&manifest, &["tzdata"], dest);

    assert!(result.is_ok(), "missing bin/ must not be an error");
    // bin/ is still created (the directory itself satisfies vminit's check).
    assert!(
        dest.join("bin").is_dir(),
        "bin/ must be created even if empty"
    );
    assert_eq!(
        std::fs::read_dir(dest.join("bin")).unwrap().count(),
        0,
        "bin/ must be empty when no binaries were installed"
    );
}

/// First package in the names list wins when two packages provide the same
/// binary name — the second is skipped silently (no error, no overwrite).
///
/// Bug it catches: overwriting the first package's symlink with the second's
/// would silently change which binary is called. Error-on-collision would make
/// it impossible to install pairs like (postgresql, su-exec) if they ever share
/// a name, so silent-skip is the right behavior.
#[test]
#[cfg(unix)]
fn test_generate_root_layout_first_package_wins_on_name_collision() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path();

    let store_a = "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-pkga-1.0";
    let store_b = "/nix/store/bbbbccccddddeeeeffffgggg0000aaaa-pkgb-1.0";
    create_fake_store_bin(dest, store_a, &["shared-bin"]);
    create_fake_store_bin(dest, store_b, &["shared-bin"]);

    let manifest_text = r#"{
        "packages": {
            "pkga": "/nix/store/aaaabbbbccccddddeeeeffffgggg0000-pkga-1.0",
            "pkgb": "/nix/store/bbbbccccddddeeeeffffgggg0000aaaa-pkgb-1.0"
        }
    }"#;
    let manifest = parse_manifest(manifest_text).unwrap();

    generate_root_layout(&manifest, &["pkga", "pkgb"], dest).unwrap();

    let link = dest.join("bin").join("shared-bin");
    let target = std::fs::read_link(&link).unwrap();
    assert!(
        target.to_string_lossy().contains("pkga"),
        "first package (pkga) must win on collision; got {:?}",
        target
    );
}
