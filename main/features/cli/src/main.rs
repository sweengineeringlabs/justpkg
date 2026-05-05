use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use cas::FsCas;
use ext4::{Ext4Error, Filesystem};
use justpkg_config::load as load_config;
use justpkg_nix::{FlakeLock, NixFetcher};
use justpkg_pkg::UreqClient;
use justpkg_vminit::{VminitInstaller, parse_manifest};

#[derive(Parser)]
#[command(name = "justpkg", about = "Nix NAR fetcher and extractor")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch and extract all NARs from a flake.lock into a destination directory
    Build {
        /// Path to flake.lock
        flake_lock: PathBuf,
        /// Destination directory for extracted packages
        dest_dir: PathBuf,
    },
    /// Fetch all NARs from a flake.lock to the local CAS cache
    Fetch {
        /// Path to flake.lock
        flake_lock: PathBuf,
    },
    /// Extract all NARs from a previously fetched flake.lock
    Extract {
        /// Path to flake.lock
        flake_lock: PathBuf,
        /// Destination directory for extracted packages
        dest_dir: PathBuf,
    },

    /// Resolve a packages.toml to a vminit-compatible manifest.json
    ///
    /// Downloads the NixOS channel closure index (store-paths.xz) and
    /// writes manifest.json with name → absolute Nix store path entries.
    Resolve {
        /// Path to packages.toml
        packages_toml: PathBuf,
        /// Path to write manifest.json
        #[arg(long)]
        out: PathBuf,
    },

    /// Fetch and extract Nix packages from a manifest.json into a directory tree
    ///
    /// Reads manifest.json produced by `justpkg resolve`, downloads each
    /// package's NAR closure from the binary cache, and extracts into
    /// <dest-dir>/nix/store/<hash>-<name>.  If no --package flags are given,
    /// all packages in the manifest are installed.
    ///
    /// Substituters are tried in order: --substituter flags first, then any
    /// substituters from application.toml, then cache.nixos.org as final
    /// fallback.  A 404 from one cache silently advances to the next.
    Install {
        /// Path to manifest.json
        manifest: PathBuf,
        /// Destination directory (packages land under <dest-dir>/nix/store/…)
        dest_dir: PathBuf,
        /// Package names to install (repeatable; default: all in manifest)
        #[arg(long = "package", short = 'p')]
        packages: Vec<String>,
        /// Binary cache URL to prepend to the substituter list (repeatable).
        /// Tried before application.toml substituters.
        /// Example: --substituter https://cache.swe.internal/swe-private
        #[arg(long = "substituter", short = 's')]
        substituters: Vec<String>,
        /// Bearer token for the substituter at the same position in the
        /// --substituter list (repeatable, positional pairing).
        /// The first --substituter-token pairs with the first --substituter.
        /// Omit for unauthenticated caches (e.g. cache.nixos.org).
        #[arg(long = "substituter-token", short = 't')]
        substituter_tokens: Vec<String>,
    },

    /// Verify that every package in a manifest has valid ELF binaries in the ext4 image
    ///
    /// For each store path in manifest.json, looks for a `bin/` directory in the
    /// ext4 image and checks that every file there starts with the ELF magic bytes.
    /// Exits non-zero if any package is missing or any binary fails the ELF check.
    VerifyImage {
        /// Path to manifest.json (produced by `justpkg resolve`)
        manifest: PathBuf,
        /// Path to ext4 image file
        image: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config();
    let cache_base = config.nix.cache_base.clone();
    let channel_base = config.nix.channel_base.clone();

    match cli.command {
        Command::Build {
            flake_lock,
            dest_dir,
        } => {
            let lock = load_flake_lock(&flake_lock)?;
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create dest dir {:?}", dest_dir))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            fetcher
                .build(&lock, &dest_dir)
                .with_context(|| "NAR fetch/extract failed")?;
            eprintln!("done: {}", dest_dir.display());
        }
        Command::Fetch { flake_lock } => {
            let lock = load_flake_lock(&flake_lock)?;
            let cache_dir = dirs_next::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("justpkg");
            std::fs::create_dir_all(&cache_dir)
                .with_context(|| format!("create cache dir {:?}", cache_dir))?;
            let cas =
                FsCas::new(&cache_dir).with_context(|| format!("open CAS at {:?}", cache_dir))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            let map = fetcher
                .fetch_to_cas(&lock, &cas)
                .with_context(|| "NAR fetch to CAS failed")?;
            for (name, digest) in &map {
                eprintln!("cached {name}: {digest}");
            }
        }
        Command::Extract {
            flake_lock,
            dest_dir,
        } => {
            let lock = load_flake_lock(&flake_lock)?;
            let cache_dir = dirs_next::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("justpkg");
            std::fs::create_dir_all(&cache_dir)
                .with_context(|| format!("create cache dir {:?}", cache_dir))?;
            let cas =
                FsCas::new(&cache_dir).with_context(|| format!("open CAS at {:?}", cache_dir))?;
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create dest dir {:?}", dest_dir))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base, token: None };
            fetcher
                .extract_from_cas(&lock, &cas, &dest_dir)
                .with_context(|| "NAR extract from CAS failed")?;
            eprintln!("done: {}", dest_dir.display());
        }

        Command::Resolve { packages_toml, out } => {
            let spec = justpkg_resolve::load_packages_spec(&packages_toml)
                .with_context(|| format!("load {:?}", packages_toml))?;
            let resolve_cache = dirs_next::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("justpkg")
                .join("resolve");
            std::fs::create_dir_all(&resolve_cache)
                .with_context(|| format!("create resolve cache dir {:?}", resolve_cache))?;
            if let Some(parent) = out.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create output dir {:?}", parent))?;
                }
            }
            let http = UreqClient;
            let manifest = justpkg_resolve::resolve(
                &http,
                &spec,
                &channel_base,
                &resolve_cache,
                &out,
            )
            .with_context(|| format!("resolve {:?}", packages_toml))?;
            eprintln!(
                "resolved {} package(s) → {} (nixpkgs {})",
                manifest.packages.len(),
                out.display(),
                &manifest.meta.nixpkgs_rev[..8],
            );
        }

        Command::Install { manifest, dest_dir, packages, substituters: sub_flags, substituter_tokens: token_flags } => {
            let text = std::fs::read_to_string(&manifest)
                .with_context(|| format!("read {:?}", manifest))?;
            let pkg_manifest = parse_manifest(&text)
                .with_context(|| format!("parse {:?}", manifest))?;
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create dest dir {:?}", dest_dir))?;

            let all_names: Vec<String>;
            let names: Vec<&str> = if packages.is_empty() {
                all_names = pkg_manifest.entries.keys().cloned().collect();
                all_names.iter().map(|s| s.as_str()).collect()
            } else {
                packages.iter().map(|s| s.as_str()).collect()
            };

            // CLI --substituter flags prepend the config substituter list.
            // --substituter-token[i] supplies the Bearer token for --substituter[i].
            // Substituters without a matching token are unauthenticated.
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
            let installer = VminitInstaller::with_substituters(&http, pkg_manifest, effective_subs);
            installer
                .install(&names, &dest_dir)
                .with_context(|| format!("install packages into {:?}", dest_dir))?;
            eprintln!("installed {} package(s) → {}", names.len(), dest_dir.display());
        }

        Command::VerifyImage { manifest, image } => {
            let text = std::fs::read_to_string(&manifest)
                .with_context(|| format!("read {:?}", manifest))?;
            let pkg_manifest = parse_manifest(&text)
                .with_context(|| format!("parse {:?}", manifest))?;
            let f = std::fs::File::open(&image)
                .with_context(|| format!("open image {:?}", image))?;
            let mut fs = Filesystem::open(f)
                .with_context(|| format!("open ext4 {:?}", image))?;

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
                bail!("one or more packages failed ELF verification");
            }
            eprintln!("all {} package(s) verified", pkg_manifest.entries.len());
        }
    }

    Ok(())
}

fn load_flake_lock(path: &PathBuf) -> Result<FlakeLock> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {:?}", path))?;
    FlakeLock::from_json(&text).with_context(|| format!("parse {:?}", path))
}

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const SHEBANG: [u8; 2] = [b'#', b'!'];

/// Open `<store_path>/bin/` inside the ext4 image and verify every
/// regular file starts with ELF magic. Returns the count of binaries
/// checked, or an error describing the first failure.
fn verify_package<R: std::io::Read + std::io::Seek>(
    fs: &mut Filesystem<R>,
    name: &str,
    store_path: &str,
) -> Result<usize> {
    // Verify the store directory itself exists.
    fs.open_path(store_path)
        .with_context(|| format!("store path {store_path} not found in image"))?;

    // bin/ is optional (some packages are pure libraries).
    let bin_path = format!("{store_path}/bin");
    let bin_inode_num = match fs.open_path(&bin_path) {
        Ok(n) => n,
        Err(Ext4Error::NotFound { .. }) => return Ok(0),
        Err(e) => return Err(e).with_context(|| format!("open {bin_path}")),
    };
    let bin_inode = fs.read_inode(bin_inode_num)
        .with_context(|| format!("read inode for {bin_path}"))?;
    let entries = fs.read_dir(&bin_inode)
        .with_context(|| format!("read dir {bin_path}"))?;

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
        let file_inode_num = fs.open_path(&file_path)
            .with_context(|| format!("open {file_path}"))?;
        let file_inode = fs.read_inode(file_inode_num)
            .with_context(|| format!("read inode {file_path}"))?;
        if !file_inode.is_regular() {
            continue;
        }
        let data = fs.read_file(&file_inode)
            .with_context(|| format!("read file {file_path}"))?;
        let is_elf = data.len() >= 4 && data[..4] == ELF_MAGIC;
        let is_script = data.len() >= 2 && data[..2] == SHEBANG;
        if !is_elf && !is_script {
            bail!(
                "{name}: {file_path} is not an ELF binary or shell script (first bytes: {:?})",
                &data[..data.len().min(4)]
            );
        }
        count += 1;
    }
    Ok(count)
}
