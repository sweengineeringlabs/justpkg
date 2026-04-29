use std::path::PathBuf;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use justpkg_core::spi::UreqClient;
use justpkg_nix::{FlakeLock, spi::NixFetcher};

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

    match cli.command {
        Command::Build { flake_lock, dest_dir } => {
            let lock = load_flake_lock(&flake_lock)?;
            std::fs::create_dir_all(&dest_dir)
                .with_context(|| format!("create dest dir {:?}", dest_dir))?;
            let http = UreqClient;
            let fetcher = NixFetcher { http: &http };
            fetcher.build(&lock, &dest_dir)
                .with_context(|| "NAR fetch/extract failed")?;
            eprintln!("done: {}", dest_dir.display());
        }
        Command::Fetch { flake_lock } => {
            let _lock = load_flake_lock(&flake_lock)?;
            eprintln!("fetch-only not yet implemented — use 'build'");
        }
        Command::Extract { flake_lock, dest_dir } => {
            let _lock = load_flake_lock(&flake_lock)?;
            eprintln!("extract-only not yet implemented — use 'build'");
            let _ = dest_dir;
        }
    }

    Ok(())
}

fn load_flake_lock(path: &PathBuf) -> Result<FlakeLock> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read {:?}", path))?;
    FlakeLock::from_json(&text)
        .with_context(|| format!("parse {:?}", path))
}

