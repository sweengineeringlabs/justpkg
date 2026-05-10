use serde::Deserialize;

use crate::spi::defaults::{CACHE_BASE_DEFAULT, CHANNEL_BASE_DEFAULT};

/// A Nix binary cache substituter: a base URL and an optional Bearer token.
///
/// Tokens are read from `application.toml` only; they must never appear in
/// checked-in manifest files.
#[derive(Debug, Clone, Deserialize)]
pub struct SubstituterConfig {
    /// Base URL of the binary cache, e.g. `"https://cache.nixos.org"` or
    /// `"https://cache.swe.internal/swe-private"`.
    pub url: String,

    /// Optional Bearer token for authenticated caches (Attic, Cachix private).
    /// When set, requests carry `Authorization: Bearer <token>`.
    #[serde(default)]
    pub token: Option<String>,
}

impl SubstituterConfig {
    /// Construct an unauthenticated substituter from a URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: None,
        }
    }
}

/// Top-level application configuration (`application.toml`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub nix: NixConfig,
}

/// `[nix]` section of `application.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct NixConfig {
    /// Single binary cache base URL — kept for backwards compatibility.
    /// Ignored when `substituters` is non-empty.
    #[serde(default = "default_cache_base")]
    pub cache_base: String,

    /// Ordered list of substituters tried in priority order.
    /// When non-empty, `cache_base` is ignored.
    /// Each entry may carry an optional Bearer token for authenticated caches.
    #[serde(default)]
    pub substituters: Vec<SubstituterConfig>,

    /// NixOS channel index base URL. Resolver appends `/<channel>/store-paths.xz`.
    #[serde(default = "default_channel_base")]
    pub channel_base: String,
}

impl NixConfig {
    /// Returns the effective ordered substituter list.
    ///
    /// If `substituters` is non-empty, returns it directly.
    /// Otherwise wraps `cache_base` as a single unauthenticated entry so callers
    /// never have to handle the "no substituters" case.
    pub fn effective_substituters(&self) -> Vec<SubstituterConfig> {
        if !self.substituters.is_empty() {
            self.substituters.clone()
        } else {
            vec![SubstituterConfig::new(&self.cache_base)]
        }
    }
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
            substituters: Vec::new(),
            channel_base: CHANNEL_BASE_DEFAULT.to_string(),
        }
    }
}
