pub use crate::api::error::{is_not_found, NixFetchError};
pub use crate::api::flake_lock::{FlakeLock, LockedNode};
pub use crate::api::narinfo::NarInfo;
pub use crate::spi::fetcher::NixFetcher;
pub use crate::spi::nar::extract_nar;
pub use crate::spi::nix_hash::{nix_base32_to_hex, sri_to_hex};

/// Default Nix binary cache URL. Use this when no `application.toml` is loaded.
pub const DEFAULT_CACHE_BASE: &str = "https://cache.nixos.org";
