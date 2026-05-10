pub use crate::api::error::VminitInstallError;
pub use crate::api::manifest::PackageManifest;
pub use crate::spi::installer::{generate_root_layout, install_packages};
pub use crate::spi::manifest::parse_manifest;

/// High-level installer: bundles an `HttpClient` reference, a parsed manifest,
/// and an optional substituter list so callers can invoke `install` without
/// threading all three through every call site.
pub struct VminitInstaller<'a> {
    http: &'a dyn justpkg_pkg::HttpClient,
    manifest: PackageManifest,
    substituters: Vec<justpkg_config::SubstituterConfig>,
}

impl<'a> VminitInstaller<'a> {
    /// Create a new installer using the default binary cache (`cache.nixos.org`).
    pub fn new(http: &'a dyn justpkg_pkg::HttpClient, manifest: PackageManifest) -> Self {
        Self {
            http,
            manifest,
            substituters: Vec::new(),
        }
    }

    /// Create a new installer with an explicit ordered substituter list.
    ///
    /// Substituters are tried in order; the first that has the store path wins.
    /// Each entry may carry a Bearer token for authenticated caches.
    pub fn with_substituters(
        http: &'a dyn justpkg_pkg::HttpClient,
        manifest: PackageManifest,
        substituters: Vec<justpkg_config::SubstituterConfig>,
    ) -> Self {
        Self {
            http,
            manifest,
            substituters,
        }
    }

    /// Fetch and extract `names` into `dest_dir`, then generate the root layout.
    ///
    /// Equivalent to calling [`install_packages`] followed by
    /// [`generate_root_layout`] with the same arguments. The root layout
    /// step creates `dest_dir/bin/<exe>` symlinks pointing into the Nix store
    /// so that the guest's vminit `access("/rootfs/bin", F_OK)` check passes.
    pub fn install(
        &self,
        names: &[&str],
        dest_dir: &std::path::Path,
    ) -> Result<(), VminitInstallError> {
        install_packages(
            self.http,
            &self.manifest,
            names,
            dest_dir,
            &self.substituters,
        )?;
        generate_root_layout(&self.manifest, names, dest_dir)
    }
}
