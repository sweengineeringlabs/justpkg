use edge_configbuilder::ConfigLoaderFactory;

use crate::api::config::{AppConfig, NixConfig};

/// Load `application.toml` from the XDG justpkg config directories.
///
/// Search order (last-wins overlay):
///   1. `$XDG_CONFIG_DIRS/justpkg/application.toml`
///   2. `$XDG_CONFIG_HOME/justpkg/application.toml`
///   3. `$SWE_EDGE_CONFIG_DIR/application.toml` (if set)
///
/// Returns `AppConfig::default()` when no file is found. Logs a warning to
/// stderr and returns the default when the loader cannot be initialised or
/// the `[nix]` section fails to parse.
pub fn load() -> AppConfig {
    let loader = match ConfigLoaderFactory::create_loader_xdg("justpkg") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("justpkg: warning: config loader init failed: {e}");
            return AppConfig::default();
        }
    };
    let nix = match loader.load_section::<NixConfig>("nix") {
        Ok(n) => n,
        Err(e) => {
            eprintln!("justpkg: warning: {e}");
            NixConfig::default()
        }
    };
    AppConfig { nix }
}

#[cfg(test)]
mod tests_loader {
    use edge_configbuilder::ConfigLoaderFactory;

    use crate::api::config::NixConfig;

    #[test]
    fn test_load_returns_default_when_no_config_file_exists() {
        // edge-configbuilder returns NotFound when no application.toml is present at all.
        // Our load() maps that to NixConfig::default(); verify the default values directly.
        let dir = tempfile::tempdir().unwrap();
        let loader = ConfigLoaderFactory::create_loader_for_dir(dir.path());
        let nix: NixConfig = loader.load_section("nix").unwrap_or_default();
        assert_eq!(
            nix.cache_base, "https://cache.nixos.org",
            "default cache_base must be cache.nixos.org when no config file is found"
        );
    }

    #[test]
    fn test_load_reads_cache_base_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("application.toml"),
            "[nix]\ncache_base = \"https://my-cache.example.com\"\n",
        )
        .unwrap();
        let loader = ConfigLoaderFactory::create_loader_for_dir(dir.path());
        let nix: NixConfig = loader.load_section("nix").unwrap();
        assert_eq!(
            nix.cache_base, "https://my-cache.example.com",
            "cache_base from application.toml must override the default"
        );
    }

    #[test]
    fn test_load_surfaces_error_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("application.toml"), "not valid toml }{").unwrap();
        let loader = ConfigLoaderFactory::create_loader_for_dir(dir.path());
        // edge-configbuilder surfaces malformed TOML as Err rather than silently defaulting.
        // Our load() maps this to AppConfig::default() + a warning — but the raw error
        // is visible here so operators can diagnose misconfiguration.
        let result = loader.load_section::<NixConfig>("nix");
        assert!(result.is_err(), "malformed TOML must produce a parse error");
    }

    #[test]
    fn test_load_reads_substituter_list_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("application.toml"),
            r#"
[nix]
cache_base = "https://cache.nixos.org"

[[nix.substituters]]
url = "https://private.cache.example"
token = "secret-token"

[[nix.substituters]]
url = "https://cache.nixos.org"
"#,
        )
        .unwrap();
        let loader = ConfigLoaderFactory::create_loader_for_dir(dir.path());
        let nix: NixConfig = loader.load_section("nix").unwrap();
        assert_eq!(nix.substituters.len(), 2, "both substituters must be loaded");
        assert_eq!(nix.substituters[0].url, "https://private.cache.example");
        assert_eq!(nix.substituters[0].token.as_deref(), Some("secret-token"));
        assert_eq!(nix.substituters[1].url, "https://cache.nixos.org");
        assert!(nix.substituters[1].token.is_none());
    }
}
