/// Parse a JSON manifest into a [`PackageManifest`].
///
/// Expected format:
/// ```json
/// {"packages": {"curl": "sha256-xxx", "git": "sha256-yyy"}}
/// ```
///
/// Returns [`VminitInstallError::ManifestParse`] for any malformed input.
use crate::api::error::VminitInstallError;
use crate::api::manifest::PackageManifest;

pub fn parse_manifest(text: &str) -> Result<PackageManifest, VminitInstallError> {
    let root: serde_json::Value =
        serde_json::from_str(text).map_err(|e| VminitInstallError::ManifestParse(e.to_string()))?;

    let packages = root
        .get("packages")
        .ok_or_else(|| {
            VminitInstallError::ManifestParse("missing required key \"packages\"".to_string())
        })?
        .as_object()
        .ok_or_else(|| {
            VminitInstallError::ManifestParse("\"packages\" must be a JSON object".to_string())
        })?;

    let mut entries = std::collections::HashMap::new();
    for (k, v) in packages {
        let hash = v.as_str().ok_or_else(|| {
            VminitInstallError::ManifestParse(format!("value for package {k:?} must be a string"))
        })?;
        entries.insert(k.clone(), hash.to_string());
    }

    Ok(PackageManifest { entries })
}
