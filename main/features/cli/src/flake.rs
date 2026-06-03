//! `pkg flake build` — build packages from a Nix flake, write manifest.json.
//!
//! When `[profile]` is set in `packages.toml`, justpkg generates a temporary
//! flake that applies the requested nixpkgs overrides (icuSupport=false, etc.)
//! directly — no named variant outputs needed in the workload flake.
//!
//! When no profile is set, the existing `attr` field selects a named flake
//! output (backward-compatible with `postgresql_16_slim` etc.).

use std::collections::{BTreeMap, BTreeSet};
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

    // When a [profile] is present, discover each package's overridable args from
    // nixpkgs (self-describing via `override.__functionArgs`), then:
    //   1. fail loudly on a flag no package can honor (silent-no-op guard), and
    //   2. generate a temp flake that applies the resolved overrides.
    let temp_flake = if let Some(profile) = spec.profile.as_ref() {
        let nixpkgs_url = read_nixpkgs_url(flake_dir)?;
        let accepted = discover_all_override_args(flake_dir, &nixpkgs_url, system, &spec.packages)?;

        let per_package: Vec<BTreeSet<String>> = accepted.values().cloned().collect();
        let unhonored = unhonored_flags(&per_package, profile);
        if !unhonored.is_empty() {
            return Err(HandlerError::InvalidRequest(format!(
                "[profile] flag(s) in {} are not supported by any package in the manifest \
                 and would be silently ignored: {}.\n\
                 Each package's overridable features are discovered from nixpkgs; none of \
                 these packages (on this nixpkgs channel) expose a matching `override` arg. \
                 Remove the flag(s), or switch to a channel/package that supports them. \
                 `check` is universal.",
                packages_toml.display(),
                unhonored.join(", "),
            )));
        }

        Some(generate_profile_flake(flake_dir, &spec.packages, system, profile, &nixpkgs_url, &accepted)?)
    } else {
        None
    };

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
    _workload_flake_dir: &Path,
    packages: &[justpkg_resolve::PackageEntry],
    system: &str,
    profile: &BuildProfile,
    nixpkgs_url: &str,
    accepted: &BTreeMap<String, BTreeSet<String>>,
) -> Result<TempFlake, HandlerError> {
    let dir = tempfile::tempdir()
        .map_err(|e| HandlerError::ExecutionFailed(format!("create temp flake dir: {e}")))?;

    // Inherit the workload flake's pinned nixpkgs. Each package gets an output
    // whose overrides are resolved against its discovered `override` args.
    let flake_content = generate_self_contained_flake(
        packages, system, profile, nixpkgs_url, accepted,
    )?;

    let flake_path = dir.path().join("flake.nix");
    std::fs::write(&flake_path, &flake_content)
        .map_err(|e| HandlerError::ExecutionFailed(format!("write temp flake.nix: {e}")))?;

    // Write a minimal flake.lock that Nix won't reject.
    // We'll let `nix flake update` generate it on first use.
    eprintln!("[profile] generated temporary flake at {}", dir.path().display());

    Ok(TempFlake(dir))
}

/// Generate a self-contained flake pinned to `nixpkgs_url`, applying each
/// package's profile overrides resolved against its discovered `override` args.
fn generate_self_contained_flake(
    packages: &[justpkg_resolve::PackageEntry],
    system: &str,
    profile: &BuildProfile,
    nixpkgs_url: &str,
    accepted: &BTreeMap<String, BTreeSet<String>>,
) -> Result<String, HandlerError> {
    let empty = BTreeSet::new();
    let mut pkg_outputs = String::new();
    for pkg in packages {
        let attr = pkg.name.replace('-', "_");
        let pkg_accepted = accepted.get(&pkg.name).unwrap_or(&empty);
        let overrides = resolve_overrides(pkg_accepted, profile);
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

/// Canonical profile feature → candidate nixpkgs `override` arg names, in
/// preference order. **Package-agnostic**: which candidate a given derivation
/// actually accepts is discovered at build time from its
/// `override.__functionArgs`, not hardcoded per package. nixpkgs uses different
/// names across derivations *and* channels (e.g. `withSystemd` on redis vs
/// `systemdSupport` on postgres; `icuSupport` exists on nixos-25.05 but not
/// 24.11), so resolution must be data-driven to stay correct.
pub fn feature_arg_aliases(feature: &str) -> &'static [&'static str] {
    match feature {
        "tls"     => &["tlsSupport", "opensslSupport", "sslSupport", "withTLS"],
        "systemd" => &["withSystemd", "systemdSupport"],
        "icu"     => &["icuSupport"],
        "jit"     => &["jitSupport"],
        "gss"     => &["gssSupport"],
        "pam"     => &["pamSupport"],
        "python"  => &["pythonSupport"],
        "perl"    => &["perlSupport"],
        "tcl"     => &["tclSupport"],
        _         => &[],
    }
}

/// The profile's explicitly-set feature flags as `(canonical name, value)`,
/// excluding `check` (handled separately via `overrideAttrs`).
fn set_feature_flags(profile: &BuildProfile) -> Vec<(&'static str, bool)> {
    [
        ("tls", profile.tls),       ("systemd", profile.systemd), ("icu", profile.icu),
        ("jit", profile.jit),       ("gss", profile.gss),         ("pam", profile.pam),
        ("python", profile.python), ("perl", profile.perl),       ("tcl", profile.tcl),
    ]
    .into_iter()
    .filter_map(|(name, val)| val.map(|v| (name, v)))
    .collect()
}

/// Resolve the profile's set flags to concrete nixpkgs `override` args for one
/// package, given the arg names that package accepts (its
/// `override.__functionArgs` keys, from [`discover_override_args`]). Each flag
/// maps to the first alias present in `accepted`; flags with no matching alias
/// are omitted — that derivation cannot toggle them (reported by
/// [`unhonored_flags`]).
pub fn resolve_overrides(
    accepted: &BTreeSet<String>,
    profile: &BuildProfile,
) -> BTreeMap<&'static str, bool> {
    let mut m = BTreeMap::new();
    for (feature, value) in set_feature_flags(profile) {
        if let Some(arg) = feature_arg_aliases(feature)
            .iter()
            .find(|a| accepted.contains(**a))
        {
            m.insert(*arg, value);
        }
    }
    m
}

/// Profile flags the user set explicitly but that **no** package in the manifest
/// can honor — i.e. no alias appears in any package's accepted `override` args.
/// These are pure no-ops; reporting them lets the build fail loudly instead of
/// silently producing an unexpectedly large image.
///
/// A flag accepted by *some* package (e.g. `tls` on redis but not a co-listed
/// tzdata) is correct and not reported. `check` is universal (applied via
/// `overrideAttrs`) and never appears here.
pub fn unhonored_flags(
    per_package_accepted: &[BTreeSet<String>],
    profile: &BuildProfile,
) -> Vec<&'static str> {
    set_feature_flags(profile)
        .into_iter()
        .filter(|(feature, _)| {
            let aliases = feature_arg_aliases(feature);
            !per_package_accepted
                .iter()
                .any(|acc| aliases.iter().any(|a| acc.contains(*a)))
        })
        .map(|(feature, _)| feature)
        .collect()
}

/// Read the pinned nixpkgs flake URL (`github:owner/repo/rev`) from the
/// workload's `flake.lock`. Prefers `nixpkgs-25_05`, falls back to `nixpkgs`.
fn read_nixpkgs_url(flake_dir: &Path) -> Result<String, HandlerError> {
    let lock_path = flake_dir.join("flake.lock");
    let lock_text = std::fs::read_to_string(&lock_path).map_err(|e| {
        HandlerError::InvalidRequest(format!("read flake.lock at {}: {e}", lock_path.display()))
    })?;
    let lock: serde_json::Value = serde_json::from_str(&lock_text)
        .map_err(|e| HandlerError::InvalidRequest(format!("parse flake.lock: {e}")))?;
    find_nixpkgs_url(&lock, &["nixpkgs-25_05", "nixpkgs"]).ok_or_else(|| {
        HandlerError::InvalidRequest("flake.lock has no nixpkgs or nixpkgs-25_05 input".to_string())
    })
}

/// Discover the `override` args a package accepts by evaluating its
/// `override.__functionArgs` against the pinned nixpkgs. `nix eval` is a pure
/// evaluation (no realisation), so this is cheap. The guard returns `[]` for
/// derivations without an overridable interface rather than erroring.
fn discover_override_args(
    flake_dir: &Path,
    nixpkgs_url: &str,
    system: &str,
    package_name: &str,
) -> Result<BTreeSet<String>, HandlerError> {
    let expr = format!(
        "let p = (builtins.getFlake \"{nixpkgs_url}\").legacyPackages.{system}.{package_name}; \
         in if (p ? override) && (p.override ? __functionArgs) \
            then builtins.attrNames p.override.__functionArgs else []"
    );
    let out = run_nix(flake_dir, &["eval", "--impure", "--json", "--expr", &expr]).map_err(|e| {
        HandlerError::ExecutionFailed(format!("nix eval __functionArgs for {package_name}: {e}"))
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(HandlerError::ExecutionFailed(format!(
            "discover override args for '{package_name}' failed:\n{stderr}"
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let names: Vec<String> = serde_json::from_str(stdout.trim()).map_err(|e| {
        HandlerError::ExecutionFailed(format!(
            "parse __functionArgs json for '{package_name}': {e} (got: {})",
            stdout.trim()
        ))
    })?;
    Ok(names.into_iter().collect())
}

/// Discover override args for every distinct package, returning
/// `name → accepted-args`. One pure `nix eval` per package (deduplicated).
fn discover_all_override_args(
    flake_dir: &Path,
    nixpkgs_url: &str,
    system: &str,
    packages: &[justpkg_resolve::PackageEntry],
) -> Result<BTreeMap<String, BTreeSet<String>>, HandlerError> {
    let mut map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for pkg in packages {
        if !map.contains_key(&pkg.name) {
            let args = discover_override_args(flake_dir, nixpkgs_url, system, &pkg.name)?;
            map.insert(pkg.name.clone(), args);
        }
    }
    Ok(map)
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

    // ── resolve_overrides (eval-driven) ───────────────────────────────────────

    /// Build an accepted-args set from a slice of nixpkgs `override` arg names.
    fn args(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_resolve_overrides_redis_args() {
        // redis exposes tlsSupport + withSystemd among its override args.
        let accepted = args(&["tlsSupport", "withSystemd", "jemalloc", "lua", "openssl"]);
        let p = BuildProfile { tls: Some(false), systemd: Some(false), ..Default::default() };
        let m = resolve_overrides(&accepted, &p);
        assert_eq!(m.get("tlsSupport"),  Some(&false));
        assert_eq!(m.get("withSystemd"), Some(&false));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn test_resolve_overrides_systemd_resolves_to_accepted_alias() {
        // postgres exposes systemdSupport (not withSystemd); the systemd flag must
        // resolve to whichever spelling the derivation actually accepts.
        let accepted = args(&["jitSupport", "systemdSupport", "gssSupport", "pythonSupport"]);
        let p = BuildProfile { systemd: Some(false), jit: Some(false), ..Default::default() };
        let m = resolve_overrides(&accepted, &p);
        assert_eq!(m.get("systemdSupport"), Some(&false), "systemd → systemdSupport here");
        assert_eq!(m.get("jitSupport"),     Some(&false));
        assert!(!m.contains_key("withSystemd"), "must not emit an arg the pkg lacks");
    }

    #[test]
    fn test_resolve_overrides_unsupported_flag_is_omitted_not_emitted_broken() {
        // nixos-24.11 postgres has NO icuSupport (only the bare `icu` dep). icu
        // must be omitted, NOT emitted as a `icuSupport = false` that would error
        // the nix build — this is the exact bug the hardcoded mapping produced.
        let accepted = args(&["jitSupport", "icu", "linux-pam", "openssl"]);
        let p = BuildProfile { icu: Some(false), jit: Some(false), ..Default::default() };
        let m = resolve_overrides(&accepted, &p);
        assert!(!m.contains_key("icuSupport"), "icu unsupported on this channel → omitted");
        assert_eq!(m.get("jitSupport"), Some(&false));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn test_resolve_overrides_true_emits_explicit_enable() {
        // Per-feature opt-IN: Some(true) emits `arg = true`, not dropped.
        let accepted = args(&["tlsSupport", "withSystemd"]);
        let p = BuildProfile { tls: Some(true), systemd: Some(false), ..Default::default() };
        let m = resolve_overrides(&accepted, &p);
        assert_eq!(m.get("tlsSupport"),  Some(&true));
        assert_eq!(m.get("withSystemd"), Some(&false));
    }

    #[test]
    fn test_resolve_overrides_none_flags_not_emitted() {
        // Absent (None) flag → no override even if the arg is accepted.
        let accepted = args(&["tlsSupport", "withSystemd"]);
        let p = BuildProfile { tls: Some(false), ..Default::default() }; // systemd: None
        let m = resolve_overrides(&accepted, &p);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("tlsSupport"), Some(&false));
    }

    #[test]
    fn test_feature_arg_aliases_systemd_lists_both_spellings() {
        // Guards the canonical alias table — systemd must cover both nixpkgs names.
        let a = feature_arg_aliases("systemd");
        assert!(a.contains(&"withSystemd") && a.contains(&"systemdSupport"));
        assert!(feature_arg_aliases("does-not-exist").is_empty());
    }

    // ── unhonored_flags ───────────────────────────────────────────────────────

    #[test]
    fn test_unhonored_flags_clean_when_each_flag_accepted_by_some_package() {
        // redis accepts tls+systemd; tzdata/bash accept neither — fine, honored
        // by redis. check is universal and never reported.
        let per_pkg = vec![
            args(&["tlsSupport", "withSystemd"]), // redis
            args(&["coreutils"]),                 // tzdata-ish
            args(&["pkgsStatic"]),                // bash-ish
        ];
        let p = BuildProfile {
            tls: Some(false), systemd: Some(false), check: Some(false), ..Default::default()
        };
        assert!(unhonored_flags(&per_pkg, &p).is_empty());
    }

    #[test]
    fn test_unhonored_flags_reports_flag_no_package_accepts() {
        // icu accepted by no package here; tls accepted by redis ⇒ only icu flagged.
        let per_pkg = vec![args(&["tlsSupport", "withSystemd"]), args(&["pkgsStatic"])];
        let p = BuildProfile { icu: Some(false), tls: Some(false), ..Default::default() };
        assert_eq!(unhonored_flags(&per_pkg, &p), vec!["icu"]);
    }

    #[test]
    fn test_unhonored_flags_check_never_reported() {
        let per_pkg = vec![args(&["coreutils"]), args(&["pkgsStatic"])];
        let p = BuildProfile { check: Some(false), ..Default::default() };
        assert!(unhonored_flags(&per_pkg, &p).is_empty());
    }

    #[test]
    fn test_unhonored_flags_all_unmapped_reports_every_set_flag() {
        // opensearch-ish: only package deps, no feature toggles ⇒ every set flag
        // (except check) is a no-op and must be reported.
        let per_pkg = vec![args(&["coreutils", "jre_headless", "stdenv"])];
        let p = BuildProfile { tls: Some(false), systemd: Some(false), ..Default::default() };
        let mut u = unhonored_flags(&per_pkg, &p);
        u.sort_unstable();
        assert_eq!(u, vec!["systemd", "tls"]);
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
