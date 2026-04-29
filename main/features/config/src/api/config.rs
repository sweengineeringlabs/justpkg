use serde::Deserialize;

use crate::spi::defaults::CACHE_BASE_DEFAULT;

/// Top-level application configuration (`application.toml`).
#[derive(Debug, Clone, Deserialize)]
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
}

fn default_cache_base() -> String {
    CACHE_BASE_DEFAULT.to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { nix: NixConfig::default() }
    }
}

impl Default for NixConfig {
    fn default() -> Self {
        Self { cache_base: CACHE_BASE_DEFAULT.to_string() }
    }
}
