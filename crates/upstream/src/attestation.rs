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

use crate::http::{Http, HttpError};

const API: &str = "https://api.github.com/repos/pnpm/pnpm/attestations/sha256:";
const RELEASE: &str = "https://in-toto.io/attestation/release/v0.2";
const REPOSITORY: &str = "pnpm/pnpm";

/// The single network operation `verify` needs. `Http` implements it for
/// production; tests supply a canned response so every branch can be checked
/// without a network or a local HTTP server.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
}

impl Fetch for Http {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        Http::get(self, url)
    }
}

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
pub fn verify<F: Fetch>(http: &F, digest_hex: &str, asset: &str, tag: &str) -> Result<()> {
    let url = format!("{API}{digest_hex}");
    let body = match http.get(&url) {
        Ok(body) => body,
        Err(HttpError::NotFound { .. }) => bail!(
            "crucible: GitHub has no attestation for pnpm {tag} asset {asset} \
             (sha256:{digest_hex}); check the release and its asset digest at \
             https://github.com/pnpm/pnpm/releases/tag/{tag}"
        ),
        Err(HttpError::Status { code: 403, .. }) => bail!(
            "crucible: GitHub rate-limited the attestation lookup for {tag} asset {asset}; \
             set GITHUB_TOKEN to a token with read access and retry `crucible update`"
        ),
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "crucible: could not look up the attestation for pnpm {tag} asset {asset}; \
                     check network access to {url} and retry `crucible update`"
                )
            });
        }
    };
    let response: Response = serde_json::from_slice(&body).with_context(|| {
        format!("crucible: failed to parse the attestation response for {asset} ({tag}) from {url}")
    })?;

    for attestation in &response.attestations {
        let bytes = STANDARD
            .decode(&attestation.bundle.dsse_envelope.payload)
            .context(
                "crucible: an attestation's DSSE payload is not base64; check the GitHub response",
            )?;
        let statement: Statement = serde_json::from_slice(&bytes).context(
            "crucible: an attestation's DSSE payload is not an in-toto statement; check the GitHub response",
        )?;
        if predicate_matches(&statement, tag) && has_subject(&statement, asset, digest_hex) {
            return Ok(());
        }
    }
    bail!(
        "crucible: GitHub has no release attestation for pnpm {tag}: {asset} at \
         sha256:{digest_hex} (see https://github.com/pnpm/pnpm/attestations); check the release \
         assets and retry `crucible update`"
    )
}

/// The predicate ties an attestation to one pnpm release.
fn predicate_matches(statement: &Statement, tag: &str) -> bool {
    statement.predicate_type == RELEASE
        && statement.predicate.repository == REPOSITORY
        && statement.predicate.tag == tag
}

/// The subject names the asset and the digest GitHub recorded for it.
fn has_subject(statement: &Statement, asset: &str, digest_hex: &str) -> bool {
    statement.subject.iter().any(|subject| {
        subject.name.as_deref() == Some(asset)
            && subject.digest.sha256.as_deref() == Some(digest_hex)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    const DIGEST: &str = "b515e29f2ac1adc0fbe63173414a6682827baff24f17e7ad3476821bdef236ac";
    const ASSET: &str = "pnpm-linux-x64.tar.gz";
    const TAG: &str = "v12.23.0";

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

    type Get = Box<dyn Fn(&str) -> Result<Vec<u8>, HttpError>>;

    struct Fake {
        get: Get,
    }

    impl Fetch for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            (self.get)(url)
        }
    }

    fn fake(body: &str) -> Fake {
        let body = body.to_string();
        Fake {
            get: Box::new(move |_| Ok(body.clone().into_bytes())),
        }
    }

    fn failing(make: impl Fn() -> HttpError + 'static) -> Fake {
        Fake {
            get: Box::new(move |_| Err(make())),
        }
    }

    /// Wrap an in-toto statement in the DSSE envelope GitHub's API returns.
    fn response(statement: &str) -> String {
        let payload = STANDARD.encode(statement);
        format!(r#"{{"attestations":[{{"bundle":{{"dsseEnvelope":{{"payload":"{payload}"}}}}}}]}}"#)
    }

    fn statement(json: &str) -> Statement {
        serde_json::from_str(json).unwrap()
    }

    fn verifies(statement: &str, digest: &str, asset: &str, tag: &str) -> Result<()> {
        verify(&fake(&response(statement)), digest, asset, tag)
    }

    #[test]
    fn parses_a_release_attestation() {
        let s = statement(STATEMENT);
        assert_eq!(s.predicate.repository, "pnpm/pnpm");
        assert_eq!(s.predicate.tag, TAG);
        let subject = &s.subject[0];
        assert_eq!(subject.name.as_deref(), Some(ASSET));
        assert_eq!(subject.digest.sha256.as_deref(), Some(DIGEST));
    }

    #[test]
    fn accepts_a_real_shaped_attestation() {
        verifies(STATEMENT, DIGEST, ASSET, TAG).unwrap();
    }

    #[test]
    fn predicate_and_subject_are_checked_independently() {
        let s = statement(STATEMENT);
        assert!(predicate_matches(&s, TAG));
        assert!(!predicate_matches(&s, "v12.24.0"));
        assert!(has_subject(&s, ASSET, DIGEST));
        assert!(!has_subject(&s, "pnpm-linux-arm64.tar.gz", DIGEST));
        assert!(!has_subject(&s, ASSET, "deadbeef"));
    }

    #[test]
    fn rejects_a_wrong_tag_repository_or_in_toto_type() {
        let wrong_tag = verifies(STATEMENT, DIGEST, ASSET, "v12.24.0").unwrap_err();
        assert!(wrong_tag.to_string().contains("no release attestation"));

        let other_repo = STATEMENT.replace("pnpm/pnpm", "attacker/pnpm");
        let wrong_repo = verifies(&other_repo, DIGEST, ASSET, TAG).unwrap_err();
        assert!(wrong_repo.to_string().contains("no release attestation"));

        let other_type = STATEMENT.replace(
            "https://in-toto.io/attestation/release/v0.2",
            "https://in-toto.io/attestation/unrelated/v0.2",
        );
        let wrong_type = verifies(&other_type, DIGEST, ASSET, TAG).unwrap_err();
        assert!(wrong_type.to_string().contains("no release attestation"));
    }

    #[test]
    fn rejects_a_wrong_asset_or_digest() {
        let wrong_asset = verifies(STATEMENT, DIGEST, "pnpm-linux-arm64.tar.gz", TAG).unwrap_err();
        assert!(wrong_asset.to_string().contains("pnpm-linux-arm64.tar.gz"));
        let wrong_digest = verifies(STATEMENT, "deadbeef", ASSET, TAG).unwrap_err();
        assert!(wrong_digest.to_string().contains("deadbeef"));
    }

    #[test]
    fn rejects_a_response_with_no_attestations() {
        let err = verify(&fake(r#"{"attestations":[]}"#), DIGEST, ASSET, TAG).unwrap_err();
        assert!(err.to_string().contains("no release attestation"), "{err}");
    }

    #[test]
    fn rejects_a_malformed_dsse_payload() {
        let not_base64 = r#"{"attestations":[{"bundle":{"dsseEnvelope":{"payload":"!!!"}}}]}"#;
        let err = verify(&fake(not_base64), DIGEST, ASSET, TAG).unwrap_err();
        assert!(err.to_string().contains("not base64"), "{err}");
        let err = verifies("not json", DIGEST, ASSET, TAG).unwrap_err();
        assert!(
            err.to_string().contains("not an in-toto statement"),
            "{err}"
        );
    }

    #[test]
    fn a_404_names_the_release_and_asset() {
        let http = failing(|| HttpError::NotFound {
            url: format!("{API}{DIGEST}"),
        });
        let err = verify(&http, DIGEST, ASSET, TAG).unwrap_err();
        let message = err.to_string();
        assert!(message.starts_with("crucible:"), "{message}");
        assert!(
            message.contains(ASSET) && message.contains(TAG),
            "{message}"
        );
        assert!(message.contains("check the release"), "{message}");
        assert!(!message.contains("returned 404"), "{message}");
    }

    #[test]
    fn a_403_hints_at_github_token() {
        let http = failing(|| HttpError::Status {
            url: format!("{API}{DIGEST}"),
            code: 403,
        });
        let err = verify(&http, DIGEST, ASSET, TAG).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("GITHUB_TOKEN"), "{message}");
        assert!(message.contains("retry `crucible update`"), "{message}");
    }

    #[test]
    fn other_fetch_errors_carry_context() {
        let http = failing(|| HttpError::Transport {
            url: format!("{API}{DIGEST}"),
            source: Box::new(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
        });
        let err = verify(&http, DIGEST, ASSET, TAG).unwrap_err();
        let message = err.to_string();
        assert!(message.starts_with("crucible:"), "{message}");
        assert!(
            message.contains(ASSET) && message.contains(TAG),
            "{message}"
        );
        assert!(format!("{err:#}").contains("reset"), "{err:#}");
    }
}
