use serde::Deserialize;

use crate::spi::defaults::{CACHE_BASE_DEFAULT, CHANNEL_BASE_DEFAULT};

/// Top-level application configuration (`application.toml`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub nix: NixConfig,
}

/// `[nix]` section of `application.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct NixConfig {
    /// Binary cache base URL. Fetcher appends `/<hash>.narinfo` and `/<url>` paths.
    #[serde(default = "default_cache_base")]
    pub cache_base: String,

    /// NixOS channel index base URL. Resolver appends `/<channel>/store-paths.xz`.
    #[serde(default = "default_channel_base")]
    pub channel_base: String,
}

fn default_cache_base() -> String {
    CACHE_BASE_DEFAULT.to_string()
}

fn default_channel_base() -> String {
    CHANNEL_BASE_DEFAULT.to_string()
}

impl Default for NixConfig {
    fn default() -> Self {
        Self {
            cache_base: CACHE_BASE_DEFAULT.to_string(),
            channel_base: CHANNEL_BASE_DEFAULT.to_string(),
        }
    }
}
