use base64::{engine::general_purpose::STANDARD, Engine as _};
use crate::api::error::ResolveError;

/// Convert a Nix base-32 narHash (as found in `.narinfo` `NarHash: sha256:<b32>`)
/// to the SRI format expected by `vminit`: `sha256-<standard-base64>=`.
///
/// Pipeline:
///   nix-base32 → hex (via `justpkg_nix::nix_base32_to_hex`)
///             → raw bytes (hex::decode)
///             → standard base64 (STANDARD alphabet, `=` padding)
///             → `sha256-<b64>`
pub fn nix_base32_to_sri(nix_b32: &str) -> Result<String, ResolveError> {
    let hex_str = justpkg_nix::nix_base32_to_hex(nix_b32)
        .map_err(|e| ResolveError::HashConvert(e.to_string()))?;
    let raw = hex::decode(&hex_str)
        .map_err(|e| ResolveError::HashConvert(format!("hex decode: {e}")))?;
    Ok(format!("sha256-{}", STANDARD.encode(&raw)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All-zero 32-byte hash in Nix base-32 is 52 '0' chars.
    /// nix_base32_encode([0u8; 32]) = "0" * 52
    /// base64([0u8; 32]) = "A" * 43 + "="
    /// SRI = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    #[test]
    fn test_nix_base32_to_sri_all_zero_bytes() {
        let nix_b32 = "0000000000000000000000000000000000000000000000000000"; // 52 zeros
        let sri = nix_base32_to_sri(nix_b32).expect("must succeed for all-zero hash");
        assert_eq!(sri, "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
    }

    #[test]
    fn test_nix_base32_to_sri_output_has_correct_structure() {
        let nix_b32 = "0000000000000000000000000000000000000000000000000000";
        let sri = nix_base32_to_sri(nix_b32).unwrap();
        assert!(sri.starts_with("sha256-"), "SRI must start with 'sha256-'");
        assert!(sri.ends_with('='), "SRI must end with base64 padding '='");
        // "sha256-" (7) + 44 base64 chars for 32 bytes = 51 total
        assert_eq!(sri.len(), 51, "SRI for SHA-256 must be 51 chars: {sri}");
    }

    #[test]
    fn test_nix_base32_to_sri_rejects_invalid_char() {
        // 'u' is not in the Nix base-32 alphabet (0-9abcdfghijklmnpqrsvwxyz)
        let err = nix_base32_to_sri("uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuu");
        assert!(err.is_err(), "must fail for 'u' which is not in Nix base-32 alphabet");
    }

    #[test]
    fn test_nix_base32_to_sri_round_trips_via_sri_to_hex() {
        // All-zero hash: known good input, verifiable without network.
        let nix_b32 = "0000000000000000000000000000000000000000000000000000";
        let sri = nix_base32_to_sri(nix_b32).unwrap();
        let hex_via_sri = justpkg_nix::sri_to_hex(&sri).unwrap();
        let hex_direct = justpkg_nix::nix_base32_to_hex(nix_b32).unwrap();
        assert_eq!(hex_via_sri, hex_direct, "sri_to_hex must be inverse of nix_base32_to_sri");
    }
}
