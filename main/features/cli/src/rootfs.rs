//! `pkg rootfs build` — TOML-driven ext4 rootfs image builder.
//!
//! Replaces per-workload `build-rootfs.sh` scripts. Reads `packages.toml`
//! (which must contain a `[rootfs]` section), installs the resolved packages
//! into a tempdir, applies users/dirs/files from the TOML config, builds an
//! ext4 image via the `ext4` library (no shell-out), and verifies it.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use edge_domain::HandlerError;
use ext4::{format as fs_format, Config, Ext4Error, Filesystem};
use justpkg_config::{AppConfig, SubstituterConfig};
use justpkg_vminit::{parse_manifest, VminitInstaller};
use justpkg_pkg::UreqClient;
use justpkg_resolve::{BinarySpec, FileSpec, UserSpec};

// ── Public entry point ────────────────────────────────────────────────────────

/// Build an ext4 rootfs image from the `[rootfs]` section of `packages_toml`.
///
/// Requires a pre-resolved `manifest.json` alongside `packages_toml`
/// (same directory). Run `pkg resolve <packages.toml> --out manifest.json` first.
///
/// `out_override` replaces `rootfs.image_out` when supplied.
pub fn build_rootfs(
    packages_toml: &Path,
    out_override: Option<PathBuf>,
    cli_substituters: Vec<SubstituterConfig>,
    config: &AppConfig,
) -> Result<PathBuf, HandlerError> {
    // ── 1. Load spec ─────────────────────────────────────────────────────────
    let spec = justpkg_resolve::load_packages_spec(packages_toml)
        .map_err(|e| HandlerError::InvalidRequest(format!("load {}: {e}", packages_toml.display())))?;

    let rootfs = spec.rootfs.as_ref().ok_or_else(|| {
        HandlerError::InvalidRequest(format!(
            "{} has no [rootfs] section — add one or use build-rootfs.sh",
            packages_toml.display()
        ))
    })?;

    // ── 2. Validate all vfs paths up-front ───────────────────────────────────
    for dir in &rootfs.dirs {
        validate_vfs_path(&dir.path).map_err(|e| {
            HandlerError::InvalidRequest(format!("[[rootfs.dir]] {}: {e}", dir.path))
        })?;
    }
    for file in &rootfs.files {
        validate_vfs_path(&file.path).map_err(|e| {
            HandlerError::InvalidRequest(format!("[[rootfs.file]] {}: {e}", file.path))
        })?;
    }
    for binary in &rootfs.binaries {
        validate_vfs_path(&binary.path).map_err(|e| {
            HandlerError::InvalidRequest(format!("[[rootfs.binary]] {}: {e}", binary.path))
        })?;
    }

    // ── 3. Resolve manifest path ──────────────────────────────────────────────
    let manifest_dir = packages_toml
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let manifest_path = manifest_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Err(HandlerError::InvalidRequest(format!(
            "manifest.json not found at {}\n  Run first: pkg resolve {} --out {}",
            manifest_path.display(),
            packages_toml.display(),
            manifest_path.display(),
        )));
    }

    // ── 4. Resolve image output path ─────────────────────────────────────────
    let image_path = out_override.unwrap_or_else(|| {
        // Resolve image_out relative to the packages.toml directory's parent
        // (the repo root for the standard packages/<name>/ layout).
        let base = packages_toml
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| Path::new("."));
        base.join(&rootfs.image_out)
    });

    if let Some(parent) = image_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                HandlerError::ExecutionFailed(format!(
                    "create image dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }

    // ── 5. Load manifest ──────────────────────────────────────────────────────
    let manifest_text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        HandlerError::InvalidRequest(format!("read {}: {e}", manifest_path.display()))
    })?;
    let pkg_manifest = parse_manifest(&manifest_text)
        .map_err(|e| HandlerError::InvalidRequest(format!("parse manifest: {e}")))?;

    // ── 6. Wire substituters ──────────────────────────────────────────────────
    let mut effective_subs = cli_substituters;
    effective_subs.extend(config.nix.effective_substituters());

    // ── 7. Install packages into tempdir ─────────────────────────────────────
    let dest = tempfile::tempdir().map_err(|e| {
        HandlerError::ExecutionFailed(format!("create tempdir: {e}"))
    })?;
    let dest_path = dest.path();

    eprintln!("==> Installing packages...");
    let all_names: Vec<String> = pkg_manifest.entries.keys().cloned().collect();
    let name_refs: Vec<&str> = all_names.iter().map(|s| s.as_str()).collect();

    let http = UreqClient;
    let installer = VminitInstaller::with_substituters(&http, pkg_manifest.clone(), effective_subs);
    installer
        .install(&name_refs, dest_path)
        .map_err(|e| HandlerError::ExecutionFailed(format!("install packages: {e}")))?;

    eprintln!("==> Nix store roots:");
    if let Ok(entries) = std::fs::read_dir(dest_path.join("nix/store")) {
        for e in entries.flatten() {
            eprintln!("    {}", e.file_name().to_string_lossy());
        }
    }

    // ── 8. /etc/passwd + /etc/group ──────────────────────────────────────────
    if !rootfs.users.is_empty() {
        let etc = dest_path.join("etc");
        std::fs::create_dir_all(&etc).map_err(|e| {
            HandlerError::ExecutionFailed(format!("mkdir /etc: {e}"))
        })?;
        write_passwd(&etc, &rootfs.users)?;
        write_group(&etc, &rootfs.users)?;
    }

    // ── 9. Data/log directories ───────────────────────────────────────────────
    for dir in &rootfs.dirs {
        let host_path = dest_path.join(dir.path.trim_start_matches('/'));
        std::fs::create_dir_all(&host_path).map_err(|e| {
            HandlerError::ExecutionFailed(format!("mkdir {}: {e}", dir.path))
        })?;
    }

    // ── 10. Entrypoint + extra files ─────────────────────────────────────────
    let store_paths = &pkg_manifest.entries;
    for file in &rootfs.files {
        write_rootfs_file(dest_path, file, store_paths)?;
    }

    // ── 10a. Pre-built binaries ───────────────────────────────────────────────
    for binary in &rootfs.binaries {
        write_rootfs_binary(dest_path, manifest_dir, binary)?;
    }

    // ── 11. Build ext4 image ─────────────────────────────────────────────────
    eprintln!("==> Building ext4 image → {}", image_path.display());
    let (size_blocks, inodes_per_group) = size_estimate(dest_path)?;
    // Pad blocks by 50% — workloads with large JDK or module files (e.g.
    // OpenSearch's openjdk/lib/modules at 300 MB+) exceed the 25% headroom
    // that size_estimate provides. 50% is conservative but avoids a rebuild.
    let size_blocks = size_blocks.saturating_mul(3) / 2;
    eprintln!("==> Image params: --size-blocks {size_blocks} --inodes {inodes_per_group}");

    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&image_path)
            .map_err(|e| {
                HandlerError::ExecutionFailed(format!(
                    "create {}: {e}",
                    image_path.display()
                ))
            })?;
        let cfg = Config { size_blocks, inodes_per_group, ..Config::default() };
        fs_format(&mut file, &cfg)
            .map_err(|e| HandlerError::ExecutionFailed(format!("format image: {e}")))?;
    }

    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&image_path)
            .map_err(|e| {
                HandlerError::ExecutionFailed(format!(
                    "re-open {}: {e}",
                    image_path.display()
                ))
            })?;
        let mut fs = Filesystem::open(file)
            .map_err(|e| HandlerError::ExecutionFailed(format!("open ext4: {e}")))?;

        populate_from_host_tree(&mut fs, dest_path)?;

        // ── 12. /bin/sh symlink ───────────────────────────────────────────────
        if let Some(shell_pkg) = &rootfs.shell {
            if let Some(shell_store) = store_paths.get(shell_pkg.as_str()) {
                let target = format!("{shell_store}/bin/bash");
                mkdir_p(&mut fs, "/bin")?;
                fs.symlink("/bin/sh", target.as_bytes()).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("symlink /bin/sh: {e}"))
                })?;
            }
        }

        // ── 13. chown dirs in the image ───────────────────────────────────────
        for dir in &rootfs.dirs {
            let uid = dir.owner.unwrap_or(0);
            let gid = dir.group.unwrap_or(uid);
            if uid != 0 || gid != 0 {
                fs.chown(&dir.path, uid, gid).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("chown {}: {e}", dir.path))
                })?;
            }
        }
    }

    eprintln!("==> Image size: {} bytes", image_path.metadata().map(|m| m.len()).unwrap_or(0));

    // ── 14. Verify ───────────────────────────────────────────────────────────
    eprintln!("==> Verifying ELF binaries...");
    verify_image(&image_path, &pkg_manifest)?;

    eprintln!();
    eprintln!("Done.");
    eprintln!("  Rootfs: {}", image_path.display());

    Ok(image_path)
}

// ── Path validation ───────────────────────────────────────────────────────────

/// Reject paths that are not absolute, contain null bytes, or traverse with `..`.
/// Prevents a malicious `packages.toml` from writing outside the image.
pub(crate) fn validate_vfs_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("must be absolute (start with /), got {path:?}"));
    }
    if path.contains('\0') {
        return Err(format!("contains null byte: {path:?}"));
    }
    for component in path.split('/') {
        if component == ".." {
            return Err(format!("must not contain '..' components: {path:?}"));
        }
    }
    Ok(())
}

// ── Store-path variable expansion ─────────────────────────────────────────────

/// Replace `{name_store}` placeholders with resolved Nix store paths.
///
/// Package names with hyphens use underscores in the key:
/// `su-exec` → `{su_exec_store}`.
pub(crate) fn expand_store_vars(content: &str, store_paths: &HashMap<String, String>) -> String {
    let mut result = content.to_string();
    for (name, store_path) in store_paths {
        let key = format!("{{{}_store}}", name.replace('-', "_"));
        result = result.replace(&key, store_path);
    }
    result
}

// ── /etc/passwd and /etc/group generation ────────────────────────────────────

fn write_passwd(etc: &Path, users: &[UserSpec]) -> Result<(), HandlerError> {
    let path = etc.join("passwd");
    if path.exists() {
        return Ok(());
    }
    let mut content = String::from("root:x:0:0:root:/root:/bin/sh\n");
    for u in users {
        content.push_str(&format!(
            "{name}:x:{uid}:{gid}:{name}:/var/lib/{name}:/bin/sh\n",
            name = u.name,
            uid  = u.uid,
            gid  = u.gid,
        ));
    }
    std::fs::write(&path, content)
        .map_err(|e| HandlerError::ExecutionFailed(format!("write /etc/passwd: {e}")))
}

fn write_group(etc: &Path, users: &[UserSpec]) -> Result<(), HandlerError> {
    let path = etc.join("group");
    if path.exists() {
        return Ok(());
    }
    let mut content = String::from("root:x:0:\n");
    for u in users {
        content.push_str(&format!("{name}:x:{gid}:\n", name = u.name, gid = u.gid));
    }
    std::fs::write(&path, content)
        .map_err(|e| HandlerError::ExecutionFailed(format!("write /etc/group: {e}")))
}

// ── File writing with store-path expansion ────────────────────────────────────

fn write_rootfs_file(
    dest_root: &Path,
    file: &FileSpec,
    store_paths: &HashMap<String, String>,
) -> Result<(), HandlerError> {
    let content = expand_store_vars(&file.content, store_paths);
    let host_path = dest_root.join(file.path.trim_start_matches('/'));
    if let Some(parent) = host_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            HandlerError::ExecutionFailed(format!(
                "mkdir parent of {}: {e}",
                file.path
            ))
        })?;
    }
    std::fs::write(&host_path, content.as_bytes())
        .map_err(|e| HandlerError::ExecutionFailed(format!("write {}: {e}", file.path)))?;

    // Apply mode on Unix hosts (on Windows the ext4 library detects ELF/#! and
    // sets the mode at populate_from_host_tree time).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&host_path, std::fs::Permissions::from_mode(file.mode))
            .map_err(|e| {
                HandlerError::ExecutionFailed(format!("chmod {}: {e}", file.path))
            })?;
    }

    Ok(())
}

// ── Pre-built binary injection ────────────────────────────────────────────────

/// Copy a pre-built binary from the host filesystem into the rootfs temp tree.
///
/// `packages_dir` is the directory that contains `packages.toml`; relative
/// `source` paths are resolved against it so callers can use bare filenames
/// (e.g. `source = "libcrypto.so.3"`) for files sitting alongside the TOML.
fn write_rootfs_binary(
    dest_root: &Path,
    packages_dir: &Path,
    binary: &BinarySpec,
) -> Result<(), HandlerError> {
    let source_path = {
        let p = Path::new(&binary.source);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            packages_dir.join(p)
        }
    };

    if !source_path.exists() {
        return Err(HandlerError::InvalidRequest(format!(
            "[[rootfs.binary]] source not found: {} (resolved from {:?})",
            binary.source,
            source_path
        )));
    }

    let bytes = std::fs::read(&source_path).map_err(|e| {
        HandlerError::ExecutionFailed(format!("read {}: {e}", source_path.display()))
    })?;

    let host_path = dest_root.join(binary.path.trim_start_matches('/'));
    if let Some(parent) = host_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            HandlerError::ExecutionFailed(format!("mkdir parent of {}: {e}", binary.path))
        })?;
    }

    std::fs::write(&host_path, &bytes).map_err(|e| {
        HandlerError::ExecutionFailed(format!("write {}: {e}", binary.path))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = binary.mode.unwrap_or_else(|| {
            if bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!") {
                0o755
            } else {
                0o644
            }
        });
        std::fs::set_permissions(&host_path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| {
                HandlerError::ExecutionFailed(format!("chmod {}: {e}", binary.path))
            })?;
    }

    Ok(())
}

// ── Image sizing ──────────────────────────────────────────────────────────────

/// Return `(size_blocks, inodes_per_group)` for a tree rooted at `host_root`.
///
/// Uses 25% data headroom plus proper per-group ext4 metadata overhead.
/// The old fixed 256-block overhead constant is far too small for multi-GB images:
/// a ~2.8 GB image has ~22 block groups each needing ~213 overhead blocks → ~4700
/// blocks of metadata that the constant was ignoring.
fn size_estimate(host_root: &Path) -> Result<(u32, u32), HandlerError> {
    let block_size: u64 = 4096;
    // ext4 default: 8 bits per byte * block_size bytes = blocks per group.
    let blocks_per_group: u64 = 8 * block_size;

    let (object_count, data_bytes) = walk_tree_stats(host_root)?;

    let inodes_needed = (object_count.saturating_mul(5) / 4).saturating_add(16);
    let inodes_per_group = inodes_needed.max(32);

    // Data blocks with 25% headroom for runtime writes.
    let data_blocks = data_bytes.div_ceil(block_size).saturating_mul(5) / 4;

    // Per-group overhead: block bitmap + inode bitmap + inode table.
    // (Superblock and GDT copies are only in select groups; 2 extra covers group 0.)
    let inode_table_per_group = (inodes_per_group as u64 * 128).div_ceil(block_size);
    let overhead_per_group = 2 + inode_table_per_group;

    // Estimate block groups needed; +1 for the partial final group.
    let num_groups = (data_blocks.div_ceil(blocks_per_group)).max(1);
    let total_overhead = overhead_per_group * num_groups + 2;

    let size_blocks = ((data_blocks + total_overhead).min(u32::MAX as u64) as u32).max(256);

    Ok((size_blocks, inodes_per_group))
}

fn walk_tree_stats(root: &Path) -> Result<(u32, u64), HandlerError> {
    let root = root.canonicalize().map_err(|e| {
        HandlerError::ExecutionFailed(format!("canonicalize {}: {e}", root.display()))
    })?;
    let mut object_count: u32 = 0;
    let mut data_bytes: u64 = 0;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            HandlerError::ExecutionFailed(format!("read_dir {}: {e}", dir.display()))
        })?;
        for entry in entries.flatten() {
            let ft = entry.file_type().map_err(|e| {
                HandlerError::ExecutionFailed(format!("file_type {:?}: {e}", entry.path()))
            })?;
            object_count = object_count.saturating_add(1);
            if ft.is_file() && !ft.is_symlink() {
                if let Ok(meta) = entry.metadata() {
                    data_bytes = data_bytes.saturating_add(meta.len());
                }
            } else if ft.is_dir() && !ft.is_symlink() {
                stack.push(entry.path());
            }
        }
    }
    Ok((object_count, data_bytes))
}

// ── Tree population ───────────────────────────────────────────────────────────

/// Walk `host_root` and replicate every file, directory, and symlink into the
/// open ext4 `fs`. Mirrors the logic in `justext4`'s `populate_from_host_tree`
/// but uses `HandlerError` instead of `CliError` for consistent error typing.
fn populate_from_host_tree<F: std::io::Read + std::io::Write + std::io::Seek>(
    fs: &mut Filesystem<F>,
    host_root: &Path,
) -> Result<(), HandlerError> {
    let host_root = host_root.canonicalize().map_err(|e| {
        HandlerError::ExecutionFailed(format!("canonicalize {}: {e}", host_root.display()))
    })?;

    let mut stack: Vec<(PathBuf, String)> = vec![(host_root, String::new())];

    while let Some((host_dir, vfs_dir)) = stack.pop() {
        let read_dir = std::fs::read_dir(&host_dir).map_err(|e| {
            HandlerError::ExecutionFailed(format!("read_dir {}: {e}", host_dir.display()))
        })?;
        for entry in read_dir {
            let entry = entry.map_err(|e| {
                HandlerError::ExecutionFailed(format!(
                    "dir entry in {}: {e}",
                    host_dir.display()
                ))
            })?;
            let name = entry.file_name();
            let name_str = match name.to_str() {
                Some(s) => s,
                None => {
                    eprintln!("warning: skipping non-UTF-8 name in {}", host_dir.display());
                    continue;
                }
            };
            let vfs_child = format!("{vfs_dir}/{name_str}");
            let file_type = entry.file_type().map_err(|e| {
                HandlerError::ExecutionFailed(format!("file_type {:?}: {e}", entry.path()))
            })?;

            if file_type.is_dir() && !file_type.is_symlink() {
                fs.mkdir(&vfs_child).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("mkdir {vfs_child}: {e}"))
                })?;
                stack.push((entry.path(), vfs_child));
            } else if file_type.is_file() {
                let bytes = std::fs::read(entry.path()).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("read {:?}: {e}", entry.path()))
                })?;
                fs.create_file(&vfs_child, &bytes).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("create_file {vfs_child}: {e}"))
                })?;
                let mode = host_file_mode(&entry.path(), &bytes);
                if mode != 0o644 {
                    fs.chmod(&vfs_child, mode).map_err(|e| {
                        HandlerError::ExecutionFailed(format!("chmod {vfs_child}: {e}"))
                    })?;
                }
            } else if file_type.is_symlink() {
                let target = std::fs::read_link(entry.path()).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("read_link {:?}: {e}", entry.path()))
                })?;
                let target_bytes = target.to_string_lossy().as_bytes().to_vec();
                fs.symlink(&vfs_child, &target_bytes).map_err(|e| {
                    HandlerError::ExecutionFailed(format!("symlink {vfs_child}: {e}"))
                })?;
            }
            // Device nodes and FIFOs are skipped; Nix store trees don't contain them.
        }
    }
    Ok(())
}

/// Permission bits for a regular file being written into the image.
/// On Unix, reads the real mode from the host filesystem.
/// On Windows, falls back to content inspection: ELF/shebang → 0o755, else 0o644.
#[cfg(unix)]
fn host_file_mode(path: &Path, _bytes: &[u8]) -> u16 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| (m.permissions().mode() & 0o7777) as u16)
        .unwrap_or(0o644)
}

#[cfg(not(unix))]
fn host_file_mode(_path: &Path, bytes: &[u8]) -> u16 {
    if bytes.starts_with(b"\x7fELF") || bytes.starts_with(b"#!") {
        0o755
    } else {
        0o644
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Create a VFS directory and all its parents, ignoring AlreadyExists.
fn mkdir_p<F: std::io::Read + std::io::Write + std::io::Seek>(
    fs: &mut Filesystem<F>,
    path: &str,
) -> Result<(), HandlerError> {
    let mut parts = Vec::new();
    let mut current = path;
    loop {
        parts.push(current);
        match current.rfind('/') {
            Some(0) | None => break,
            Some(i) => current = &current[..i],
        }
    }
    for part in parts.into_iter().rev() {
        if part.is_empty() {
            continue;
        }
        match fs.mkdir(part) {
            Ok(_) | Err(Ext4Error::AlreadyExists { .. }) => {}
            Err(e) => {
                return Err(HandlerError::ExecutionFailed(format!("mkdir {part}: {e}")))
            }
        }
    }
    Ok(())
}

// ── Inline verify ─────────────────────────────────────────────────────────────

fn verify_image(image_path: &Path, manifest: &justpkg_vminit::PackageManifest) -> Result<(), HandlerError> {
    let f = std::fs::File::open(image_path).map_err(|e| {
        HandlerError::ExecutionFailed(format!("open image {}: {e}", image_path.display()))
    })?;
    let mut fs = Filesystem::open(f).map_err(|e| {
        HandlerError::ExecutionFailed(format!("open ext4 {}: {e}", image_path.display()))
    })?;

    let mut all_ok = true;
    for (name, store_path) in &manifest.entries {
        match crate::verify_package(&mut fs, name, store_path) {
            Ok(count) => eprintln!("    ok  {name}: {count} ELF binaries"),
            Err(e) => {
                eprintln!("    FAIL {name}: {e}");
                all_ok = false;
            }
        }
    }
    if !all_ok {
        return Err(HandlerError::ExecutionFailed(
            "one or more packages failed ELF verification".into(),
        ));
    }
    eprintln!("    all {} package(s) verified", manifest.entries.len());
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TOML schema parsing ───────────────────────────────────────────────────

    #[test]
    fn test_packages_toml_with_rootfs_parses_correctly() {
        // r##"..."## needed: the TOML content contains "# sequences (TOML string
        // followed by a hash) which would prematurely terminate r#"..."#.
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
system          = "x86_64-linux"

[[package]]
attr = "opensearch"
name = "opensearch"

[[package]]
attr = "bash"
name = "bash"

[rootfs]
image_out = "dist/opensearch-rootfs.ext4"
shell     = "bash"

[[rootfs.user]]
name = "opensearch"
uid  = 1000
gid  = 1000

[[rootfs.dir]]
path  = "/var/lib/opensearch"
owner = 1000
group = 1000

[[rootfs.dir]]
path = "/var/log/opensearch"

[[rootfs.file]]
path    = "/bin/opensearch-vm"
mode    = 493
content = "#!{bash_store}/bin/bash\nexec {opensearch_store}/bin/opensearch"
"##;
        let spec: justpkg_resolve::PackagesSpec =
            toml::from_str(toml).expect("parse failed");

        assert_eq!(spec.packages.len(), 2);
        let rootfs = spec.rootfs.expect("rootfs section must be present");
        assert_eq!(rootfs.image_out, "dist/opensearch-rootfs.ext4");
        assert_eq!(rootfs.shell.as_deref(), Some("bash"));
        assert_eq!(rootfs.users.len(), 1);
        assert_eq!(rootfs.users[0].uid, 1000);
        assert_eq!(rootfs.dirs.len(), 2);
        assert_eq!(rootfs.dirs[0].owner, Some(1000));
        assert_eq!(rootfs.dirs[1].owner, None);
        assert_eq!(rootfs.files[0].mode, 493); // 0o755
    }

    #[test]
    fn test_packages_toml_without_rootfs_parses_as_none() {
        let toml = r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
"#;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        assert!(spec.rootfs.is_none(), "rootfs must be None when [rootfs] is absent");
    }

    #[test]
    fn test_rootfs_file_defaults_mode_to_0o755() {
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
[[rootfs.file]]
path    = "/bin/start"
content = "#!/bin/sh\necho hi"
"##;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        assert_eq!(spec.rootfs.unwrap().files[0].mode, 0o755);
    }

    // ── Store-path expansion ─────────────────────────────────────────────────

    #[test]
    fn test_expand_store_vars_replaces_known_package() {
        // Store paths start with '/'. Template uses #!{bash_store}/bin/bash (no
        // extra slash between #! and the placeholder) → #!/nix/store/.../bin/bash.
        let mut store_paths = HashMap::new();
        store_paths.insert("bash".to_string(), "/nix/store/aaaa-bash-5.2".to_string());
        let expanded = expand_store_vars("#!{bash_store}/bin/bash", &store_paths);
        assert_eq!(expanded, "#!/nix/store/aaaa-bash-5.2/bin/bash");
    }

    #[test]
    fn test_expand_store_vars_normalises_hyphens_to_underscores() {
        let mut store_paths = HashMap::new();
        store_paths.insert("su-exec".to_string(), "/nix/store/bbbb-su-exec-0.2".to_string());
        let expanded = expand_store_vars("{su_exec_store}/bin/su-exec", &store_paths);
        assert_eq!(expanded, "/nix/store/bbbb-su-exec-0.2/bin/su-exec");
    }

    #[test]
    fn test_expand_store_vars_unknown_placeholder_unchanged() {
        let expanded = expand_store_vars("{unknown_store}/bin/foo", &HashMap::new());
        assert_eq!(expanded, "{unknown_store}/bin/foo");
    }

    #[test]
    fn test_expand_store_vars_multiple_packages() {
        let mut m = HashMap::new();
        m.insert("bash".to_string(), "/nix/store/A-bash".to_string());
        m.insert("opensearch".to_string(), "/nix/store/B-opensearch".to_string());
        let out = expand_store_vars(
            "#!{bash_store}/bin/bash\nexec {opensearch_store}/bin/opensearch",
            &m,
        );
        assert!(out.contains("/nix/store/A-bash/bin/bash"));
        assert!(out.contains("/nix/store/B-opensearch/bin/opensearch"));
    }

    // ── Path validation (security) ────────────────────────────────────────────

    #[test]
    fn test_validate_vfs_path_accepts_absolute_paths() {
        assert!(validate_vfs_path("/bin/start").is_ok());
        assert!(validate_vfs_path("/var/lib/opensearch").is_ok());
        assert!(validate_vfs_path("/etc/passwd").is_ok());
        assert!(validate_vfs_path("/").is_ok());
    }

    #[test]
    fn test_validate_vfs_path_rejects_dotdot_traversal() {
        let result = validate_vfs_path("/var/lib/../../../etc/shadow");
        assert!(result.is_err(), "path traversal via '..' must be rejected");
        assert!(result.unwrap_err().contains(".."));
    }

    #[test]
    fn test_validate_vfs_path_rejects_dotdot_at_root() {
        assert!(validate_vfs_path("/../etc/shadow").is_err());
    }

    #[test]
    fn test_validate_vfs_path_rejects_dotdot_at_end() {
        assert!(validate_vfs_path("/var/lib/..").is_err());
    }

    #[test]
    fn test_validate_vfs_path_rejects_relative_path() {
        let result = validate_vfs_path("bin/start");
        assert!(result.is_err(), "relative paths must be rejected");
        assert!(result.unwrap_err().contains("absolute"));
    }

    #[test]
    fn test_validate_vfs_path_rejects_null_byte() {
        let result = validate_vfs_path("/bin/start\x00/../etc/shadow");
        assert!(result.is_err(), "null bytes must be rejected");
        assert!(result.unwrap_err().contains("null"));
    }

    #[test]
    fn test_validate_vfs_path_accepts_names_containing_dots_but_not_dotdot() {
        // "..hidden" is a valid filename; only the bare ".." component is rejected.
        assert!(validate_vfs_path("/var/lib/..hidden").is_ok());
        assert!(validate_vfs_path("/etc/file..cfg").is_ok());
    }

    // ── [[rootfs.binary]] parsing ─────────────────────────────────────────────

    #[test]
    fn test_rootfs_binary_parses_with_explicit_mode() {
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
[[rootfs.binary]]
path   = "/usr/local/bin/fleetd"
source = "fleetd"
mode   = 493
"##;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        let rootfs = spec.rootfs.unwrap();
        assert_eq!(rootfs.binaries.len(), 1);
        let b = &rootfs.binaries[0];
        assert_eq!(b.path, "/usr/local/bin/fleetd");
        assert_eq!(b.source, "fleetd");
        assert_eq!(b.mode, Some(0o755));
    }

    #[test]
    fn test_rootfs_binary_mode_defaults_to_none() {
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
[[rootfs.binary]]
path   = "/lib/x86_64-linux-gnu/libcrypto.so.3"
source = "libcrypto.so.3"
"##;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        let b = &spec.rootfs.unwrap().binaries[0];
        assert!(b.mode.is_none(), "mode must be None when absent from TOML");
    }

    #[test]
    fn test_rootfs_binary_absent_when_no_binary_section() {
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
"##;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        assert!(spec.rootfs.unwrap().binaries.is_empty());
    }

    #[test]
    fn test_rootfs_binary_multiple_entries() {
        let toml = r##"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "bash"
attr = "bash"
[rootfs]
image_out = "out.ext4"
[[rootfs.binary]]
path   = "/usr/local/bin/fleetd"
source = "fleetd"
[[rootfs.binary]]
path   = "/lib/x86_64-linux-gnu/libssl.so.3"
source = "libssl.so.3"
[[rootfs.binary]]
path   = "/lib/x86_64-linux-gnu/libcrypto.so.3"
source = "libcrypto.so.3"
"##;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        let binaries = &spec.rootfs.unwrap().binaries;
        assert_eq!(binaries.len(), 3);
        assert_eq!(binaries[0].path, "/usr/local/bin/fleetd");
        assert_eq!(binaries[1].path, "/lib/x86_64-linux-gnu/libssl.so.3");
        assert_eq!(binaries[2].path, "/lib/x86_64-linux-gnu/libcrypto.so.3");
    }

    #[test]
    fn test_write_rootfs_binary_copies_file_to_dest_tree() {
        use std::io::Write;
        let src_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();

        // Write a fake ELF-magic binary to the source dir.
        let src_file = src_dir.path().join("mybin");
        let mut f = std::fs::File::create(&src_file).unwrap();
        f.write_all(b"\x7fELFfakebinarycontents").unwrap();

        let binary = BinarySpec {
            path: "/usr/local/bin/mybin".to_string(),
            source: "mybin".to_string(),
            mode: Some(0o755),
        };

        write_rootfs_binary(dest_dir.path(), src_dir.path(), &binary)
            .expect("write_rootfs_binary must succeed");

        let written = std::fs::read(dest_dir.path().join("usr/local/bin/mybin")).unwrap();
        assert_eq!(written, b"\x7fELFfakebinarycontents");
    }

    #[test]
    fn test_write_rootfs_binary_errors_when_source_missing() {
        let src_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();

        let binary = BinarySpec {
            path: "/usr/local/bin/ghost".to_string(),
            source: "ghost".to_string(),
            mode: None,
        };

        let result = write_rootfs_binary(dest_dir.path(), src_dir.path(), &binary);
        assert!(result.is_err(), "must fail when source file does not exist");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(msg.contains("source not found") || msg.contains("ghost"));
    }
}
