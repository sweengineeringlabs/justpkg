//! Workload package scaffolding — generates packages/<name>/{packages.toml,vm.toml}.

use std::path::Path;

use edge_domain::HandlerError;

/// Configuration for scaffolding a new workload package directory.
pub struct ScaffoldConfig {
    /// Workload name, e.g. `"opensearch"`. Becomes the directory and file prefix.
    pub name: String,
    /// UID (and GID) for the non-root user the workload runs as.
    /// When set, `packages.toml` gains a [[rootfs.user]] entry, data/log
    /// [[rootfs.dir]] entries with ownership, and an su-exec [[package]].
    pub non_root: Option<u32>,
    /// Port mappings as `(host_port, guest_port)` pairs.
    pub ports: Vec<(u16, u16)>,
    /// Guest memory in MiB.
    pub memory_mb: u32,
    /// Number of vCPUs.
    pub vcpus: u32,
}

/// Create `packages/<name>/{packages.toml,vm.toml}` under `base_dir`.
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

// ── packages.toml ─────────────────────────────────────────────────────────────

pub fn generate_packages_toml(cfg: &ScaffoldConfig) -> String {
    let name = &cfg.name;
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
        s.push_str(&format!("# su-exec drops privileges to uid {uid} at boot.\n"));
        s.push_str("[[package]]\n");
        s.push_str("attr = \"su-exec\"\n");
        s.push_str("name = \"su-exec\"\n");
    }

    // ── [rootfs] section ─────────────────────────────────────────────────────
    s.push_str(&format!("\n[rootfs]\nimage_out = \"dist/{name}-rootfs.ext4\"\n"));
    s.push_str("shell     = \"bash\"\n");

    if let Some(uid) = cfg.non_root {
        s.push_str(&format!(
            "\n[[rootfs.user]]\nname = \"{name}\"\nuid  = {uid}\ngid  = {uid}\n"
        ));
        s.push_str(&format!(
            "\n[[rootfs.dir]]\npath  = \"/var/lib/{name}\"\nowner = {uid}\ngroup = {uid}\n"
        ));
        s.push_str(&format!(
            "\n[[rootfs.dir]]\npath  = \"/var/log/{name}\"\nowner = {uid}\ngroup = {uid}\n"
        ));
    }

    // Entrypoint file — placeholder; user fills in the workload binary.
    if cfg.non_root.is_some() {
        s.push_str(&format!(
            r#"
[[rootfs.file]]
path    = "/bin/{name}-vm"
mode    = 493
content = """
#!{{bash_store}}/bin/bash
# TODO: set required environment variables
exec {{su_exec_store}}/bin/su-exec {name}:{name} \
  # TODO: replace with your workload binary
  echo "TODO: implement {name} entrypoint" >&2
"""
"#
        ));
    } else {
        s.push_str(&format!(
            r#"
[[rootfs.file]]
path    = "/bin/{name}-vm"
mode    = 493
content = """
#!{{bash_store}}/bin/bash
# TODO: replace with your workload binary
echo "TODO: implement {name} entrypoint" >&2
exit 1
"""
"#
        ));
    }

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
         #   pkg rootfs build packages/{name}/packages.toml\n\
         #\n\
         # First-time setup:\n\
         #   pkg resolve packages/{name}/packages.toml --out packages/{name}/manifest.json\n\
         #   pkg rootfs build packages/{name}/packages.toml\n\
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
    fn test_scaffold_simple_generates_two_files_no_shell_script() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        assert!(dir.path().join("packages/myapp/packages.toml").exists());
        assert!(dir.path().join("packages/myapp/vm.toml").exists());
        assert!(
            !dir.path().join("packages/myapp/build-rootfs.sh").exists(),
            "build-rootfs.sh must not be generated — use pkg rootfs build instead"
        );
    }

    #[test]
    fn test_scaffold_packages_toml_has_rootfs_section() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            toml.contains("[rootfs]"),
            "packages.toml must contain a [rootfs] section"
        );
        assert!(
            toml.contains("image_out"),
            "packages.toml [rootfs] must contain image_out"
        );
        assert!(
            toml.contains("shell"),
            "packages.toml [rootfs] must contain shell"
        );
    }

    #[test]
    fn test_scaffold_packages_toml_rootfs_image_out_uses_name() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            toml.contains("dist/myapp-rootfs.ext4"),
            "image_out must be dist/<name>-rootfs.ext4"
        );
    }

    #[test]
    fn test_scaffold_without_non_root_omits_user_and_dir_entries() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            !toml.contains("[[rootfs.user]]"),
            "simple workload must not have [[rootfs.user]]"
        );
        assert!(
            !toml.contains("[[rootfs.dir]]"),
            "simple workload must not have [[rootfs.dir]]"
        );
    }

    #[test]
    fn test_scaffold_with_non_root_includes_user_dir_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = nonroot_cfg("opensearch", 1000);
        scaffold(&cfg, dir.path()).unwrap();
        let toml =
            fs::read_to_string(dir.path().join("packages/opensearch/packages.toml")).unwrap();
        assert!(
            toml.contains("[[rootfs.user]]"),
            "non-root workload must have [[rootfs.user]]"
        );
        assert!(
            toml.contains("uid  = 1000"),
            "[[rootfs.user]] must set uid to 1000"
        );
        assert!(
            toml.contains("/var/lib/opensearch"),
            "non-root workload must have /var/lib/<name> dir"
        );
        assert!(
            toml.contains("/var/log/opensearch"),
            "non-root workload must have /var/log/<name> dir"
        );
        assert!(
            toml.contains("owner = 1000"),
            "[[rootfs.dir]] entries must set owner = 1000"
        );
    }

    #[test]
    fn test_scaffold_packages_toml_has_entrypoint_file_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/packages.toml")).unwrap();
        assert!(
            toml.contains("[[rootfs.file]]"),
            "packages.toml must have [[rootfs.file]] for the entrypoint"
        );
        assert!(
            toml.contains("/bin/myapp-vm"),
            "entrypoint path must be /bin/<name>-vm"
        );
    }

    #[test]
    fn test_scaffold_vm_toml_references_pkg_rootfs_build() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = simple_cfg("myapp");
        scaffold(&cfg, dir.path()).unwrap();
        let toml = fs::read_to_string(dir.path().join("packages/myapp/vm.toml")).unwrap();
        assert!(
            toml.contains("pkg rootfs build"),
            "vm.toml must reference 'pkg rootfs build', not 'bash build-rootfs.sh'"
        );
        assert!(
            !toml.contains("build-rootfs.sh"),
            "vm.toml must not reference build-rootfs.sh"
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
