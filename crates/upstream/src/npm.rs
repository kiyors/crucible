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

use anyhow::{anyhow, bail, Context, Result};
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
    /// RFC 3339 expiry from the keys endpoint. Parsed before comparing so a
    /// packument `time` in another offset still orders correctly.
    expires: Option<&'static str>,
}

/// The registry metadata needed to check one tarball, shared by every npm
/// package updater: yarn's releases and tsrs's platform packages.
#[derive(Debug, Deserialize)]
pub struct Dist {
    pub integrity: String,
    pub tarball: String,
    #[serde(default)]
    pub signatures: Vec<Signature>,
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
    let mut saw_pinned = false;
    let mut saw_usable = false;
    let mut retired_reason = None;
    let mut signature_error = None;

    // A registry version can carry several signatures. Try every pinned key
    // against every signature: the first signature from a pinned key is not
    // necessarily a valid one (a key may have been used for another digest),
    // and a retired key may be listed beside a current one.
    for key in &KEYS {
        for signature in signatures.iter().filter(|s| s.keyid == key.id) {
            saw_pinned = true;
            if let Some(expires) = key.expires {
                let usable = match published {
                    Some(published) => match rfc3339_before(published, expires) {
                        Some(true) => true,
                        Some(false) => {
                            retired_reason = Some(format!(
                                "{name}@{version} was published {published}, after npm key {} \
                                 expired ({expires})",
                                key.id
                            ));
                            false
                        }
                        None => {
                            retired_reason = Some(format!(
                                "{name}@{version} has an unrecognised publish time {published}; \
                                 check the registry's time entry"
                            ));
                            false
                        }
                    },
                    None => {
                        retired_reason = Some(format!(
                            "{name}@{version} is signed with retired npm key {}, and its publish \
                             time is unknown, so the signature cannot be trusted",
                            key.id
                        ));
                        false
                    }
                };
                if !usable {
                    continue;
                }
            }
            saw_usable = true;
            match signature_result(key, signature, &name, version, integrity) {
                Ok(()) => return Ok(()),
                Err(err) if signature_error.is_none() => signature_error = Some(err),
                Err(_) => {}
            }
        }
    }

    if !saw_pinned {
        if signatures.is_empty() {
            bail!("crucible: {name}@{version} carries no npm registry signature, so it cannot be verified");
        }
        let ids: Vec<&str> = signatures.iter().map(|s| s.keyid.as_str()).collect();
        bail!(
            "crucible: {name}@{version} is signed with {}, which is not a pinned npm key; if npm \
             rotated its key (https://registry.npmjs.org/-/npm/v1/keys), update KEYS in \
             crates/upstream/src/npm.rs",
            ids.join(", ")
        );
    }
    if !saw_usable {
        if let Some(reason) = retired_reason {
            bail!("crucible: {reason}");
        }
    }
    match signature_error {
        Some(err) => Err(err).context(
            "crucible: no pinned npm key could verify this release; check the registry integrity \
             and retry `crucible update`",
        ),
        None => bail!("crucible: no pinned npm key signed {name}@{version}"),
    }
}

/// Verify one signature by one pinned key, keeping the failure reason so the
/// caller can report the most useful one after trying all of them.
fn signature_result(
    key: &Key,
    signature: &Signature,
    name: &str,
    version: &str,
    integrity: &str,
) -> Result<()> {
    let der_key = STANDARD
        .decode(key.der)
        .context("a pinned npm registry key does not parse")?;
    let verifying = VerifyingKey::from_public_key_der(&der_key)
        .context("a pinned npm registry key does not parse")?;
    let der = STANDARD
        .decode(&signature.sig)
        .with_context(|| format!("{name}@{version}'s signature is not base64"))?;
    let sig = EcdsaSignature::from_der(&der)
        .with_context(|| format!("{name}@{version}'s signature is not a DER ECDSA signature"))?;
    let message = format!("{name}@{version}:{integrity}");
    verifying.verify(message.as_bytes(), &sig).map_err(|_| {
        anyhow!("the npm registry signature for {name}@{version} does not match {integrity}")
    })
}

/// The tarball's integrity, once npm's signature over it checks out. npm
/// publishes SRI sha512 here; say so explicitly rather than waiting for a
/// fetchurl with an unexpected algorithm to fail in the Nix build.
pub fn checked_integrity<'a>(
    name: &str,
    version: &str,
    dist: &'a Dist,
    published: Option<&str>,
) -> Result<&'a str> {
    if !dist.integrity.starts_with("sha512-") {
        bail!(
            "crucible: {name} {version} has integrity {}, expected a sha512- SRI hash",
            dist.integrity
        );
    }
    verify(name, version, &dist.integrity, &dist.signatures, published)?;
    Ok(&dist.integrity)
}

/// Whether `a` is earlier than `b`, comparing parsed RFC 3339 timestamps. A
/// string compare would order `...T9Z` after `...T10Z` and would ignore an
/// offset entirely.
fn rfc3339_before(a: &str, b: &str) -> Option<bool> {
    Some(parse_rfc3339(a)? < parse_rfc3339(b)?)
}

/// `YYYY-MM-DDTHH:MM:SS(.fraction)?(Z|±HH:MM)` -> (unix seconds, nanoseconds).
/// npm writes three fractional digits; packument times use the same shape but
/// may carry an offset instead of `Z`.
fn parse_rfc3339(value: &str) -> Option<(i64, u32)> {
    let (date, rest) = value.split_once('T')?;
    let (clock, offset) = split_zone(rest)?;
    let (clock, nanos) = match clock.split_once('.') {
        Some((clock, fraction)) => {
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            let nanos = fraction.parse::<u32>().ok()? * 10u32.pow(9 - fraction.len() as u32);
            (clock, nanos)
        }
        None => (clock, 0),
    };
    let mut time = clock.split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next()?.parse().ok()?;
    if time.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let local = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some((local - offset, nanos))
}

/// The UTC designator and how far local time is ahead of UTC, in seconds.
fn split_zone(value: &str) -> Option<(&str, i64)> {
    if let Some(clock) = value.strip_suffix('Z') {
        return Some((clock, 0));
    }
    let index = value.rfind(['+', '-'])?;
    let (clock, zone) = value.split_at(index);
    let sign = if zone.starts_with('+') { 1 } else { -1 };
    let mut parts = zone[1..].split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || hour > 23 || minute > 59 {
        return None;
    }
    Some((clock, sign * (hour * 3_600 + minute * 60)))
}

/// Hinnant's civil-date arithmetic, kept here so the comparison needs no date
/// crate. `upstream` cannot depend on the copy in `pins::update::rust`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let era_day = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + era_day - 719_468
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

    #[test]
    fn tries_every_pinned_key_instead_of_the_first_signature() {
        // The current key has a signature here, but it signs pnpm's digest,
        // not yarn's; the retired key's real signature must still be tried.
        let mut signatures = signed(0, SIG);
        signatures.extend(signed(1, YARN_SIG));
        verify(
            "yarn",
            "1.22.22",
            YARN_INTEGRITY,
            &signatures,
            Some(YARN_PUBLISHED),
        )
        .unwrap();
    }

    #[test]
    fn checked_integrity_accepts_a_real_signature() {
        let dist = Dist {
            integrity: INTEGRITY.to_string(),
            tarball: "pnpm-11.29.0.tgz".to_string(),
            signatures: signed(0, SIG),
        };
        assert_eq!(
            checked_integrity("pnpm", "11.29.0", &dist, None).unwrap(),
            INTEGRITY
        );
    }

    #[test]
    fn checked_integrity_rejects_a_non_sha512_hash() {
        let dist = Dist {
            integrity: "sha1-deadbeef".to_string(),
            tarball: "x.tgz".to_string(),
            signatures: signed(0, SIG),
        };
        assert!(checked_integrity("pnpm", "1.0.0", &dist, None).is_err());
    }

    #[test]
    fn compares_rfc3339_timestamps_by_value_not_text() {
        let yarn = "2024-03-09T21:35:40.868Z";
        let expiry = "2025-01-29T00:00:00.000Z";
        assert_eq!(rfc3339_before(yarn, expiry), Some(true));
        assert_eq!(rfc3339_before(expiry, yarn), Some(false));
        // Equal instants, one without fractions and one in another offset.
        assert_eq!(
            rfc3339_before("2026-01-01T00:00:00Z", "2026-01-01T00:00:00.000Z"),
            Some(false)
        );
        assert_eq!(
            rfc3339_before("2026-01-01T05:30:00+05:30", "2026-01-01T00:00:00Z"),
            Some(false)
        );
        assert_eq!(
            rfc3339_before("2026-01-01T05:30:01+05:30", "2026-01-01T00:00:00Z"),
            Some(false)
        );
        assert_eq!(rfc3339_before("nonsense", expiry), None);
        assert_eq!(
            rfc3339_before("2026-13-01T00:00:00Z", "2026-01-01T00:00:00Z"),
            None
        );
        assert_eq!(rfc3339_before("2026-01-01T00:00:00", expiry), None);
    }
}
