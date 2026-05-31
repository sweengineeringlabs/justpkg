//! Workload package scaffolding — generates packages/<name>/{packages.toml,build-rootfs.sh,vm.toml}.

use std::path::Path;

use edge_domain::HandlerError;

/// Configuration for scaffolding a new workload package directory.
pub struct ScaffoldConfig {
    /// Workload name, e.g. `"opensearch"`. Becomes the directory and file prefix.
    pub name: String,
    /// UID (and GID) for the non-root user the workload runs as.
    /// When set, `build-rootfs.sh` gains `/etc/passwd`, `/etc/group`,
    /// data/log directory creation, and `justext4 chown` steps.
    pub non_root: Option<u32>,
    /// Port mappings as `(host_port, guest_port)` pairs.
    pub ports: Vec<(u16, u16)>,
    /// Guest memory in MiB.
    pub memory_mb: u32,
    /// Number of vCPUs.
    pub vcpus: u32,
}

/// Create `packages/<name>/{packages.toml,build-rootfs.sh,vm.toml}` under `base_dir`.
///
/// Returns `Err(HandlerError::Conflict)` if the directory already exists so
/// callers never silently overwrite an existing workload.
pub fn scaffold(cfg: &ScaffoldConfig, base_dir: &Path) -> Result<(), HandlerError> {
    let out = base_dir.join("packages").join(&cfg.name);
    if out.exists() {
        return Err(HandlerError::Conflict(format!(
            "packages/{} already exists — delete it first or choose a different name",
            cfg.name
        )));
    }
    std::fs::create_dir_all(&out)
        .map_err(|e| HandlerError::ExecutionFailed(format!("create {}: {e}", out.display())))?;

    write_file(&out.join("packages.toml"), &generate_packages_toml(cfg))?;
    write_file(&out.join("build-rootfs.sh"), &generate_build_rootfs_sh(cfg))?;
    write_file(&out.join("vm.toml"), &generate_vm_toml(cfg))?;

    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), HandlerError> {
    std::fs::write(path, content)
        .map_err(|e| HandlerError::ExecutionFailed(format!("write {}: {e}", path.display())))
}

/// Derive a deterministic 6-byte MAC address from the workload name.
///
/// The first four bytes are fixed (`DE:AD:BE:EF`) to make it visually obvious
/// these are dev/test VMs. The last two bytes are a simple checksum of the name
/// so each workload gets a unique address without requiring randomness.
fn derive_mac(name: &str) -> String {
    let b1 = name.bytes().fold(0u8, |acc, b| acc.wrapping_add(b));
    let b2 = name
        .bytes()
        .enumerate()
        .fold(0u8, |acc, (i, b)| acc.wrapping_add(b.wrapping_mul((i as u8).wrapping_add(1))));
    format!("DE:AD:BE:EF:{b1:02X}:{b2:02X}")
}

// ── Template helpers ──────────────────────────────────────────────────────────
//
// Bash scripts use `${}`, `${arr[@]}`, `$()`, and `{}` freely.  Rather than
// escaping these in Rust format strings (where `{` must be written `{{`), we
// build the bash content from raw string literals and apply simple
// `__NAME__` → actual-name substitution afterwards.

fn subst(template: &str, name: &str) -> String {
    template
        .replace("__NAME__", name)
        .replace("__NAME_UPPER__", &name.to_uppercase())
}

// ── packages.toml ─────────────────────────────────────────────────────────────

pub fn generate_packages_toml(cfg: &ScaffoldConfig) -> String {
    let mut s = String::new();
    s.push_str("nixpkgs_channel = \"nixos-24.11\"\n");
    s.push_str("system          = \"x86_64-linux\"\n");
    s.push('\n');
    s.push_str("# TODO: add your workload-specific Nix packages here\n");
    s.push('\n');
    s.push_str("[[package]]\n");
    s.push_str("attr = \"bash\"\n");
    s.push_str("name = \"bash\"\n");
    s.push('\n');
    s.push_str("[[package]]\n");
    s.push_str("attr = \"tzdata\"\n");
    s.push_str("name = \"tzdata\"\n");
    if let Some(uid) = cfg.non_root {
        s.push('\n');
        s.push_str(&format!(
            "# su-exec drops privileges to uid {uid} at boot.\n"
        ));
        s.push_str("[[package]]\n");
        s.push_str("attr = \"su-exec\"\n");
        s.push_str("name = \"su-exec\"\n");
    }
    s
}

// ── build-rootfs.sh ───────────────────────────────────────────────────────────

pub fn generate_build_rootfs_sh(cfg: &ScaffoldConfig) -> String {
    let name = &cfg.name;
    let mut s = String::new();

    // Header
    s.push_str(&subst(
        r#"#!/usr/bin/env bash
# Rebuild dist/__NAME__-rootfs.ext4 from the pinned manifest.
#
# Runs natively on Windows (Git Bash) and on Linux/WSL.
# Requires:
#   - pkg (justpkg CLI) on PATH
#   - justext4 on PATH
#   - Network access to cache.nixos.org
#
# Install the tools via bootstrap.sh or the project release artifacts.
#
# Usage:
#   bash packages/__NAME__/build-rootfs.sh
#
# Output:
#   dist/__NAME__-rootfs.ext4

set -euo pipefail

die() { echo "error: $*" >&2; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MANIFEST="$REPO_ROOT/packages/__NAME__/manifest.json"
IMAGE="$REPO_ROOT/dist/__NAME__-rootfs.ext4"

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
    *)                     EXE=""     ;;
esac

JUSTPKG_BIN="$(command -v "pkg${EXE}" 2>/dev/null)" \
    || die "pkg not found in PATH — install via bootstrap.sh or release artifacts"

JUSTEXT4_BIN="$(command -v "justext4${EXE}" 2>/dev/null)" \
    || die "justext4 not found in PATH — install via bootstrap.sh or release artifacts"

command -v jq >/dev/null 2>&1 \
    || die "jq not found in PATH — install jq (apt-get install jq / brew install jq / choco install jq)"

"#,
        name,
    ));

    // Store path extraction
    s.push_str("# ── Store paths from manifest.json ───────────────────────────────────────────\n");
    s.push_str("BASH_STORE=\"$(jq -r '.packages.bash' \"$MANIFEST\")\"\n");
    s.push_str(&format!(
        "# TODO: add store path extractions for your workload packages, e.g.:\n# {}_STORE=\"$(jq -r '.packages.{}' \"$MANIFEST\")\"\n",
        name.to_uppercase().replace('-', "_"),
        name
    ));
    if cfg.non_root.is_some() {
        s.push_str("SU_EXEC_STORE=\"$(jq -r '.packages[\"su-exec\"]' \"$MANIFEST\")\"\n");
    }
    s.push('\n');

    // Manifest validation
    s.push_str(&subst(
        r#"[[ "$BASH_STORE" != "null" && -n "$BASH_STORE" ]] \
    || die "manifest.json missing .packages.bash — run: pkg resolve packages/__NAME__/packages.toml --out packages/__NAME__/manifest.json"
"#,
        name,
    ));
    if cfg.non_root.is_some() {
        s.push_str(r#"[[ "$SU_EXEC_STORE" != "null" && -n "$SU_EXEC_STORE" ]] \
    || die "manifest.json missing .packages.su-exec"
"#);
    }

    // Substituters are configured in ~/.config/justpkg/application.toml — no per-script block needed.
    s.push_str(r#"
# ── Install packages ──────────────────────────────────────────────────────────

DEST=$(mktemp -d)
trap 'rm -rf "$DEST"' EXIT

echo "==> Installing packages to $DEST..."
"$JUSTPKG_BIN" install "$MANIFEST" "$DEST"

echo "==> Nix store roots:"
ls "$DEST/nix/store/"

mkdir -p "$DEST/bin"
"#);

    // Non-root: users, groups, directories
    if let Some(uid) = cfg.non_root {
        s.push_str(&subst(
            &format!(
                r#"
# ── Users and groups ──────────────────────────────────────────────────────────
# __NAME_UPPER__ refuses to start as root. uid/gid {uid} = __NAME__.

mkdir -p "$DEST/etc"
if [[ ! -e "$DEST/etc/passwd" ]]; then
    printf 'root:x:0:0:root:/root:/bin/sh\n__NAME__:x:{uid}:{uid}:__NAME_UPPER__:/var/lib/__NAME__:/bin/sh\n' \
        > "$DEST/etc/passwd"
fi
if [[ ! -e "$DEST/etc/group" ]]; then
    printf 'root:x:0:\n__NAME__:x:{uid}:\n' > "$DEST/etc/group"
fi

# ── Data directories ──────────────────────────────────────────────────────────

mkdir -p "$DEST/var/lib/__NAME__" "$DEST/var/log/__NAME__"
"#
            ),
            name,
        ));
    }

    // Entrypoint wrapper
    if let Some(uid) = cfg.non_root {
        s.push_str(&subst(
            &format!(
                r#"
# ── Entrypoint wrapper ────────────────────────────────────────────────────────
# TODO: replace the body with your workload binary and flags.
# su-exec drops privileges to uid {uid} before exec.

cat > "$DEST/bin/__NAME__-vm" <<ENTRYPOINT
#!${{BASH_STORE}}/bin/bash
export TODO_SET_REQUIRED_ENV_VARS=1
exec ${{SU_EXEC_STORE}}/bin/su-exec __NAME__:__NAME__ \\\\
  # TODO: replace with your workload binary
  echo "TODO: implement __NAME__ entrypoint" >&2; exit 1
ENTRYPOINT
chmod +x "$DEST/bin/__NAME__-vm"
"#
            ),
            name,
        ));
    } else {
        s.push_str(&subst(
            r#"
# ── Entrypoint wrapper ────────────────────────────────────────────────────────
# TODO: replace the body with your workload binary and flags.

cat > "$DEST/bin/__NAME__-vm" <<'ENTRYPOINT'
#!/bin/sh
# TODO: replace with your workload binary
echo "TODO: implement __NAME__ entrypoint" >&2
exit 1
ENTRYPOINT
chmod +x "$DEST/bin/__NAME__-vm"
"#,
            name,
        ));
    }

    // Build ext4 image
    s.push_str(&subst(
        r#"
# ── Build ext4 image ──────────────────────────────────────────────────────────

mkdir -p "$(dirname "$IMAGE")"
echo "==> Building ext4 image -> $IMAGE..."
SIZE_ARGS=$("$JUSTEXT4_BIN" size-estimate "$DEST")
echo "==> Image params: $SIZE_ARGS"
# shellcheck disable=SC2086
"$JUSTEXT4_BIN" build-from-tree "$DEST" "$IMAGE" $SIZE_ARGS

echo "==> Image size: $(du -sh "$IMAGE" | cut -f1)"

# ── /bin/sh symlink ───────────────────────────────────────────────────────────
# justext4 symlink stores targets > 60 bytes in a data block (slow-symlink path).
"$JUSTEXT4_BIN" symlink "$IMAGE" /bin/sh "$BASH_STORE/bin/bash"
"#,
        name,
    ));

    // Chown step — only for non-root workloads
    if let Some(uid) = cfg.non_root {
        s.push_str(&subst(
            &format!(
                r#"
# ── Fix ownership ─────────────────────────────────────────────────────────────
# justext4 build-from-tree assigns uid/gid 0 on Windows (NTFS has no Unix
# ownership). Chown data/log dirs so uid {uid} can write there at runtime.

echo "==> Setting ownership of data/log dirs to __NAME__ ({uid}:{uid})..."
"$JUSTEXT4_BIN" chown "$IMAGE" /var/lib/__NAME__ {uid} {uid}
"$JUSTEXT4_BIN" chown "$IMAGE" /var/log/__NAME__  {uid} {uid}
"#
            ),
            name,
        ));
    }

    // Validate
    s.push_str(&subst(
        r#"
# ── Validate ──────────────────────────────────────────────────────────────────

echo "==> Running justpkg verify-image..."
"$JUSTPKG_BIN" verify-image "$MANIFEST" "$IMAGE"

echo ""
echo "Done."
echo "  Rootfs: $IMAGE"
echo ""
echo "Run: vmic run --config packages/__NAME__/vm.toml"
"#,
        name,
    ));

    s
}

// ── vm.toml ───────────────────────────────────────────────────────────────────

pub fn generate_vm_toml(cfg: &ScaffoldConfig) -> String {
    let name = &cfg.name;
    let name_upper = name.to_uppercase().replace('-', "_");
    let memory_bytes: u64 = cfg.memory_mb as u64 * 1024 * 1024;
    let mac = derive_mac(name);

    let port_mappings = if cfg.ports.is_empty() {
        "  # TODO: add port mappings, e.g.:\n  # { host_port = 8080, container_port = 8080, protocol = \"tcp\" }\n".to_string()
    } else {
        cfg.ports
            .iter()
            .enumerate()
            .map(|(i, (h, g))| {
                let comma = if i < cfg.ports.len() - 1 { "," } else { "" };
                format!("  {{ host_port = {h}, container_port = {g}, protocol = \"tcp\" }}{comma}\n")
            })
            .collect()
    };

    format!(
        "# {name_upper} workload VM — run with: vmic run --config packages/{name}/vm.toml\n\
         #\n\
         # Rebuild the rootfs after a package update:\n\
         #   bash packages/{name}/build-rootfs.sh\n\
         #\n\
         # First-time setup:\n\
         #   pkg resolve packages/{name}/packages.toml --out packages/{name}/manifest.json\n\
         #   bash packages/{name}/build-rootfs.sh\n\
         \n\
         vcpu_count  = {vcpus}\n\
         memory_size = {memory_bytes}   # {memory_mb} MiB\n\
         boot_timeout_secs = 0          # run until stopped; no boot deadline\n\
         \n\
         kernel_path    = \"downloads/bzImage_7.0.4\"\n\
         kernel_cmdline = \"console=ttyS0 earlyprintk=serial,ttyS0,115200 nokaslr tsc=reliable no_timer_check lpj=1000000 quiet rdinit=/init\"\n\
         \n\
         entrypoint = [\"/bin/{name}-vm\"]\n\
         \n\
         [rootfs_device]\n\
         path      = \"dist/{name}-rootfs.ext4\"\n\
         read_only = false\n\
         \n\
         [[net_devices]]\n\
         mac           = \"{mac}\"\n\
         net_mode      = \"nat\"\n\
         port_mappings = [\n\
         {port_mappings}]\n",
        vcpus = cfg.vcpus,
        memory_mb = cfg.memory_mb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn simple_cfg(name: &str) -> ScaffoldConfig {
        ScaffoldConfig {
            name: name.to_string(),
            non_root: None,
            ports: vec![],
            memory_mb: 512,
            vcpus: 1,
        }
    }

    fn nonroot_cfg(name: &str, uid: u32) -> ScaffoldConfig {
        ScaffoldConfig {
            name: name.to_string(),
            non_root: Some(uid),
            ports: vec![(9200, 9200), (9300, 9300)],
            memory_mb: 1024,
            vcpus: 2,
        }
    }

    #[test]
    fn test_scaffold_simple_generates_three_files() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        assert!(dir.path().join("packages/myapp/packages.toml").exists());
        assert!(dir.path().join("packages/myapp/build-rootfs.sh").exists());
        assert!(dir.path().join("packages/myapp/vm.toml").exists());
    }

    #[test]
    fn test_scaffold_generates_bin_sh_symlink_via_justext4() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let sh = fs::read_to_string(dir.path().join("packages/myapp/build-rootfs.sh")).unwrap();
        assert!(
            sh.contains("symlink \"$IMAGE\" /bin/sh \"$BASH_STORE/bin/bash\""),
            "build-rootfs.sh must create /bin/sh via justext4 symlink (not a wrapper script)"
        );
        assert!(
            !sh.contains("printf '#!/"),
            "build-rootfs.sh must not contain a wrapper script — use justext4 symlink instead"
        );
    }

    #[test]
    fn test_scaffold_without_non_root_omits_chown() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let sh = fs::read_to_string(dir.path().join("packages/myapp/build-rootfs.sh")).unwrap();
        assert!(
            !sh.contains("justext4 chown"),
            "non-root not requested — build-rootfs.sh must not contain justext4 chown"
        );
    }

    #[test]
    fn test_scaffold_with_non_root_includes_chown() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = nonroot_cfg("opensearch", 1000);
        scaffold(&cfg, dir.path()).unwrap();
        let sh =
            fs::read_to_string(dir.path().join("packages/opensearch/build-rootfs.sh")).unwrap();
        assert!(
            sh.contains("chown \"$IMAGE\" /var/lib/opensearch 1000 1000"),
            "non-root requested — build-rootfs.sh must contain chown for data dir with uid 1000"
        );
        assert!(
            sh.contains("chown \"$IMAGE\" /var/log/opensearch"),
            "non-root requested — build-rootfs.sh must contain chown for log dir"
        );
    }

    #[test]
    fn test_scaffold_with_non_root_includes_passwd() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = nonroot_cfg("opensearch", 1000);
        scaffold(&cfg, dir.path()).unwrap();
        let sh =
            fs::read_to_string(dir.path().join("packages/opensearch/build-rootfs.sh")).unwrap();
        assert!(
            sh.contains("/etc/passwd"),
            "non-root requested — build-rootfs.sh must create /etc/passwd"
        );
        assert!(
            sh.contains("opensearch:x:1000:1000"),
            "passwd entry must use the correct uid 1000"
        );
    }

    #[test]
    fn test_scaffold_port_appears_in_vm_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ScaffoldConfig {
            name: "myapp".to_string(),
            non_root: None,
            ports: vec![(8080, 8080)],
            memory_mb: 512,
            vcpus: 1,
        };
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/vm.toml")).unwrap();
        assert!(
            toml.contains("host_port = 8080"),
            "declared port must appear in vm.toml port_mappings"
        );
        assert!(
            toml.contains("container_port = 8080"),
            "declared guest port must appear in vm.toml port_mappings"
        );
    }

    #[test]
    fn test_scaffold_memory_appears_in_vm_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/vm.toml")).unwrap();
        // 512 MiB = 536870912 bytes
        assert!(
            toml.contains("memory_size = 536870912"),
            "memory_size must be memory_mb * 1024 * 1024"
        );
    }

    #[test]
    fn test_scaffold_fails_if_directory_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("packages/myapp")).unwrap();
        let cfg = simple_cfg("myapp");
        let result = scaffold(&cfg, dir.path());
        assert!(
            matches!(result, Err(HandlerError::Conflict(_))),
            "must return Conflict when packages/<name> already exists"
        );
    }

    #[test]
    fn test_scaffold_non_root_packages_toml_includes_su_exec() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = nonroot_cfg("myapp", 1000);
        scaffold(&cfg, dir.path()).unwrap();
        let toml =
            fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            toml.contains("su-exec"),
            "non-root workload must declare su-exec in packages.toml"
        );
    }

    #[test]
    fn test_scaffold_simple_packages_toml_omits_su_exec() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml =
            fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            !toml.contains("su-exec"),
            "simple workload must not declare su-exec in packages.toml"
        );
    }

    #[test]
    fn test_derive_mac_is_deterministic() {
        assert_eq!(derive_mac("redis"), derive_mac("redis"));
        assert_ne!(
            derive_mac("redis"),
            derive_mac("opensearch"),
            "different names must produce different MACs"
        );
    }
}
