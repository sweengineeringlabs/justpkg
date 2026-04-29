use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use cas::FsCas;
use justpkg_config::load as load_config;
use justpkg_nix::{FlakeLock, NixFetcher};
use justpkg_pkg::UreqClient;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config();
    let cache_base = config.nix.cache_base;

    match cli.command {
        Command::Build {
            flake_lock,
            dest_dir,
        } => {
            let lock = load_flake_lock(&flake_lock)?;
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create dest dir {:?}", dest_dir))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base };
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
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base };
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
            let fetcher = NixFetcher { http: &http, cache_base: &cache_base };
            fetcher
                .extract_from_cas(&lock, &cas, &dest_dir)
                .with_context(|| "NAR extract from CAS failed")?;
            eprintln!("done: {}", dest_dir.display());
        }
    }

    Ok(())
}

fn load_flake_lock(path: &PathBuf) -> Result<FlakeLock> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {:?}", path))?;
    FlakeLock::from_json(&text).with_context(|| format!("parse {:?}", path))
}
