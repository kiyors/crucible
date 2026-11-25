//! The pieces of a release pin that every updater shares: the `{ url, hash }`
//! pair written into `data/`, looking a hash up from a checksum file, and
//! building and checking a GitHub release asset URL.
//!
//! These used to be redefined in each `update/<lang>.rs`. Keeping them here
//! means a change to a URL rule or to how a checksum is trusted has one place
//! to land; the language modules keep only their own archive naming.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::attestation::Fetch;
use crate::sri;

/// One platform's artifact, as written into `data/<lang>/*.json`.
#[derive(Debug, Serialize)]
pub(crate) struct Source {
    pub(crate) url: String,
    pub(crate) hash: String,
}

/// The SRI hash of `file` in a `SHA256SUMS`-style list. `source` names the
/// checksum file so a missing row says where to look rather than just "no
/// hash".
pub(crate) fn hash_of(sums: &[(String, String)], file: &str, source: &str) -> Result<String> {
    let hex = sums
        .iter()
        .find(|(_, entry)| entry == file)
        .map(|(hex, _)| hex)
        .with_context(|| format!("{source} has no {file}"))?;
    sri::from_hex(hex)
}

/// Must match `releaseUrl` in `lib/python/toolchain.nix`. GitHub serves the
/// `+` in python-build-standalone's names only percent-encoded.
pub(crate) fn release_url(repo: &str, tag: &str, file: &str) -> String {
    format!(
        "https://github.com/{repo}/releases/download/{tag}/{}",
        file.replace('+', "%2B")
    )
}

/// Read one of the pinned JSON files. Only `update --check` reads pins; the
/// updater itself builds new ones in memory and never parses its own output.
pub(crate) fn read_json(root: &Path, relative: &str) -> Result<Value> {
    let path = root.join(relative);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// The SRI hash pinned for `system` in one pin file.
pub(crate) fn platform_hash(pins: &Value, system: &str) -> Result<String> {
    pins.get("platforms")
        .and_then(|platforms| platforms.get(system))
        .and_then(|source| source.get("hash"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("pinned data has no hash for {system}"))
}

/// The newest release of `repo`, as the GitHub API reports it. `update` uses
/// this to pick a version; `update --check` never does, because that would fail
/// on every upstream release after the pin was made.
pub(crate) fn latest_release<F: Fetch>(http: &F, repo: &str) -> Result<Value> {
    let body = http.get(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    serde_json::from_slice(&body)
        .with_context(|| format!("failed to parse {repo}'s latest release"))
}

/// One exact release of `repo`, named by its tag. This is what provenance checks
/// use: the pinned version identifies the release, so a newer one upstream does
/// not change what is being verified.
pub(crate) fn release_by_tag<F: Fetch>(http: &F, repo: &str, tag: &str) -> Result<Value> {
    let body = http.get(&format!(
        "https://api.github.com/repos/{repo}/releases/tags/{tag}"
    ))?;
    serde_json::from_slice(&body).with_context(|| format!("failed to parse {repo}'s {tag} release"))
}

/// The release's tag, without a leading `v`.
pub(crate) fn tag(release: &Value) -> Result<String> {
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .map(|tag| tag.strip_prefix('v').unwrap_or(tag).to_string())
        .context("the latest release has no tag_name")
}

/// A release's checksum file, after checking it against the sha256 GitHub
/// computed when it was uploaded: the hashes in it are only as good as the
/// file, and neither project signs it.
pub(crate) fn checked_asset<F: Fetch>(
    http: &F,
    repo: &str,
    release: &Value,
    name: &str,
) -> Result<String> {
    let tag = release_tag(release)?;
    let url = release_url(repo, tag, name);
    let body = http.get(&url)?;
    verified_asset(release, name, &body)
        .with_context(|| format!("{url} is not the release asset GitHub recorded"))
}

/// The pure half of [`checked_asset`]: check an already-fetched asset body
/// against the digest in the release JSON and decode it as UTF-8. Keeping the
/// check separate from the fetch makes the trust decision testable offline.
pub(crate) fn verified_asset(release: &Value, name: &str, body: &[u8]) -> Result<String> {
    let tag = release_tag(release)?;
    let digest = asset_digest(release, name)
        .with_context(|| format!("release {tag} has no {name} with a sha256 digest"))?;
    if sri::sri(body) != sri::from_hex(digest)? {
        bail!("{name} does not match the digest GitHub recorded for it");
    }
    String::from_utf8(body.to_vec()).with_context(|| format!("{name} is not UTF-8"))
}

fn release_tag(release: &Value) -> Result<&str> {
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .context("the latest release has no tag_name")
}

/// The hex sha256 of a release asset, from its `digest` ("sha256:<hex>").
pub(crate) fn asset_digest<'a>(release: &'a Value, name: &str) -> Option<&'a str> {
    release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|asset| asset.get("name").and_then(Value::as_str) == Some(name))?
        .get("digest")?
        .as_str()?
        .strip_prefix("sha256:")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "3fe0d7c0538eb5d190c34582992d26230532551355cbd789fabc6591621cfc07";

    #[test]
    fn looks_up_a_hash_by_filename() {
        let sums = parse_sums(&format!("{HEX}  cpython-3.14.10+build-triple.tar.gz\n"));
        assert_eq!(
            hash_of(&sums, "cpython-3.14.10+build-triple.tar.gz", "SHA256SUMS").unwrap(),
            crate::sri::from_hex(HEX).unwrap()
        );
        let err = hash_of(&sums, "missing.tar.gz", "SHA256SUMS").unwrap_err();
        assert!(err.to_string().contains("SHA256SUMS has no missing.tar.gz"));
    }

    #[test]
    fn encodes_the_plus_in_a_release_url() {
        assert_eq!(
            release_url("astral-sh/python-build-standalone", "20261110", "a+b.tar.gz"),
            "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/a%2Bb.tar.gz"
        );
    }

    #[test]
    fn reads_a_tag_and_an_asset_digest() {
        let release: Value = serde_json::from_str(
            r#"{ "tag_name": "v0.13.9", "assets": [
              { "name": "sha256.sum", "digest": "sha256:ab12" },
              { "name": "old.sum", "digest": null }
            ] }"#,
        )
        .unwrap();
        assert_eq!(tag(&release).unwrap(), "0.13.9");
        assert_eq!(asset_digest(&release, "sha256.sum"), Some("ab12"));
        assert_eq!(asset_digest(&release, "old.sum"), None);
        assert_eq!(asset_digest(&release, "missing"), None);
    }

    fn release(digest: &str) -> Value {
        serde_json::json!({
            "tag_name": "v0.103.0-1",
            "assets": [
                { "name": "SHA256SUMS", "digest": format!("sha256:{digest}") },
                { "name": "unsigned.txt", "digest": null }
            ]
        })
    }

    #[test]
    fn trusts_an_asset_only_when_the_recorded_digest_matches() {
        // sha256 of exactly these bytes, as GitHub records it.
        let digest = "5c635caa1ce13ffc61036e5d19be120cec7a066829d4f1c77a6907a5780ec623";
        let body = b"4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd  a.tar.gz\n";
        assert_eq!(
            verified_asset(&release(digest), "SHA256SUMS", body).unwrap(),
            String::from_utf8(body.to_vec()).unwrap()
        );
        let tampered =
            b"0000000000000000000000000000000000000000000000000000000000000000  a.tar.gz\n";
        let err = verified_asset(&release(digest), "SHA256SUMS", tampered)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match the digest"), "{err}");
        assert!(verified_asset(&release(digest), "unsigned.txt", body).is_err());
        assert!(verified_asset(&release(digest), "missing", body).is_err());
        assert!(verified_asset(&release("zz"), "SHA256SUMS", body).is_err());
    }

    fn parse_sums(text: &str) -> Vec<(String, String)> {
        crate::update::parse_sha256sums(text)
    }
}
