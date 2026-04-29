pub use crate::api::error::VminitInstallError;
pub use crate::api::manifest::PackageManifest;
pub use crate::spi::installer::install_packages;
pub use crate::spi::manifest::parse_manifest;

/// High-level installer: bundles an `HttpClient` reference with a parsed manifest
/// so callers can invoke `install` without threading both through every call site.
pub struct VminitInstaller<'a> {
    http: &'a dyn justpkg_pkg::HttpClient,
    manifest: PackageManifest,
}

impl<'a> VminitInstaller<'a> {
    /// Create a new installer bound to `http` and `manifest`.
    pub fn new(http: &'a dyn justpkg_pkg::HttpClient, manifest: PackageManifest) -> Self {
        Self { http, manifest }
    }

    /// Fetch and extract `names` into `dest_dir`.
    pub fn install(
        &self,
        names: &[&str],
        dest_dir: &std::path::Path,
    ) -> Result<(), VminitInstallError> {
        install_packages(self.http, &self.manifest, names, dest_dir)
    }
}
