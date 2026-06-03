//! `pkg flake build` — build packages from a Nix flake, write manifest.json.
//!
//! When `[profile]` is set in `packages.toml`, justpkg generates a temporary
//! flake that applies the requested nixpkgs overrides (icuSupport=false, etc.)
//! directly — no named variant outputs needed in the workload flake.
//!
//! When no profile is set, the existing `attr` field selects a named flake
//! output (backward-compatible with `postgresql_16_slim` etc.).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use edge_domain::HandlerError;
use justpkg_config::SubstituterConfig;
use justpkg_resolve::BuildProfile;

// ── Public entry point ────────────────────────────────────────────────────────

pub fn build(
    packages_toml: &Path,
    substituter: Option<&str>,
    token: Option<&str>,
    out: Option<&Path>,
) -> Result<(), HandlerError> {
    let spec = justpkg_resolve::load_packages_spec(packages_toml)
        .map_err(|e| HandlerError::InvalidRequest(format!("load {}: {e}", packages_toml.display())))?;

    let flake_dir = packages_toml
        .parent()
        .unwrap_or_else(|| Path::new("."));

    if !flake_dir.join("flake.nix").exists() {
        return Err(HandlerError::InvalidRequest(format!(
            "no flake.nix found at {} — pkg flake build requires a flake alongside packages.toml",
            flake_dir.display()
        )));
    }

    let system = &spec.system;
    let mut entries: BTreeMap<String, String> = BTreeMap::new();

    // When a [profile] is present, generate a temporary flake that applies
    // the profile overrides directly — no named variant outputs needed.
    let temp_flake = spec.profile.as_ref()
        .map(|p| generate_profile_flake(flake_dir, &spec.packages, system, p))
        .transpose()?;

    let effective_flake_dir: &Path = temp_flake
        .as_ref()
        .map(|t| t.path())
        .unwrap_or(flake_dir);

    for pkg in &spec.packages {
        let output = if temp_flake.is_some() {
            // Profile-driven: each package gets a dedicated output named by its name.
            format!("packages.{system}.{}", pkg.name.replace('-', "_"))
        } else {
            // Attr-driven: use explicit attr or fall back to name.
            let attr = if pkg.attr.is_empty() || pkg.attr == pkg.name {
                pkg.name.replace('-', "_")
            } else {
                pkg.attr.replace('-', "_")
            };
            format!("packages.{system}.{attr}")
        };

        eprintln!("==> nix build .#{output}");

        let result = run_nix(effective_flake_dir, &[
            "build", &format!(".#{output}"),
            "--print-out-paths", "--no-link",
        ])
        .map_err(|e| HandlerError::ExecutionFailed(format!("nix build: {e}")))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(HandlerError::ExecutionFailed(format!(
                "nix build .#{output} failed:\n{stderr}"
            )));
        }

        let all_paths: Vec<&str> = std::str::from_utf8(&result.stdout)
            .unwrap_or("")
            .lines()
            .filter(|l| !l.is_empty())
            .collect();

        if all_paths.is_empty() {
            return Err(HandlerError::ExecutionFailed(format!(
                "nix build .#{output} produced no output path"
            )));
        }

        let primary = pick_primary_output(&all_paths);
        eprintln!("    {primary}");

        if let Some(sub_url) = substituter {
            push_to_substituter(effective_flake_dir, sub_url, token, &all_paths);
        }

        entries.insert(pkg.name.clone(), primary.to_string());
    }

    let manifest_path = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| flake_dir.join("manifest.json"));

    let manifest = serde_json::json!({
        "packages": entries,
        "meta": {
            "nixpkgs_rev":  "flake-build",
            "channel":      spec.nixpkgs_channel,
            "resolved_at":  std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs().to_string())
                                .unwrap_or_else(|_| "0".to_string()),
        }
    });
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap())
        .map_err(|e| HandlerError::ExecutionFailed(format!("write {}: {e}", manifest_path.display())))?;

    eprintln!();
    eprintln!("Done. Wrote {} package(s) to {}", entries.len(), manifest_path.display());
    eprintln!("Next: pkg rootfs build {}", packages_toml.display());
    Ok(())
}

// ── Profile-driven temporary flake ───────────────────────────────────────────

/// Generate a temporary flake that applies the profile overrides to each package.
/// Returns a `TempFlake` handle that cleans up on drop.
fn generate_profile_flake(
    workload_flake_dir: &Path,
    packages: &[justpkg_resolve::PackageEntry],
    system: &str,
    profile: &BuildProfile,
) -> Result<TempFlake, HandlerError> {
    let dir = tempfile::tempdir()
        .map_err(|e| HandlerError::ExecutionFailed(format!("create temp flake dir: {e}")))?;

    // Inherit the workload flake's inputs so nixpkgs is already pinned.
    // Each package gets an output with overrides computed from the profile.
    let flake_content = generate_self_contained_flake(
        workload_flake_dir, packages, system, profile,
    )?;

    let flake_path = dir.path().join("flake.nix");
    std::fs::write(&flake_path, &flake_content)
        .map_err(|e| HandlerError::ExecutionFailed(format!("write temp flake.nix: {e}")))?;

    // Write a minimal flake.lock that Nix won't reject.
    // We'll let `nix flake update` generate it on first use.
    eprintln!("[profile] generated temporary flake at {}", dir.path().display());

    Ok(TempFlake(dir))
}

/// Generate a self-contained flake using the nixpkgs revision from the
/// workload's flake.lock. Applies profile overrides to each package.
fn generate_self_contained_flake(
    workload_flake_dir: &Path,
    packages: &[justpkg_resolve::PackageEntry],
    system: &str,
    profile: &BuildProfile,
) -> Result<String, HandlerError> {
    // Parse the workload's flake.lock to find the nixpkgs-25_05 or nixpkgs revision.
    let lock_path = workload_flake_dir.join("flake.lock");
    let lock_text = std::fs::read_to_string(&lock_path)
        .map_err(|e| HandlerError::InvalidRequest(format!(
            "read flake.lock at {}: {e}", lock_path.display()
        )))?;
    let lock: serde_json::Value = serde_json::from_str(&lock_text)
        .map_err(|e| HandlerError::InvalidRequest(format!("parse flake.lock: {e}")))?;

    // Prefer nixpkgs-25_05 (has icuSupport, jitSupport params), fall back to nixpkgs.
    let nixpkgs_url = find_nixpkgs_url(&lock, &["nixpkgs-25_05", "nixpkgs"])
        .ok_or_else(|| HandlerError::InvalidRequest(
            "flake.lock has no nixpkgs or nixpkgs-25_05 input".to_string()
        ))?;

    let mut pkg_outputs = String::new();
    for pkg in packages {
        let attr = pkg.name.replace('-', "_");
        let overrides = nixpkgs_overrides(&pkg.name, profile);
        // .override {} for feature flags; .overrideAttrs {} for build behaviour.
        // Build the derivation expression with correct Nix operator precedence.
        // `f { }.attr` in Nix parses as `f ({ }.attr)` — parentheses required
        // when chaining .override {} and .overrideAttrs {}.
        let needs_check_override = matches!(profile.check, Some(false));
        let has_overrides = !overrides.is_empty();

        let expr = match (has_overrides, needs_check_override) {
            (false, false) => format!("pkgs.{}", pkg.name),
            (true,  false) => {
                let pairs: Vec<String> = overrides.iter()
                    .map(|(k, v)| format!("{k} = {};", if *v { "true" } else { "false" }))
                    .collect();
                format!("pkgs.{}.override {{ {} }}", pkg.name, pairs.join(" "))
            }
            (false, true)  =>
                format!("pkgs.{}.overrideAttrs (_: {{ doCheck = false; doInstallCheck = false; }})", pkg.name),
            (true,  true)  => {
                let pairs: Vec<String> = overrides.iter()
                    .map(|(k, v)| format!("{k} = {};", if *v { "true" } else { "false" }))
                    .collect();
                // Parentheses ensure `.overrideAttrs` is called on the RESULT of
                // `.override {}`, not parsed as `override ({ }.overrideAttrs)`.
                format!("(pkgs.{}.override {{ {} }}).overrideAttrs (_: {{ doCheck = false; doInstallCheck = false; }})",
                    pkg.name, pairs.join(" "))
            }
        };

        // Output name uses underscores (valid Nix identifier); nixpkgs attr
        // keeps hyphens (e.g. su-exec). Both are valid Nix identifiers.
        pkg_outputs.push_str(&format!("        {attr} = {expr};\n"));
    }

    Ok(format!(
        r#"{{
  inputs.nixpkgs.url = "{nixpkgs_url}";

  outputs = {{ self, nixpkgs }}:
    let
      pkgs = nixpkgs.legacyPackages.{system};
    in
    {{
      packages.{system} = {{
{pkg_outputs}
      }};
    }};
}}"#,
        nixpkgs_url = nixpkgs_url,
        system = system,
        pkg_outputs = pkg_outputs,
    ))
}

/// Extract the nixpkgs GitHub URL from a flake.lock node, trying keys in order.
fn find_nixpkgs_url(lock: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let nodes = lock.get("nodes")?;
    for key in keys {
        if let Some(node) = nodes.get(*key) {
            if let Some(locked) = node.get("locked") {
                let owner = locked.get("owner")?.as_str()?;
                let repo  = locked.get("repo")?.as_str()?;
                let rev   = locked.get("rev")?.as_str()?;
                return Some(format!("github:{owner}/{repo}/{rev}"));
            }
        }
    }
    None
}

/// Temporary flake directory — cleaned up on drop.
struct TempFlake(tempfile::TempDir);

impl TempFlake {
    fn path(&self) -> &Path {
        self.0.path()
    }
}

// ── Profile → nixpkgs parameter mapping ──────────────────────────────────────

/// Map `[profile]` flags to the correct nixpkgs `override {}` parameter names
/// for the given package. Returns only parameters explicitly set to `false`.
///
/// The mapping is package-specific because nixpkgs uses different parameter
/// names across derivations (e.g. `withSystemd` for redis, `systemdSupport`
/// for postgres). Unknown packages return an empty map — overrides are silently
/// skipped, preserving the nixpkgs defaults.
pub fn nixpkgs_overrides(package_name: &str, profile: &BuildProfile) -> BTreeMap<&'static str, bool> {
    let mut m = BTreeMap::new();

    // Emit an override for any explicitly-set flag, preserving its value, so the
    // mapping is bidirectional and per-feature:
    //   Some(false) → param = false (feature OFF)
    //   Some(true)  → param = true  (feature ON, explicit)
    //   None        → no override — nixpkgs default applies
    macro_rules! set {
        ($map:expr, $key:expr, $val:expr) => {
            if let Some(v) = $val { $map.insert($key, v); }
        };
    }

    match package_name {
        n if n.starts_with("postgresql") => {
            set!(m, "icuSupport",    profile.icu);
            set!(m, "jitSupport",    profile.jit);
            set!(m, "pythonSupport", profile.python);
            set!(m, "perlSupport",   profile.perl);
            set!(m, "tclSupport",    profile.tcl);
            set!(m, "pamSupport",    profile.pam);
            set!(m, "gssSupport",    profile.gss);
            set!(m, "systemdSupport",profile.systemd);
        }
        "redis" => {
            set!(m, "tlsSupport", profile.tls);
            set!(m, "withSystemd",profile.systemd);
        }
        _ => {} // unknown package — no overrides, use nixpkgs defaults
    }

    m
}

// ── Output path helpers ───────────────────────────────────────────────────────

/// Pick the primary store path from `--print-out-paths` output.
/// Split-output derivations (postgresql → main + -man + -doc + -lib) return
/// multiple lines; the primary output has no well-known suffix.
pub fn pick_primary_output<'a>(paths: &[&'a str]) -> &'a str {
    const SPLIT_SUFFIXES: &[&str] = &[
        "-man", "-doc", "-dev", "-lib", "-bin", "-debug", "-info",
        "-out", "-static", "-headers", "-pltcl", "-plperl", "-plpython3",
        "-jit",
    ];
    paths.iter()
        .find(|p| {
            let base = p.split('/').next_back().unwrap_or("");
            !SPLIT_SUFFIXES.iter().any(|s| base.ends_with(s))
        })
        .copied()
        .unwrap_or(paths[0])
}

// ── Attic push ────────────────────────────────────────────────────────────────

fn push_to_substituter(flake_dir: &Path, sub_url: &str, token: Option<&str>, paths: &[&str]) {
    let dest = match token {
        Some(t) => format!("{sub_url}?token={t}"),
        None    => sub_url.to_string(),
    };
    for path in paths {
        eprintln!("==> nix copy --to {sub_url} {path}");
        match run_nix(flake_dir, &["copy", "--to", &dest, path]) {
            Err(e) => {
                eprintln!("warning: nix copy failed (Attic unreachable?): {e}");
                eprintln!("         Re-run: nix copy --to {sub_url} {path}");
                return;
            }
            Ok(r) if !r.status.success() => {
                let stderr = String::from_utf8_lossy(&r.stderr);
                eprintln!("warning: nix copy failed:\n{stderr}");
                return;
            }
            Ok(_) => {}
        }
    }
}

// ── Nix invocation (platform-aware) ──────────────────────────────────────────

/// Run `nix <args>` in `flake_dir`.
/// On Windows, Nix lives in WSL2 — use `wsl bash -lc` so the login shell
/// sources the Nix profile. On Linux/macOS, `nix` is called directly.
pub fn run_nix(flake_dir: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    #[cfg(windows)]
    {
        let wsl_dir = windows_path_to_wsl(flake_dir);
        let quoted: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
        let cmd = format!("cd '{}' && nix {}", wsl_dir, quoted.join(" "));
        std::process::Command::new("wsl")
            .args(["bash", "-lc", &cmd])
            .output()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("nix")
            .args(args)
            .current_dir(flake_dir)
            .output()
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn windows_path_to_wsl(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        let drive = s.chars().next().unwrap().to_ascii_lowercase();
        let rest  = s[2..].replace('\\', "/");
        format!("/mnt/{drive}{rest}")
    } else {
        s.replace('\\', "/")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use justpkg_resolve::BuildProfile;

    // ── nixpkgs_overrides mapping ─────────────────────────────────────────────

    #[test]
    fn test_nixpkgs_overrides_postgres_icu_jit_disabled() {
        let p = BuildProfile { icu: Some(false), jit: Some(false), ..Default::default() };
        let m = nixpkgs_overrides("postgresql_16", &p);
        assert_eq!(m.get("icuSupport"), Some(&false));
        assert_eq!(m.get("jitSupport"), Some(&false));
        assert!(!m.contains_key("pythonSupport"), "python not in profile");
    }

    #[test]
    fn test_nixpkgs_overrides_postgres_full_profile() {
        let p = BuildProfile {
            icu: Some(false), jit: Some(false), python: Some(false),
            perl: Some(false), tcl: Some(false), pam: Some(false),
            gss: Some(false), systemd: Some(false), ..Default::default()
        };
        let m = nixpkgs_overrides("postgresql_16", &p);
        assert_eq!(m.len(), 8);
        assert_eq!(m.get("icuSupport"),    Some(&false));
        assert_eq!(m.get("jitSupport"),    Some(&false));
        assert_eq!(m.get("pythonSupport"), Some(&false));
        assert_eq!(m.get("perlSupport"),   Some(&false));
        assert_eq!(m.get("tclSupport"),    Some(&false));
        assert_eq!(m.get("pamSupport"),    Some(&false));
        assert_eq!(m.get("gssSupport"),    Some(&false));
        assert_eq!(m.get("systemdSupport"),Some(&false));
    }

    #[test]
    fn test_nixpkgs_overrides_redis_tls_systemd() {
        let p = BuildProfile { tls: Some(false), systemd: Some(false), ..Default::default() };
        let m = nixpkgs_overrides("redis", &p);
        assert_eq!(m.get("tlsSupport"),  Some(&false));
        assert_eq!(m.get("withSystemd"), Some(&false));
        assert!(!m.contains_key("icuSupport"), "icu not applicable to redis");
    }

    #[test]
    fn test_nixpkgs_overrides_unknown_package_returns_empty() {
        let p = BuildProfile { icu: Some(false), jit: Some(false), ..Default::default() };
        let m = nixpkgs_overrides("opensearch", &p);
        assert!(m.is_empty(), "unknown package must produce no overrides");
    }

    #[test]
    fn test_nixpkgs_overrides_none_values_not_emitted() {
        // Absent (None) = use nixpkgs default — no override key at all.
        // Explicitly-set flags (true or false) DO appear (see the opt-in test below).
        let p = BuildProfile { icu: None, jit: Some(false), ..Default::default() };
        let m = nixpkgs_overrides("postgresql_16", &p);
        assert!(!m.contains_key("icuSupport"),  "None should not produce an override");
        assert_eq!(m.get("jitSupport"), Some(&false));
    }

    #[test]
    fn test_nixpkgs_overrides_true_emits_explicit_enable() {
        // Per-feature opt-IN: Some(true) must emit `param = true`, not be dropped.
        // Guards the bidirectional mapping — a regression to opt-out-only (the old
        // `off!` macro that ignored Some(true)) would fail this.
        let p = BuildProfile { tls: Some(true), systemd: Some(false), ..Default::default() };
        let m = nixpkgs_overrides("redis", &p);
        assert_eq!(m.get("tlsSupport"),  Some(&true),  "tls = true must enable tlsSupport");
        assert_eq!(m.get("withSystemd"), Some(&false), "systemd = false must disable withSystemd");
    }

    #[test]
    fn test_nixpkgs_overrides_matches_all_postgresql_variants() {
        // postgresql_17, postgresql_15 etc. should all match the postgres branch.
        for name in &["postgresql_15", "postgresql_16", "postgresql_17"] {
            let p = BuildProfile { icu: Some(false), ..Default::default() };
            let m = nixpkgs_overrides(name, &p);
            assert!(m.contains_key("icuSupport"), "{name} must map icu → icuSupport");
        }
    }

    // ── pick_primary_output ───────────────────────────────────────────────────

    #[test]
    fn test_pick_primary_output_selects_unsuffixed_path() {
        let paths = vec![
            "/nix/store/abc-postgresql-16.10-man",
            "/nix/store/def-postgresql-16.10",
            "/nix/store/ghi-postgresql-16.10-doc",
        ];
        assert_eq!(pick_primary_output(&paths), "/nix/store/def-postgresql-16.10");
    }

    #[test]
    fn test_pick_primary_output_falls_back_to_first_when_all_suffixed() {
        let paths = vec![
            "/nix/store/abc-redis-7.2.7-bin",
            "/nix/store/def-redis-7.2.7-lib",
        ];
        assert_eq!(pick_primary_output(&paths), "/nix/store/abc-redis-7.2.7-bin");
    }

    #[test]
    fn test_pick_primary_output_single_path() {
        let paths = vec!["/nix/store/abc-redis-7.2.7"];
        assert_eq!(pick_primary_output(&paths), "/nix/store/abc-redis-7.2.7");
    }

    // ── Profile TOML parsing ──────────────────────────────────────────────────

    #[test]
    fn test_build_profile_parses_from_toml() {
        let toml = r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "postgresql_16"
attr = "postgresql_16"

[profile]
jit     = false
icu     = false
systemd = false
"#;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        let p = spec.profile.expect("profile must be present");
        assert_eq!(p.jit, Some(false));
        assert_eq!(p.icu, Some(false));
        assert_eq!(p.systemd, Some(false));
        assert_eq!(p.tls, None, "absent flag must be None");
    }

    #[test]
    fn test_build_profile_absent_defaults_to_none() {
        let toml = r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "redis"
attr = "redis"
"#;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        assert!(spec.profile.is_none());
    }

    #[test]
    fn test_build_profile_partial_flags() {
        let toml = r#"
nixpkgs_channel = "nixos-24.11"
[[package]]
name = "redis"
attr = "redis"
[profile]
tls = false
"#;
        let spec: justpkg_resolve::PackagesSpec = toml::from_str(toml).unwrap();
        let p = spec.profile.unwrap();
        assert_eq!(p.tls, Some(false));
        assert_eq!(p.jit, None);
        assert_eq!(p.icu, None);
    }
}
