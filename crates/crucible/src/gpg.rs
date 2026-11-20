//! Verify nodejs.org's `SHASUMS256.txt` with the detached signature published
//! beside it. The hashes in the pin data are only as trustworthy as that file,
//! and HTTPS alone is not a signature.
//!
//! The trusted keys are an allow-list vendored in `data/node/release-keys.asc`,
//! not whatever key happens to be on the signer's machine. rPGP does not
//! enforce revocation or subkey-binding semantics, so a signature is accepted
//! only from a key in that file, on the primary key or one of its signing
//! subkeys. Refreshing the file is a deliberate act when nodejs rotates keys.

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::types::VerifyingKey;

/// The active Node.js release keys, from nodejs/release-keys.
const RELEASE_KEYS: &str = include_str!("../../../data/node/release-keys.asc");

const BEGIN: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";
const END: &str = "-----END PGP PUBLIC KEY BLOCK-----";

/// rPGP's `from_string_many` stops at the first armor block, so parse each
/// key separately. The file's leading `#` comments are harmless: only the
/// armor blocks are handed to the parser.
fn keys() -> Result<Vec<SignedPublicKey>> {
    let mut keys = Vec::new();
    let mut rest = RELEASE_KEYS;
    while let Some(start) = rest.find(BEGIN) {
        let block = &rest[start..];
        let end = block
            .find(END)
            .map(|end| end + END.len())
            .unwrap_or(block.len());
        let (key, _) = SignedPublicKey::from_string(&block[..end]).context(
            "crucible: data/node/release-keys.asc holds no valid Node.js release keys; \
             refresh it from https://github.com/nodejs/release-keys",
        )?;
        keys.push(key);
        rest = &block[end..];
    }
    Ok(keys)
}

/// `DetachedSignature::verify` takes `impl VerifyingKey`, so the two kinds of
/// key have to go through the same generic function rather than a trait object.
fn verifies<K: VerifyingKey>(sig: &DetachedSignature, key: &K, data: &[u8]) -> bool {
    sig.verify(key, data).is_ok()
}

/// Check that `signature` is a valid detached signature of `shasums` by a
/// trusted release key. Called before `SHASUMS256.txt` is parsed or pinned, so
/// a bad signature never reaches `data/node/node.json`.
pub fn verify_shasums(version: &str, shasums: &[u8], signature: &[u8]) -> Result<()> {
    let sig = DetachedSignature::from_bytes(Cursor::new(signature))
        .with_context(|| format!("crucible: Node.js v{version} SHASUMS256.txt.sig is malformed"))?;

    let keys = keys()?;
    if keys.is_empty() {
        bail!(
            "crucible: data/node/release-keys.asc holds no valid Node.js release keys; \
             refresh it from https://github.com/nodejs/release-keys"
        );
    }

    for key in &keys {
        // The primary key often cannot sign; the signing subkey is what
        // nodejs.org actually uses. Try both rather than trusting key flags.
        if verifies(&sig, &key.primary_key, shasums)
            || key
                .public_subkeys
                .iter()
                .any(|sub| verifies(&sig, &sub.key, shasums))
        {
            return Ok(());
        }
    }

    bail!(
        "crucible: Node.js v{version} SHASUMS256.txt has no valid signature from a trusted \
         release key; refusing to pin. If the signer rotated, refresh \
         data/node/release-keys.asc from https://github.com/nodejs/release-keys"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHASUMS: &[u8] = include_bytes!("../tests/fixtures/shasums-v22.23.5.txt");
    const SIGNATURE: &[u8] = include_bytes!("../tests/fixtures/shasums-v22.23.5.txt.sig");

    #[test]
    fn accepts_a_real_release_signature() {
        verify_shasums("22.23.5", SHASUMS, SIGNATURE).unwrap();
    }

    #[test]
    fn rejects_modified_checksums() {
        let mut changed = SHASUMS.to_vec();
        changed[0] = if changed[0] == b'0' { b'1' } else { b'0' };
        let error = verify_shasums("22.23.5", &changed, SIGNATURE).unwrap_err();
        assert!(error.to_string().contains("no valid signature"));
    }

    #[test]
    fn rejects_a_malformed_signature() {
        let error = verify_shasums("22.23.5", SHASUMS, b"not a signature").unwrap_err();
        assert!(error.to_string().contains("malformed"));
    }

    #[test]
    fn the_vendored_keyring_parses() {
        assert_eq!(keys().unwrap().len(), 8);
    }
}
