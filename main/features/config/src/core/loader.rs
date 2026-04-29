use crate::api::config::AppConfig;
use crate::spi::xdg::config_search_dirs;

/// Load `application.toml` from the first matching XDG config path.
///
/// Search order:
///   1. `$XDG_CONFIG_HOME/justpkg/application.toml`
///   2. Each `$XDG_CONFIG_DIRS` entry: `<dir>/justpkg/application.toml`
///
/// Returns `AppConfig::default()` when no file is found. Logs a warning
/// to stderr and continues to the next candidate on parse failure.
pub fn load() -> AppConfig {
    for dir in config_search_dirs() {
        let path = dir.join("justpkg").join("application.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        match toml::from_str::<AppConfig>(&text) {
            Ok(cfg) => return cfg,
            Err(e) => {
                eprintln!("justpkg: warning: ignoring {:?}: {e}", path);
            }
        }
    }
    AppConfig::default()
}

#[cfg(test)]
mod tests_loader {
    use super::*;

    #[test]
    fn test_load_returns_default_when_no_config_file_exists() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/justpkg-nonexistent-xdg-test-dir");
        std::env::set_var("XDG_CONFIG_DIRS", "");
        let cfg = load();
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CONFIG_DIRS");
        assert_eq!(
            cfg.nix.cache_base, "https://cache.nixos.org",
            "default cache_base must be cache.nixos.org when no config file is found"
        );
    }

    #[test]
    fn test_load_reads_cache_base_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let justpkg_dir = dir.path().join("justpkg");
        std::fs::create_dir_all(&justpkg_dir).unwrap();
        std::fs::write(
            justpkg_dir.join("application.toml"),
            "[nix]\ncache_base = \"https://my-cache.example.com\"\n",
        )
        .unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::set_var("XDG_CONFIG_DIRS", "");
        let cfg = load();
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CONFIG_DIRS");
        assert_eq!(
            cfg.nix.cache_base, "https://my-cache.example.com",
            "cache_base from application.toml must override the default"
        );
    }

    #[test]
    fn test_load_falls_back_to_default_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let justpkg_dir = dir.path().join("justpkg");
        std::fs::create_dir_all(&justpkg_dir).unwrap();
        std::fs::write(justpkg_dir.join("application.toml"), "not valid toml }{").unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::set_var("XDG_CONFIG_DIRS", "");
        let cfg = load();
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CONFIG_DIRS");
        assert_eq!(
            cfg.nix.cache_base, "https://cache.nixos.org",
            "malformed TOML must fall back to default rather than panicking"
        );
    }

    #[test]
    fn test_load_prefers_xdg_config_home_over_xdg_config_dirs() {
        let home_dir = tempfile::tempdir().unwrap();
        let sys_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home_dir.path().join("justpkg")).unwrap();
        std::fs::create_dir_all(sys_dir.path().join("justpkg")).unwrap();
        std::fs::write(
            home_dir.path().join("justpkg").join("application.toml"),
            "[nix]\ncache_base = \"https://user-cache.example.com\"\n",
        )
        .unwrap();
        std::fs::write(
            sys_dir.path().join("justpkg").join("application.toml"),
            "[nix]\ncache_base = \"https://sys-cache.example.com\"\n",
        )
        .unwrap();
        std::env::set_var("XDG_CONFIG_HOME", home_dir.path());
        std::env::set_var("XDG_CONFIG_DIRS", sys_dir.path().to_str().unwrap());
        let cfg = load();
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_CONFIG_DIRS");
        assert_eq!(
            cfg.nix.cache_base, "https://user-cache.example.com",
            "XDG_CONFIG_HOME must take precedence over XDG_CONFIG_DIRS"
        );
    }
}
