//! Verify a pnpm GitHub release asset against its GitHub artifact attestation.
//!
//! pnpm publishes self-contained release archives on GitHub, but unlike the
//! npm registry they carry no signature over their checksum. GitHub's
//! artifact attestations fill that gap: the release workflow asks GitHub to
//! attest a statement listing every asset's sha256, the repository and the
//! tag. The API serves it keyed by the digest of the asset being pinned, and
//! it is GitHub's own authenticated endpoint, so an attacker who swaps an
//! asset cannot make it return an attestation for the new bytes. The
//! attestation's `repository` and `tag` are checked as well, so an asset from
//! another workflow cannot be used for this release.
//!
//! We do not verify the Sigstore bundle's signature locally: the lookup is
//! already authenticated by GitHub's TLS, which is the same trust tier as
//! bun's `SHASUMS256.txt` served from its release. That keeps the updater's
//! dependencies small; a GitHub compromise breaks both models alike.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

use crate::http::Http;

const API: &str = "https://api.github.com/repos/pnpm/pnpm/attestations/sha256:";
const RELEASE: &str = "https://in-toto.io/attestation/release/v0.2";
const REPOSITORY: &str = "pnpm/pnpm";

#[derive(Debug, Deserialize)]
struct Response {
    attestations: Vec<Attestation>,
}

#[derive(Debug, Deserialize)]
struct Attestation {
    bundle: Bundle,
}

#[derive(Debug, Deserialize)]
struct Bundle {
    #[serde(rename = "dsseEnvelope")]
    dsse_envelope: DsseEnvelope,
}

#[derive(Debug, Deserialize)]
struct DsseEnvelope {
    payload: String,
}

#[derive(Debug, Deserialize)]
struct Statement {
    subject: Vec<Subject>,
    #[serde(rename = "predicateType")]
    predicate_type: String,
    predicate: Predicate,
}

#[derive(Debug, Deserialize)]
struct Subject {
    /// The first subject names the release itself (`uri`), not an asset.
    #[serde(default)]
    name: Option<String>,
    digest: Digest,
}

#[derive(Debug, Deserialize)]
struct Digest {
    #[serde(rename = "sha256")]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Predicate {
    repository: String,
    tag: String,
}

/// Check that GitHub attests `digest_hex` as `asset` of pnpm's `tag` release.
/// `digest_hex` is the asset's sha256, as the release API reports it.
pub fn verify(http: &Http, digest_hex: &str, asset: &str, tag: &str) -> Result<()> {
    let url = format!("{API}{digest_hex}");
    let response: Response = serde_json::from_slice(&http.get(&url)?)
        .with_context(|| format!("failed to parse {url}"))?;
    for attestation in &response.attestations {
        let bytes = STANDARD
            .decode(&attestation.bundle.dsse_envelope.payload)
            .context("an attestation's DSSE payload is not base64")?;
        let statement: Statement = serde_json::from_slice(&bytes)
            .context("an attestation's DSSE payload is not an in-toto statement")?;
        if statement.predicate_type != RELEASE
            || statement.predicate.repository != REPOSITORY
            || statement.predicate.tag != tag
        {
            continue;
        }
        let listed = statement.subject.iter().any(|subject| {
            subject.name.as_deref() == Some(asset)
                && subject.digest.sha256.as_deref() == Some(digest_hex)
        });
        if listed {
            return Ok(());
        }
    }
    bail!(
        "GitHub has no release attestation for pnpm {tag}: {asset} at sha256:{digest_hex} \
         (see https://github.com/pnpm/pnpm/attestations)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The shape GitHub serves for pnpm 12.23.0's pnpm-linux-x64.tar.gz.
    const STATEMENT: &str = r#"{
      "_type": "https://in-toto.io/Statement/v1",
      "subject": [
        { "name": "pnpm-linux-x64.tar.gz",
          "digest": { "sha256": "b515e29f2ac1adc0fbe63173414a6682827baff24f17e7ad3476821bdef236ac" } }
      ],
      "predicateType": "https://in-toto.io/attestation/release/v0.2",
      "predicate": { "repository": "pnpm/pnpm", "tag": "v12.23.0" }
    }"#;

    fn statement(json: &str) -> Statement {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parses_a_release_attestation() {
        let s = statement(STATEMENT);
        assert_eq!(s.predicate.repository, "pnpm/pnpm");
        assert_eq!(s.predicate.tag, "v12.23.0");
        let subject = &s.subject[0];
        assert_eq!(subject.name.as_deref(), Some("pnpm-linux-x64.tar.gz"));
        assert_eq!(
            subject.digest.sha256.as_deref(),
            Some("b515e29f2ac1adc0fbe63173414a6682827baff24f17e7ad3476821bdef236ac")
        );
    }
}
