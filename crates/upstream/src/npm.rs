//! Verify the npm registry's signature over a package's integrity, the way
//! pnpm's own installer (get.pnpm.io) and `npm audit signatures` do.
//!
//! The registry serves both the tarball and its sha512 integrity, so an
//! integrity read from it proves nothing on its own; the ECDSA signature does,
//! because the key that made it is pinned here, not fetched with the data. npm
//! publishes its keys at https://registry.npmjs.org/-/npm/v1/keys and rotates
//! them rarely: when it does, every pin fails here with an error naming the
//! new key, and `KEYS` below is updated by hand.
//!
//! A retired key still vouches for what it signed before it expired, as in
//! `npm audit signatures`: yarn 1.22.22 (2024) carries only the old key's
//! signature. So a signature by a key with an expiry is accepted only for a
//! release whose publish time is known and earlier.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature as EcdsaSignature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use serde::Deserialize;

/// One registry signing key (ecdsa-sha2-nistp256), as the keys endpoint
/// lists it.
struct Key {
    id: &'static str,
    /// Base64 SubjectPublicKeyInfo DER.
    der: &'static str,
    /// RFC 3339, in the endpoint's own `...T00:00:00.000Z` form, so it
    /// compares as a string against the packument's `time` entries.
    expires: Option<&'static str>,
}

const KEYS: [Key; 2] = [
    Key {
        id: "SHA256:DhQ8wR5APBvFHLF/+Tc+AYvPOdTpcIDqOhxsBHRwC7U",
        der: "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEY6Ya7W++7aUPzvMTrezH6Ycx3c+HOKYCcNGybJZSCJq/fd7Qa8uuAKtdIkUQtQiEKERhAmE5lMMJhP8OkDOa2g==",
        expires: None,
    },
    Key {
        id: "SHA256:jl3bwswu80PjjokCgh0o2w5c2U4LhQAE57gj9cz1kzA",
        der: "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE1Olb3zMAFFxXKHiIkQO5cJ3Yhl5i6UPp+IhuteBJbuHcA5UogKo0EWtlWwW6KSaKoTNEYL7JlCQiVnkhBktUgg==",
        expires: Some("2025-01-29T00:00:00.000Z"),
    },
];

/// One entry of a version's `dist.signatures`.
#[derive(Debug, Clone, Deserialize)]
pub struct Signature {
    pub keyid: String,
    pub sig: String,
}

/// Check that npm signed `<name>@<version>:<integrity>` with a pinned key.
/// `name` may be URL-encoded (`@pnpm%2Fexe`), as the registry URLs are.
/// `published` is the release's entry in the packument's `time`, needed only
/// when the signature is by a retired key.
pub fn verify(
    name: &str,
    version: &str,
    integrity: &str,
    signatures: &[Signature],
    published: Option<&str>,
) -> Result<()> {
    let name = name.replace("%2F", "/").replace("%2f", "/");
    let Some((key, signature)) = KEYS.iter().find_map(|key| {
        signatures
            .iter()
            .find(|s| s.keyid == key.id)
            .map(|s| (key, s))
    }) else {
        if signatures.is_empty() {
            bail!("{name}@{version} carries no npm registry signature, so it cannot be verified");
        }
        let ids: Vec<&str> = signatures.iter().map(|s| s.keyid.as_str()).collect();
        bail!(
            "{name}@{version} is signed with {}, which is not a pinned npm key; if npm rotated \
             its key (https://registry.npmjs.org/-/npm/v1/keys), update KEYS in \
             crates/upstream/src/npm.rs",
            ids.join(", ")
        );
    };
    if let Some(expires) = key.expires {
        match published {
            Some(published) if published < expires => {}
            Some(published) => bail!(
                "{name}@{version} was published {published}, after npm key {} expired ({expires})",
                key.id
            ),
            None => bail!(
                "{name}@{version} is signed with retired npm key {}, and its publish time is \
                 unknown, so the signature cannot be trusted",
                key.id
            ),
        }
    }
    let key = VerifyingKey::from_public_key_der(&STANDARD.decode(key.der)?)
        .context("a pinned npm registry key does not parse")?;
    let der = STANDARD
        .decode(&signature.sig)
        .with_context(|| format!("{name}@{version}'s signature is not base64"))?;
    let sig = EcdsaSignature::from_der(&der)
        .with_context(|| format!("{name}@{version}'s signature is not a DER ECDSA signature"))?;
    let message = format!("{name}@{version}:{integrity}");
    key.verify(message.as_bytes(), &sig).map_err(|_| {
        anyhow::anyhow!(
            "the npm registry signature for {name}@{version} does not match {integrity}"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // pnpm 11.29.0 as the registry serves it.
    const INTEGRITY: &str = "sha512-2RP5s3zV/meABS6Al3Ttq7e9z2mm5tm4/Ts4vAUvqKfYkbbCAaOPtQSgf/zi5ejCYu9U+49X22LyLTU6MLyKlg==";
    const SIG: &str = "MEYCIQCzLZw9XFDgpVrQ1WaKxkh7x0XkoSA2ipD/WIsexKzOEAIhAIHETzPKLjbep48f/J9O9tIC/hNhYzPMPxgBIgp3Zpfj";

    // yarn 1.22.22, signed only by the retired key.
    const YARN_INTEGRITY: &str = "sha512-prL3kGtyG7o9Z9Sv8IPfBNrWTDmXB4Qbes8A9rEzt6wkJV8mUvoirjU0Mp3GGAU06Y0XQyA3/2/RQFVuK7MTfg==";
    const YARN_SIG: &str = "MEYCIQD8IYJ9/vVOO8pfUq+cj3OdBClW7fPnUosQ4TvgyO1XJAIhAOi8CE+iN39Td3aR4dJz2zLO6isnr+h44dxVeoYjDyta";
    const YARN_PUBLISHED: &str = "2024-03-09T21:35:40.868Z";

    fn signed(key: usize, sig: &str) -> Vec<Signature> {
        vec![Signature {
            keyid: KEYS[key].id.to_string(),
            sig: sig.to_string(),
        }]
    }

    #[test]
    fn accepts_the_registry_signature() {
        verify("pnpm", "11.29.0", INTEGRITY, &signed(0, SIG), None).unwrap();
    }

    #[test]
    fn rejects_another_integrity_version_or_name() {
        let other = INTEGRITY.replace("2RP5", "2RP6");
        assert!(verify("pnpm", "11.29.0", &other, &signed(0, SIG), None).is_err());
        assert!(verify("pnpm", "11.29.1", INTEGRITY, &signed(0, SIG), None).is_err());
        assert!(verify("@pnpm%2Fexe", "11.29.0", INTEGRITY, &signed(0, SIG), None).is_err());
    }

    #[test]
    fn rejects_a_missing_or_unknown_key() {
        assert!(verify("pnpm", "11.29.0", INTEGRITY, &[], None).is_err());
        let unknown = vec![Signature {
            keyid: "SHA256:other".to_string(),
            sig: SIG.to_string(),
        }];
        let err = verify("pnpm", "11.29.0", INTEGRITY, &unknown, None).unwrap_err();
        assert!(err.to_string().contains("update KEYS"));
    }

    #[test]
    fn trusts_a_retired_key_only_for_what_it_signed_before_expiring() {
        let sigs = signed(1, YARN_SIG);
        verify(
            "yarn",
            "1.22.22",
            YARN_INTEGRITY,
            &sigs,
            Some(YARN_PUBLISHED),
        )
        .unwrap();
        assert!(verify("yarn", "1.22.22", YARN_INTEGRITY, &sigs, None).is_err());
        let late = "2026-01-01T00:00:00.000Z";
        assert!(verify("yarn", "1.22.22", YARN_INTEGRITY, &sigs, Some(late)).is_err());
    }
}
