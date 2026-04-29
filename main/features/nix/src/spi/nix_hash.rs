//! Nix uses three hash encodings in different contexts. Getting them
//! wrong causes silent mismatches. All conversions go through here.
//!
//! - **Base-32** (Nix-alphabet): store path hashes, e.g. `abc123xyz...`
//! - **Base-64 SRI**: `flake.lock` narHash, e.g. `sha256-AAAA...=`
//! - **Hex**: some narinfo fields, CAS keys

use crate::api::error::NixFetchError;

const NIX_BASE32_CHARS: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Decode a Nix base-32 encoded hash to raw bytes.
pub fn nix_base32_decode(s: &str) -> Result<Vec<u8>, NixFetchError> {
    let s = s.as_bytes();
    let len = s.len();
    let out_len = len * 5 / 8;
    let mut out = vec![0u8; out_len];

    for (i, &c) in s.iter().rev().enumerate() {
        let digit = NIX_BASE32_CHARS
            .iter()
            .position(|&x| x == c)
            .ok_or_else(|| {
                NixFetchError::InvalidNixHash(format!("invalid base-32 char '{}'", c as char))
            })? as u64;

        let b = i * 5;
        let byte_idx = b / 8;
        let bit_off = b % 8;

        if byte_idx < out_len {
            out[byte_idx] |= (digit << bit_off) as u8;
        }
        if bit_off > 3 && byte_idx + 1 < out_len {
            out[byte_idx + 1] |= (digit >> (8 - bit_off)) as u8;
        }
    }

    Ok(out)
}

/// Convert a Nix base-32 hash to lowercase hex (for CAS key lookup).
pub fn nix_base32_to_hex(s: &str) -> Result<String, NixFetchError> {
    let bytes = nix_base32_decode(s)?;
    Ok(hex::encode(bytes))
}

/// Decode a base-64 SRI hash (`sha256-<base64>=`) to hex.
///
/// Used to convert `flake.lock` `narHash` values to CAS keys.
pub fn sri_to_hex(sri: &str) -> Result<String, NixFetchError> {
    let b64 = sri
        .strip_prefix("sha256-")
        .ok_or_else(|| {
            NixFetchError::InvalidNixHash(format!("SRI hash must start with 'sha256-', got: {sri}"))
        })?
        .trim_end_matches('=');

    // standard base64 alphabet
    let bytes = base64_decode(b64).ok_or_else(|| {
        NixFetchError::InvalidNixHash(format!("invalid base-64 in SRI hash: {sri}"))
    })?;

    Ok(hex::encode(bytes))
}

/// Encode raw bytes as a Nix base-32 string (custom Nix alphabet).
///
/// This is the inverse of `nix_base32_decode`. Ported from Nix C++ `printHash32`.
pub fn nix_base32_encode(bytes: &[u8]) -> String {
    let out_len = (bytes.len() * 8).div_ceil(5);
    let mut out = Vec::with_capacity(out_len);
    for n in (0..out_len).rev() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;
        let c0 = bytes[i] as u32;
        let c1 = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        let c = ((c0 >> j) | (c1 << (8 - j))) & 0x1f;
        out.push(NIX_BASE32_CHARS[c as usize]);
    }
    // SAFETY: NIX_BASE32_CHARS is ASCII-only.
    String::from_utf8(out).expect("nix_base32_encode produced non-UTF-8")
}

/// Derive the Nix store path hash component from a `flake.lock` SRI narHash.
///
/// Nix derives a fixed-output derivation store path for recursive ("source") mode via:
///   fingerprint = "source:sha256:<hex(narHash_bytes)>:/nix/store:source"
///   store_hash  = nix_base32( sha256(fingerprint)[0..20] )
///
/// This is the 32-character prefix in `<store_hash>.narinfo` on cache.nixos.org.
pub fn nar_hash_to_store_path_hash(sri: &str) -> Result<String, NixFetchError> {
    let hash_hex = sri_to_hex(sri)?;
    let fingerprint = format!("source:sha256:{}:/nix/store:source", hash_hex);
    let full_hash = {
        use sha2::Digest;
        sha2::Sha256::digest(fingerprint.as_bytes())
    };
    Ok(nix_base32_encode(&full_hash[..20]))
}

#[cfg(test)]
mod tests_encode {
    use super::*;

    #[test]
    fn test_nix_base32_encode_all_zeros_produces_all_zero_chars() {
        let result = nix_base32_encode(&[0u8; 20]);
        assert_eq!(result.len(), 32, "20 bytes must produce 32 Nix base-32 chars");
        assert!(
            result.chars().all(|c| c == '0'),
            "all-zero bytes must encode to all '0' chars, got: {result}"
        );
    }

    #[test]
    fn test_nix_base32_encode_all_ones_produces_all_z_chars() {
        let result = nix_base32_encode(&[0xffu8; 20]);
        assert_eq!(result.len(), 32, "20 bytes must produce 32 Nix base-32 chars");
        assert!(
            result.chars().all(|c| c == 'z'),
            "all-0xff bytes must encode to all 'z' chars, got: {result}"
        );
    }

    #[test]
    fn test_nix_base32_encode_decode_round_trip() {
        let original = "0123456789abcdfghijklmnpqrsvwxyz";
        let decoded = nix_base32_decode(original).expect("decode failed");
        let re_encoded = nix_base32_encode(&decoded);
        assert_eq!(
            re_encoded, original,
            "encode(decode(s)) must equal s for Nix base-32"
        );
    }

    #[test]
    fn test_nar_hash_to_store_path_hash_output_is_32_valid_nix_base32_chars() {
        // sha256 of empty string in SRI format
        let sri = "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
        let result = nar_hash_to_store_path_hash(sri)
            .expect("must succeed for valid SRI");
        assert_eq!(result.len(), 32, "store path hash must be 32 chars");
        let valid_chars: &str = "0123456789abcdfghijklmnpqrsvwxyz";
        assert!(
            result.chars().all(|c| valid_chars.contains(c)),
            "store path hash must use only Nix base-32 alphabet: {result}"
        );
    }

    #[test]
    fn test_nar_hash_to_store_path_hash_rejects_invalid_sri() {
        let err = nar_hash_to_store_path_hash("md5-not-valid")
            .expect_err("must fail for non-sha256 SRI");
        assert!(
            matches!(err, NixFetchError::InvalidNixHash(_)),
            "must return InvalidNixHash error, got: {err:?}"
        );
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let bytes: Vec<u8> = s.bytes().collect();
    let mut i = 0;
    while i + 3 < bytes.len() {
        let a = alphabet.iter().position(|&x| x == bytes[i])? as u32;
        let b = alphabet.iter().position(|&x| x == bytes[i + 1])? as u32;
        let c = alphabet.iter().position(|&x| x == bytes[i + 2])? as u32;
        let d = alphabet.iter().position(|&x| x == bytes[i + 3])? as u32;
        let n = (a << 18) | (b << 12) | (c << 6) | d;
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    // handle remaining bytes
    match bytes.len() - i {
        2 => {
            let a = alphabet.iter().position(|&x| x == bytes[i])? as u32;
            let b = alphabet.iter().position(|&x| x == bytes[i + 1])? as u32;
            out.push(((a << 2) | (b >> 4)) as u8);
        }
        3 => {
            let a = alphabet.iter().position(|&x| x == bytes[i])? as u32;
            let b = alphabet.iter().position(|&x| x == bytes[i + 1])? as u32;
            let c = alphabet.iter().position(|&x| x == bytes[i + 2])? as u32;
            out.push(((a << 2) | (b >> 4)) as u8);
            out.push(((b << 4) | (c >> 2)) as u8);
        }
        _ => {}
    }
    Some(out)
}
