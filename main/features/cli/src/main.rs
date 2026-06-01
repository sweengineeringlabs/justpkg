mod scaffold;

use clap::{Parser, Subcommand};
use edge_domain::{HandlerError, ServiceError};
use std::path::PathBuf;

use cas::FsCas;
use ext4::Filesystem;
use justpkg_config::load as load_config;
use justpkg_nix::{FlakeLock, NixFetcher};
use justpkg_pkg::UreqClient;
use justpkg_rootfs::{build_rootfs, verify_package};
use justpkg_vminit::{parse_manifest, VminitInstaller};

#[derive(Parser)]
#[command(name = "pkg", about = "Nix NAR fetcher and extractor")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch and extract all NARs from a flake.lock into a destination directory
    Build {
        flake_lock: PathBuf,
        dest_dir: PathBuf,
    },
    /// Fetch all NARs from a flake.lock to the local CAS cache
    Fetch {
        flake_lock: PathBuf,
    },
    /// Extract all NARs from a previously fetched flake.lock
    Extract {
        flake_lock: PathBuf,
        dest_dir: PathBuf,
    },

    /// Resolve a packages.toml to a vminit-compatible manifest.json
    ///
    /// Downloads the NixOS channel closure index (store-paths.xz) and
    /// writes manifest.json with name → absolute Nix store path entries.
    Resolve {
        packages_toml: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },

    /// Fetch and extract Nix packages from a manifest.json into a directory tree
    ///
    /// Substituters are tried in order: --substituter flags first, then
    /// application.toml, then cache.nixos.org. A 404 silently advances to the next.
    Install {
        manifest: PathBuf,
        dest_dir: PathBuf,
        #[arg(long = "package", short = 'p')]
        packages: Vec<String>,
        #[arg(long = "substituter", short = 's')]
        substituters: Vec<String>,
        #[arg(long = "substituter-token", short = 't')]
        substituter_tokens: Vec<String>,
    },

    /// Verify that every package in a manifest has valid ELF binaries in the ext4 image
    VerifyImage {
        manifest: PathBuf,
        image: PathBuf,
    },

    /// Build a rootfs ext4 image from a packages.toml with a [rootfs] section
    ///
    /// Requires a pre-resolved manifest.json alongside packages.toml.
    /// Run `pkg resolve <packages.toml> --out <dir>/manifest.json` first.
    ///
    /// Substituters are tried in order: --substituter flags first, then
    /// application.toml [[nix.substituters]], then cache.nixos.org.
    ///
    /// Examples:
    ///   pkg rootfs build packages/opensearch/packages.toml
    ///   pkg rootfs build packages/redis/packages.toml --out /tmp/redis-test.ext4
    Rootfs {
        #[command(subcommand)]
        command: RootfsCommand,
    },

    /// Scaffold a new workload package directory
    ///
    /// Creates packages/<name>/{packages.toml,build-rootfs.sh,vm.toml} in the
    /// current directory.  Fails if packages/<name> already exists.
    ///
    /// Examples:
    ///   pkg new redis --port 6379:6379
    ///   pkg new opensearch --non-root 1000 --port 9200:9200 --port 9300:9300 --memory 1024
    New {
        /// Workload name (becomes the directory and file prefix)
        name: String,
        /// UID/GID for the non-root user the workload runs as.
        /// Adds /etc/passwd, /etc/group, data dirs, and justext4 chown steps.
        #[arg(long)]
        non_root: Option<u32>,
        /// Port mapping in HOST:GUEST format (repeatable)
        #[arg(long = "port")]
        ports: Vec<PortMapping>,
        /// Guest memory in MiB
        #[arg(long, default_value_t = 512)]
        memory: u32,
        /// Number of vCPUs
        #[arg(long, default_value_t = 1)]
        vcpus: u32,
    },
}

#[derive(Subcommand)]
enum RootfsCommand {
    /// Build an ext4 rootfs image from the [rootfs] section of packages.toml
    Build {
        packages_toml: PathBuf,
        /// Override the image output path from [rootfs].image_out
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long = "substituter", short = 's')]
        substituters: Vec<String>,
        #[arg(long = "substituter-token", short = 't')]
        substituter_tokens: Vec<String>,
    },
}

#[derive(Clone)]
struct PortMapping(u16, u16);

impl std::str::FromStr for PortMapping {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (host, guest) = s
            .split_once(':')
            .ok_or_else(|| format!("expected HOST:GUEST, got {s:?}"))?;
        let host = host.parse::<u16>().map_err(|e| format!("invalid host port: {e}"))?;
        let guest = guest.parse::<u16>().map_err(|e| format!("invalid guest port: {e}"))?;
        Ok(PortMapping(host, guest))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), HandlerError> {
    let cli = Cli::parse();
    let config = load_config();
    let cache_base = config.nix.cache_base.clone();
    let channel_base = config.nix.channel_base.clone();

    match cli.command {
        Command::Build { flake_lock, dest_dir } => {
            let lock = load_flake_lock(&flake_lock)?;
            std::fs::create_dir_all(&dest_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create {dest_dir:?}: {e}")))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            fetcher.build(&lock, &dest_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("NAR fetch/extract: {e}")))?;
            eprintln!("done: {}", dest_dir.display());
        }

        Command::Fetch { flake_lock } => {
            let lock = load_flake_lock(&flake_lock)?;
            let cache_dir = cache_dir();
            std::fs::create_dir_all(&cache_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create cache dir: {e}")))?;
            let cas = FsCas::new(&cache_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("open CAS: {e}")))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            let map = fetcher.fetch_to_cas(&lock, &cas)
                .map_err(|e| HandlerError::ExecutionFailed(format!("NAR fetch to CAS: {e}")))?;
            for (name, digest) in &map {
                eprintln!("cached {name}: {digest}");
            }
        }

        Command::Extract { flake_lock, dest_dir } => {
            let lock = load_flake_lock(&flake_lock)?;
            let cache_dir = cache_dir();
            std::fs::create_dir_all(&cache_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create cache dir: {e}")))?;
            let cas = FsCas::new(&cache_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("open CAS: {e}")))?;
            std::fs::create_dir_all(&dest_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create {dest_dir:?}: {e}")))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            fetcher.extract_from_cas(&lock, &cas, &dest_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("NAR extract: {e}")))?;
            eprintln!("done: {}", dest_dir.display());
        }

        Command::Resolve { packages_toml, out } => {
            let spec = justpkg_resolve::load_packages_spec(&packages_toml)
                .map_err(svc_to_handler)?;
            let resolve_cache = cache_dir().join("resolve");
            std::fs::create_dir_all(&resolve_cache)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create resolve cache: {e}")))?;
            if let Some(parent) = out.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        HandlerError::ExecutionFailed(format!("create output dir: {e}"))
                    })?;
                }
            }
            let http = UreqClient;
            let manifest =
                justpkg_resolve::resolve(&http, &spec, &channel_base, &resolve_cache, &out)
                    .map_err(svc_to_handler)?;
            eprintln!(
                "resolved {} package(s) → {} (nixpkgs {})",
                manifest.packages.len(),
                out.display(),
                &manifest.meta.nixpkgs_rev[..8],
            );
        }

        Command::Install {
            manifest,
            dest_dir,
            packages,
            substituters: sub_flags,
            substituter_tokens: token_flags,
        } => {
            let text = std::fs::read_to_string(&manifest)
                .map_err(|e| HandlerError::InvalidRequest(format!("read {manifest:?}: {e}")))?;
            let pkg_manifest = parse_manifest(&text).map_err(svc_to_handler)?;
            std::fs::create_dir_all(&dest_dir)
                .map_err(|e| HandlerError::ExecutionFailed(format!("create {dest_dir:?}: {e}")))?;

            let all_names: Vec<String>;
            let names: Vec<&str> = if packages.is_empty() {
                all_names = pkg_manifest.entries.keys().cloned().collect();
                all_names.iter().map(|s| s.as_str()).collect()
            } else {
                packages.iter().map(|s| s.as_str()).collect()
            };

            let mut effective_subs: Vec<justpkg_config::SubstituterConfig> = sub_flags
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    let token = token_flags.get(i).filter(|t| !t.is_empty()).cloned();
                    justpkg_config::SubstituterConfig { url: u.clone(), token }
                })
                .collect();
            effective_subs.extend(config.nix.effective_substituters());

            let http = UreqClient;
            let installer =
                VminitInstaller::with_substituters(&http, pkg_manifest, effective_subs);
            installer.install(&names, &dest_dir).map_err(svc_to_handler)?;
            eprintln!("installed {} package(s) → {}", names.len(), dest_dir.display());
        }

        Command::Rootfs { command: RootfsCommand::Build { packages_toml, out, substituters: sub_flags, substituter_tokens: token_flags } } => {
            let cli_subs: Vec<justpkg_config::SubstituterConfig> = sub_flags
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    let token = token_flags.get(i).filter(|t| !t.is_empty()).cloned();
                    justpkg_config::SubstituterConfig { url: u.clone(), token }
                })
                .collect();
            let image = build_rootfs(&packages_toml, out, cli_subs, &config)?;
            eprintln!("Run: vmic run --config {}", packages_toml.with_file_name("vm.toml").display());
            let _ = image;
        }

        Command::New { name, non_root, ports, memory, vcpus } => {
            let cfg = scaffold::ScaffoldConfig {
                name,
                non_root,
                ports: ports.into_iter().map(|p| (p.0, p.1)).collect(),
                memory_mb: memory,
                vcpus,
            };
            scaffold::scaffold(&cfg, std::path::Path::new("."))?;
            eprintln!(
                "created packages/{name}/{{packages.toml,vm.toml}}",
                name = cfg.name
            );
            eprintln!();
            eprintln!("Next steps:");
            eprintln!(
                "  1. Edit packages/{name}/packages.toml — add workload packages and fill in [rootfs] entrypoint",
                name = cfg.name
            );
            eprintln!(
                "  2. pkg resolve packages/{name}/packages.toml --out packages/{name}/manifest.json",
                name = cfg.name
            );
            eprintln!(
                "  3. pkg rootfs build packages/{name}/packages.toml",
                name = cfg.name
            );
            eprintln!(
                "  4. vmic run --config packages/{name}/vm.toml",
                name = cfg.name
            );
        }

        Command::VerifyImage { manifest, image } => {
            let text = std::fs::read_to_string(&manifest)
                .map_err(|e| HandlerError::InvalidRequest(format!("read {manifest:?}: {e}")))?;
            let pkg_manifest = parse_manifest(&text).map_err(svc_to_handler)?;
            let f = std::fs::File::open(&image)
                .map_err(|e| HandlerError::InvalidRequest(format!("open image {image:?}: {e}")))?;
            let mut fs = Filesystem::open(f).map_err(|e| {
                HandlerError::ExecutionFailed(format!("open ext4 {image:?}: {e}"))
            })?;

            let mut all_ok = true;
            for (name, store_path) in &pkg_manifest.entries {
                match verify_package(&mut fs, name, store_path) {
                    Ok(count) => eprintln!("  ok  {name}: {count} ELF binaries verified"),
                    Err(e) => {
                        eprintln!("  FAIL {name}: {e}");
                        all_ok = false;
                    }
                }
            }
            if !all_ok {
                return Err(HandlerError::ExecutionFailed(
                    "one or more packages failed ELF verification".into(),
                ));
            }
            eprintln!("all {} package(s) verified", pkg_manifest.entries.len());
        }
    }

    Ok(())
}

/// Convert any type that bridges to [`ServiceError`] into a [`HandlerError`].
fn svc_to_handler<E: Into<ServiceError>>(e: E) -> HandlerError {
    HandlerError::from(e.into())
}

fn cache_dir() -> PathBuf {
    dirs_next::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("justpkg")
}

fn load_flake_lock(path: &PathBuf) -> Result<FlakeLock, HandlerError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| HandlerError::InvalidRequest(format!("read {path:?}: {e}")))?;
    FlakeLock::from_json(&text)
        .map_err(|e| HandlerError::InvalidRequest(format!("parse {path:?}: {e}")))
}

