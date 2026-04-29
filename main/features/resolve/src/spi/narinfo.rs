/// Extract the 32-character Nix store hash from a full store path.
///
/// `/nix/store/<32-char-hash>-<name>-<version>` → `<32-char-hash>`
pub fn store_path_to_hash(store_path: &str) -> Option<&str> {
    let base = store_path.strip_prefix("/nix/store/")?;
    let hash = base.splitn(2, '-').next()?;
    if hash.len() == 32 {
        Some(hash)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_path_to_hash_extracts_32_char_prefix() {
        let path = "/nix/store/abc1234567890abcdefghijklmnopqrs-postgresql-16.6";
        assert_eq!(store_path_to_hash(path), Some("abc1234567890abcdefghijklmnopqrs"));
    }

    #[test]
    fn test_store_path_to_hash_rejects_short_hash() {
        let path = "/nix/store/tooshort-postgresql-16.6";
        assert_eq!(store_path_to_hash(path), None);
    }

    #[test]
    fn test_store_path_to_hash_rejects_non_store_path() {
        assert_eq!(store_path_to_hash("/usr/bin/postgres"), None);
    }
}
