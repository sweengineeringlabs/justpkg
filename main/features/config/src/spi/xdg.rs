/// Returns XDG config search dirs (XDG_CONFIG_HOME, then XDG_CONFIG_DIRS).
pub fn config_search_dirs() -> Vec<std::path::PathBuf> {
    config_search_dirs_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("XDG_CONFIG_DIRS").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

pub fn config_search_dirs_from(
    xdg_config_home: Option<&str>,
    xdg_config_dirs: Option<&str>,
    home: Option<&str>,
) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    match xdg_config_home {
        Some(val) if !val.is_empty() => dirs.push(val.into()),
        _ => {
            if let Some(home) = home {
                dirs.push(std::path::Path::new(home).join(".config"));
            }
        }
    }
    let system = xdg_config_dirs.unwrap_or("/etc/xdg");
    let sep = if cfg!(windows) { ';' } else { ':' };
    for entry in system.split(sep).filter(|s| !s.is_empty()) {
        dirs.push(entry.into());
    }
    dirs
}

#[cfg(test)]
mod tests_xdg {
    use super::*;

    #[test]
    fn test_config_search_dirs_contains_at_least_one_entry() {
        assert!(!config_search_dirs().is_empty());
    }

    #[test]
    fn test_config_search_dirs_from_xdg_config_home_is_first() {
        let dirs = config_search_dirs_from(Some("/tmp/test-xdg"), None, Some("/home/user"));
        assert_eq!(dirs[0], std::path::PathBuf::from("/tmp/test-xdg"));
    }

    #[test]
    fn test_config_search_dirs_from_fallback_home_dot_config() {
        let dirs = config_search_dirs_from(None, None, Some("/home/user"));
        assert_eq!(dirs[0], std::path::PathBuf::from("/home/user/.config"));
    }

    #[cfg(unix)]
    #[test]
    fn test_config_search_dirs_from_xdg_config_dirs_appended() {
        let dirs = config_search_dirs_from(None, Some("/etc/foo:/etc/bar"), Some("/home/user"));
        assert!(dirs.iter().any(|d| d == std::path::Path::new("/etc/foo")));
        assert!(dirs.iter().any(|d| d == std::path::Path::new("/etc/bar")));
    }

    #[cfg(unix)]
    #[test]
    fn test_config_search_dirs_from_default_system_dir_when_no_xdg_config_dirs() {
        let dirs = config_search_dirs_from(None, None, None);
        assert!(
            dirs.iter().any(|d| d == std::path::Path::new("/etc/xdg")),
            "must fall back to /etc/xdg when XDG_CONFIG_DIRS is absent"
        );
    }
}
