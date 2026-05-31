use crate::api::error::ResolveError;
use crate::api::types::ResolvedManifest;
use std::path::Path;

/// Serialise `manifest` to `path` as pretty-printed JSON.
pub(crate) fn write_manifest(manifest: &ResolvedManifest, path: &Path) -> Result<(), ResolveError> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| ResolveError::ManifestWrite {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    std::fs::write(path, json.as_bytes()).map_err(|e| ResolveError::ManifestWrite {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(())
}

/// Format `SystemTime::now()` as a minimal RFC 3339 UTC timestamp.
pub(crate) fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let dy = if leap { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let leap = is_leap(year);
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for dm in &months {
        if days < *dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ManifestMeta;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    fn fixture_manifest() -> ResolvedManifest {
        let mut packages = BTreeMap::new();
        packages.insert(
            "postgresql_16".to_string(),
            "/nix/store/qsmz8pss6j0s2hj65bj5wgx5yrv2qfkz-postgresql-16.9".to_string(),
        );
        ResolvedManifest {
            packages,
            meta: ManifestMeta {
                nixpkgs_rev: "50ab793786d9de88ee30ec4e4c24fb4236fc2674".to_string(),
                channel: "nixos-24.11".to_string(),
                resolved_at: "2026-04-29T00:00:00Z".to_string(),
            },
        }
    }

    #[test]
    fn test_write_manifest_produces_valid_json() {
        let f = NamedTempFile::new().unwrap();
        let manifest = fixture_manifest();
        write_manifest(&manifest, f.path()).unwrap();
        let text = std::fs::read_to_string(f.path()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["packages"]["postgresql_16"].as_str().unwrap(),
            "/nix/store/qsmz8pss6j0s2hj65bj5wgx5yrv2qfkz-postgresql-16.9"
        );
    }

    #[test]
    fn test_write_manifest_round_trips_through_vminit_parse() {
        let f = NamedTempFile::new().unwrap();
        let manifest = fixture_manifest();
        write_manifest(&manifest, f.path()).unwrap();
        let text = std::fs::read_to_string(f.path()).unwrap();
        let root: serde_json::Value = serde_json::from_str(&text).unwrap();
        let packages = root["packages"].as_object().unwrap();
        assert!(packages.contains_key("postgresql_16"));
        let val = packages["postgresql_16"].as_str().unwrap();
        assert!(val.starts_with("/nix/store/"), "vminit expects a /nix/store/ path: {val}");
    }

    #[test]
    fn test_now_rfc3339_has_correct_format() {
        let ts = now_rfc3339();
        assert_eq!(ts.len(), 20, "timestamp must be 20 chars: {ts}");
        assert!(ts.ends_with('Z'), "must be UTC: {ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
