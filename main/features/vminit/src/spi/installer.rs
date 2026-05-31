/// Fetch and extract each listed package using the manifest and `NixFetcher`.
///
/// For each `name` in `names`:
/// 1. Look up the Nix store path in `manifest.entries` — returns
///    [`VminitInstallError::PackageNotFound`] if absent.
/// 2. Try each substituter in `substituters` order.  A `NotFound` (HTTP 404)
///    or transport error (status 0 — cache unreachable) moves on to the next
///    substituter; any other error aborts.
/// 3. If no substituter has the path, returns the last error wrapped in
///    [`VminitInstallError::NotInAnyCache`].
///
/// Packages are fetched in parallel — one thread per top-level entry.
/// Transitive dependencies are fetched within each thread. The `extract_path.exists()`
/// check inside `NixFetcher::build_with_closure` makes extraction idempotent, so
/// shared transitive deps fetched by multiple threads concurrently are safe.
///
/// Passing an empty `substituters` slice falls back to `cache.nixos.org`.
use std::path::Path;

use justpkg_config::SubstituterConfig;
use justpkg_nix::{is_cache_miss, is_not_found, NixFetchError, NixFetcher, DEFAULT_CACHE_BASE};
use justpkg_pkg::HttpClient;

use crate::api::error::VminitInstallError;
use crate::api::manifest::PackageManifest;

pub fn install_packages(
    http: &dyn HttpClient,
    manifest: &PackageManifest,
    names: &[&str],
    dest_dir: &Path,
    substituters: &[SubstituterConfig],
) -> Result<(), VminitInstallError> {
    let default_sub;
    let subs: &[SubstituterConfig] = if substituters.is_empty() {
        default_sub = [SubstituterConfig::new(DEFAULT_CACHE_BASE)];
        &default_sub
    } else {
        substituters
    };

    // Resolve all store paths up-front so manifest errors surface before any I/O.
    let entries: Vec<(&str, &str)> = names
        .iter()
        .map(|&name| {
            manifest
                .entries
                .get(name)
                .map(|p| (name, p.as_str()))
                .ok_or_else(|| VminitInstallError::PackageNotFound {
                    name: name.to_string(),
                })
        })
        .collect::<Result<_, _>>()?;

    if entries.is_empty() {
        return Ok(());
    }

    // Fetch all packages in parallel — one thread per top-level entry.
    // `thread::scope` guarantees all threads finish before we return, and
    // lets us safely borrow `http`, `dest_dir`, and `subs` from this frame.
    let results: Vec<Result<(), VminitInstallError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .iter()
            .map(|&(name, store_path)| {
                scope.spawn(move || {
                    fetch_with_fallback(http, store_path, dest_dir, subs).map_err(|e| {
                        if is_not_found(&e) {
                            VminitInstallError::NotInAnyCache {
                                name: name.to_string(),
                                store_path: store_path.to_string(),
                                caches: subs.iter().map(|s| s.url.clone()).collect(),
                            }
                        } else {
                            VminitInstallError::FetchFailed {
                                name: name.to_string(),
                                source: e,
                            }
                        }
                    })
                })
            })
            .collect();

        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(VminitInstallError::FetchFailed {
                name: "<thread-panic>".to_string(),
                source: NixFetchError::NarExtract("fetch thread panicked".to_string()),
            })))
            .collect()
    });

    // Surface the first error (if any); all threads have already finished.
    results.into_iter().find(|r| r.is_err()).unwrap_or(Ok(()))
}

/// Try each substituter in order.  Falls through on HTTP 404 (package absent)
/// or transport errors / status 0 (cache unreachable — e.g. Attic not running).
/// Any other error (500, auth failure, etc.) aborts immediately.
fn fetch_with_fallback(
    http: &dyn HttpClient,
    store_path: &str,
    dest_dir: &Path,
    substituters: &[SubstituterConfig],
) -> Result<(), NixFetchError> {
    let mut last_err: Option<NixFetchError> = None;
    for sub in substituters {
        let fetcher = NixFetcher {
            http,
            cache_base: &sub.url,
            token: sub.token.as_deref(),
        };
        match fetcher.build_store_path(store_path, dest_dir) {
            Ok(()) => return Ok(()),
            Err(e) if is_cache_miss(&e) => {
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err
        .unwrap_or_else(|| NixFetchError::NarExtract("no substituters configured".to_string())))
}

/// Populate root-level `bin/` in `dest_dir` with symlinks into the Nix store.
///
/// For each package name in `names`, looks up its store path in `manifest`,
/// then walks `<dest_dir>/nix/store/<hash-name>/bin/` and creates
/// `<dest_dir>/bin/<entry>` → `<store_path>/bin/<entry>` symlinks.
///
/// The symlink target is the absolute Nix store path as it will appear inside
/// the VM's root filesystem (`/nix/store/...`), not a host-relative path.
///
/// Packages listed first take precedence — if two packages provide a binary
/// with the same name the second is skipped silently (no error, no overwrite).
///
/// This step is what makes vminit's `access("/rootfs/bin", F_OK)` check pass
/// and what allows the kernel to resolve PATH entries like `/bin/postgres` even
/// when the VM's `xkvm.conf` references full store paths for exec.
pub fn generate_root_layout(
    manifest: &PackageManifest,
    names: &[&str],
    dest_dir: &Path,
) -> Result<(), VminitInstallError> {
    let bin_dir = dest_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|source| VminitInstallError::RootLayoutFailed {
        reason: format!("create {}", bin_dir.display()),
        source,
    })?;

    for &name in names {
        let store_path = match manifest.entries.get(name) {
            Some(p) => p,
            None => continue,
        };

        // store_path is an absolute nix path like /nix/store/<hash>-<name>.
        // On the host it lives at dest_dir + that path (without the leading /).
        let store_bin = dest_dir.join(store_path.trim_start_matches('/').to_string() + "/bin");
        if !store_bin.is_dir() {
            continue;
        }

        let entries = std::fs::read_dir(&store_bin).map_err(|source| {
            VminitInstallError::RootLayoutFailed {
                reason: format!("read_dir {}", store_bin.display()),
                source,
            }
        })?;

        for entry in entries {
            let entry = entry.map_err(|source| VminitInstallError::RootLayoutFailed {
                reason: format!("read entry in {}", store_bin.display()),
                source,
            })?;

            let dest = bin_dir.join(entry.file_name());
            if dest.exists() || dest.is_symlink() {
                continue;
            }

            // Target is the absolute in-VM path, e.g.
            // /nix/store/<hash>-postgresql-16.9/bin/postgres
            let target = format!("{}/bin/{}", store_path, entry.file_name().to_string_lossy());

            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &dest).map_err(|source| {
                VminitInstallError::RootLayoutFailed {
                    reason: format!("symlink {} -> {}", dest.display(), target),
                    source,
                }
            })?;

            // On non-Unix hosts the bin/ directory is still created; symlinks
            // are deferred to the Linux build step.
            #[cfg(not(unix))]
            let _ = (&target, &dest);
        }
    }

    Ok(())
}
