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
