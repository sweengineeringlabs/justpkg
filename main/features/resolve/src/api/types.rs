use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Input: packages.toml ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PackagesSpec {
    /// NixOS channel name, e.g. `"nixos-24.11"`.
    pub nixpkgs_channel: String,

    /// Target system tuple (informational — store-paths.xz is per-channel, not per-system).
    #[serde(default = "default_system")]
    pub system: String,

    /// List of packages to resolve. TOML key is `[[package]]`.
    #[serde(rename = "package")]
    pub packages: Vec<PackageEntry>,

    /// Optional rootfs build configuration. TOML key is `[rootfs]`.
    /// When present, `pkg rootfs build` uses this to drive the full
    /// install → image → verify pipeline without a shell script.
    #[serde(default)]
    pub rootfs: Option<RootfsSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageEntry {
    /// Short name used as the manifest key and for store-path lookup.
    /// Used to search store-paths.xz: `*-{name.replace('_','-')}-*`.
    pub name: String,

    /// Nix attribute path (informational; not used in current resolver).
    #[serde(default)]
    pub attr: String,
}

fn default_system() -> String {
    "x86_64-linux".to_string()
}

// ── Rootfs build spec (optional [rootfs] section in packages.toml) ───────────

/// Describes the ext4 rootfs image to build from the resolved packages.
///
/// Consumed by `pkg rootfs build` — a TOML-driven replacement for `build-rootfs.sh`.
///
/// Example:
/// ```toml
/// [rootfs]
/// image_out = "dist/opensearch-rootfs.ext4"
/// shell     = "bash"
///
/// [[rootfs.user]]
/// name = "opensearch"
/// uid  = 1000
/// gid  = 1000
///
/// [[rootfs.dir]]
/// path  = "/var/lib/opensearch"
/// owner = 1000
/// group = 1000
///
/// [[rootfs.file]]
/// path    = "/bin/opensearch-vm"
/// mode    = 0o755
/// content = "#!/{bash_store}/bin/bash\nexec ..."
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct RootfsSpec {
    /// Output path for the ext4 image, e.g. `"dist/opensearch-rootfs.ext4"`.
    pub image_out: String,

    /// Package name whose `bin/bash` becomes `/bin/sh` in the image.
    /// Most workloads should set this to `"bash"`.
    #[serde(default)]
    pub shell: Option<String>,

    /// Non-root users to add to `/etc/passwd` and `/etc/group`.
    /// TOML key: `[[rootfs.user]]`.
    #[serde(default, rename = "user")]
    pub users: Vec<UserSpec>,

    /// Directories to create in the image (and optionally chown).
    /// TOML key: `[[rootfs.dir]]`.
    #[serde(default, rename = "dir")]
    pub dirs: Vec<DirSpec>,

    /// Files to write into the image with store-path variable expansion.
    /// TOML key: `[[rootfs.file]]`.
    #[serde(default, rename = "file")]
    pub files: Vec<FileSpec>,
}

/// A non-root user entry for `/etc/passwd` and `/etc/group`.
#[derive(Debug, Clone, Deserialize)]
pub struct UserSpec {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
}

/// A directory to create in the image, with optional ownership.
#[derive(Debug, Clone, Deserialize)]
pub struct DirSpec {
    /// Absolute VFS path, e.g. `"/var/lib/opensearch"`.
    pub path: String,
    /// UID to assign via chown. Defaults to 0 (root).
    #[serde(default)]
    pub owner: Option<u32>,
    /// GID to assign via chown. Defaults to 0 (root).
    #[serde(default)]
    pub group: Option<u32>,
}

/// A file to write into the image.
///
/// The `content` field supports `{name_store}` placeholders that expand to the
/// resolved Nix store path for the package named `name`. Hyphens in package
/// names are replaced with underscores in the placeholder key:
/// `su-exec` → `{su_exec_store}`.
#[derive(Debug, Clone, Deserialize)]
pub struct FileSpec {
    /// Absolute VFS path, e.g. `"/bin/opensearch-vm"`.
    pub path: String,
    /// POSIX permission bits. Defaults to `0o755`.
    #[serde(default = "default_file_mode")]
    pub mode: u32,
    /// File content with optional `{name_store}` placeholder expansion.
    pub content: String,
}

fn default_file_mode() -> u32 {
    0o755
}

// ── Output: manifest.json ────────────────────────────────────────────────────

/// Serialises to the format consumed by `vminit::parse_manifest`:
/// `{"packages": {"name": "/nix/store/<hash>-<name>-<version>"}, "meta": {…}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedManifest {
    /// name → Nix store path; consumed by `vminit::install_packages`.
    pub packages: BTreeMap<String, String>,
    pub meta: ManifestMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestMeta {
    /// Nixpkgs git revision from the channel's git-revision file.
    pub nixpkgs_rev: String,
    /// Channel that was resolved against, e.g. `"nixos-24.11"`.
    pub channel: String,
    /// RFC 3339 UTC timestamp of when `justpkg resolve` ran.
    pub resolved_at: String,
}
