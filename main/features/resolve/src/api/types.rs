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

    /// Optional build profile. TOML key is `[profile]`.
    /// When present, `pkg flake build` applies these feature flags to every
    /// `[[package]]` that supports them, overriding nixpkgs defaults.
    /// Packages that don't have a given flag ignore it silently.
    ///
    /// ```toml
    /// [profile]
    /// jit     = false   # drop LLVM/JIT (postgres)
    /// icu     = false   # drop ICU collation (postgres)
    /// tls     = false   # drop TLS (redis — use NAT instead)
    /// systemd = false   # drop systemd integration (no init system in VM)
    /// ```
    #[serde(default)]
    pub profile: Option<BuildProfile>,
}

/// Workload-level build profile: feature flags applied to nixpkgs overrides.
///
/// Each flag is `Option<bool>` — absent means "use nixpkgs default",
/// `false` explicitly disables the feature. `true` is accepted for
/// documentation purposes but is always the nixpkgs default.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BuildProfile {
    /// JIT query compilation via LLVM. Only beneficial for complex analytical
    /// queries (cost > jit_above_cost=100000). Safe to disable for OLTP.
    #[serde(default)]
    pub jit: Option<bool>,
    /// ICU Unicode collation provider. Required only for non-C locale collation.
    /// Safe to disable when using `--locale=C`.
    #[serde(default)]
    pub icu: Option<bool>,
    /// TLS support. Disable when all clients connect via NAT/host-boundary TLS.
    #[serde(default)]
    pub tls: Option<bool>,
    /// systemd socket activation and journal integration.
    /// Disable when running without an init system (microVM workloads).
    #[serde(default)]
    pub systemd: Option<bool>,
    /// PL/Perl procedural language. Disable when no Perl stored procedures are used.
    #[serde(default)]
    pub perl: Option<bool>,
    /// PL/Python procedural language. Disable when no Python stored procedures are used.
    #[serde(default)]
    pub python: Option<bool>,
    /// PL/Tcl procedural language. Disable when no Tcl stored procedures are used.
    #[serde(default)]
    pub tcl: Option<bool>,
    /// PAM pluggable authentication. Disable when using password/env-based auth only.
    #[serde(default)]
    pub pam: Option<bool>,
    /// Kerberos/GSSAPI authentication. Disable outside Active Directory environments.
    #[serde(default)]
    pub gss: Option<bool>,

    /// Run the nixpkgs test suite during the build (`doCheck`). Default: true.
    /// Set `check = false` when check-world fails in the Nix sandbox — the
    /// sandbox restricts Unix sockets and TCP ports needed by auth/TAP suites.
    /// The standard regression tests pass; only the extended check-world is
    /// sandbox-incompatible. Applied via `overrideAttrs { doCheck = false; }`.
    #[serde(default)]
    pub check: Option<bool>,
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

    /// Pre-built binaries to inject into the image from the host filesystem.
    /// TOML key: `[[rootfs.binary]]`.
    #[serde(default, rename = "binary")]
    pub binaries: Vec<BinarySpec>,
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

/// A pre-built binary to inject into the image from the host filesystem.
///
/// Unlike `[[rootfs.file]]`, the source is an existing file on the host —
/// no content generation or placeholder expansion. Use for ELF executables
/// and shared libraries that are built outside Nix (e.g. `fleetd` and its
/// runtime `.so` files).
///
/// ```toml
/// [[rootfs.binary]]
/// path   = "/usr/local/bin/fleetd"
/// source = "fleetd"                  # relative to packages.toml directory
/// mode   = 0o755                     # optional; auto-detected from ELF/shebang if absent
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct BinarySpec {
    /// Absolute VFS path in the image, e.g. `"/usr/local/bin/fleetd"`.
    pub path: String,
    /// Host filesystem path. Resolved relative to the `packages.toml` directory
    /// when not absolute.
    pub source: String,
    /// POSIX permission bits. Auto-detected from ELF magic / shebang when absent.
    #[serde(default)]
    pub mode: Option<u32>,
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
