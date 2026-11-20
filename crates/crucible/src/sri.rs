//! SHA-256 hashes in the SRI form Nix expects.
//!
//! `static.rust-lang.org` and the fork's `SHA256SUMS` publish bare hex, and the
//! flake only ever reads SRI, so convert here instead of shelling out to `nix`.

use anyhow::{bail, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// The SRI hash of `bytes`, as `sha256-<base64>`.
pub fn sri(bytes: &[u8]) -> String {
    format!("sha256-{}", STANDARD.encode(Sha256::digest(bytes)))
}

/// Convert a 64-character hex sha256, such as the ones in `SHA256SUMS`.
pub fn from_hex(hex: &str) -> Result<String> {
    let raw = decode_hex(hex)?;
    if raw.len() != 32 {
        bail!("expected a 32-byte sha256, got {} bytes", raw.len());
    }
    Ok(format!("sha256-{}", STANDARD.encode(raw)))
}

fn decode_hex(hex: &str) -> Result<Vec<u8>> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        bail!("hex hash has an odd length: {hex}");
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(pair)?;
        out.push(u8::from_str_radix(pair, 16)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_published_cargo_hash() {
        let hex = "4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd";
        assert_eq!(
            from_hex(hex).unwrap(),
            "sha256-TJY0STv7PW/JJqXO4UJxr6LjMAineTkyFI4xxLTVQ90="
        );
    }

    #[test]
    fn hashes_bytes() {
        assert_eq!(
            sri(b""),
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn rejects_bad_hex() {
        assert!(from_hex("zz").is_err());
        assert!(from_hex("abc").is_err());
        assert!(
            from_hex("4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd00").is_err()
        );
    }
}
