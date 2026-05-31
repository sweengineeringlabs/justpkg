use clap::{Parser, Subcommand};
use edge_domain::{HandlerError, ServiceError};
use std::path::PathBuf;

use cas::FsCas;
use ext4::{Ext4Error, Filesystem};
use justpkg_config::load as load_config;
use justpkg_nix::{FlakeLock, NixFetcher};
use justpkg_pkg::UreqClient;
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

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const SHEBANG: [u8; 2] = [b'#', b'!'];

fn verify_package<R: std::io::Read + std::io::Seek>(
    fs: &mut Filesystem<R>,
    name: &str,
    store_path: &str,
) -> Result<usize, HandlerError> {
    fs.open_path(store_path).map_err(|e| {
        HandlerError::NotFound(format!("store path {store_path} not found in image: {e}"))
    })?;

    let bin_path = format!("{store_path}/bin");
    let bin_inode_num = match fs.open_path(&bin_path) {
        Ok(n) => n,
        Err(Ext4Error::NotFound { .. }) => return Ok(0),
        Err(e) => return Err(HandlerError::ExecutionFailed(format!("open {bin_path}: {e}"))),
    };
    let bin_inode = fs
        .read_inode(bin_inode_num)
        .map_err(|e| HandlerError::ExecutionFailed(format!("read inode {bin_path}: {e}")))?;
    let entries = fs
        .read_dir(&bin_inode)
        .map_err(|e| HandlerError::ExecutionFailed(format!("read dir {bin_path}: {e}")))?;

    let mut count = 0usize;
    for entry in &entries {
        if entry.is_unused() {
            continue;
        }
        let entry_name = std::str::from_utf8(&entry.name).unwrap_or("<non-utf8>");
        if entry_name == "." || entry_name == ".." {
            continue;
        }
        let file_path = format!("{bin_path}/{entry_name}");
        let file_inode_num = fs
            .open_path(&file_path)
            .map_err(|e| HandlerError::ExecutionFailed(format!("open {file_path}: {e}")))?;
        let file_inode = fs
            .read_inode(file_inode_num)
            .map_err(|e| HandlerError::ExecutionFailed(format!("read inode {file_path}: {e}")))?;
        if !file_inode.is_regular() {
            continue;
        }
        let data = fs
            .read_file(&file_inode)
            .map_err(|e| HandlerError::ExecutionFailed(format!("read file {file_path}: {e}")))?;
        let is_elf = data.len() >= 4 && data[..4] == ELF_MAGIC;
        let is_script = data.len() >= 2 && data[..2] == SHEBANG;
        if !is_elf && !is_script {
            return Err(HandlerError::ExecutionFailed(format!(
                "{name}: {file_path} is not an ELF binary or shell script (first bytes: {:?})",
                &data[..data.len().min(4)]
            )));
        }
        count += 1;
    }
    Ok(count)
}
