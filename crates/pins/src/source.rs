//! The pieces of a release pin that every updater shares: the `{ url, hash }`
//! pair written into `data/`, looking a hash up from a checksum file, and
//! building and checking a GitHub release asset URL.
//!
//! These used to be redefined in each `update/<lang>.rs`. Keeping them here
//! means a change to a URL rule or to how a checksum is trusted has one place
//! to land; the language modules keep only their own archive naming.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;

use crate::http::Http;
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

/// The newest release of `repo`, as the GitHub API reports it.
pub(crate) fn latest_release(http: &Http, repo: &str) -> Result<Value> {
    let body = http.get(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    serde_json::from_slice(&body)
        .with_context(|| format!("failed to parse {repo}'s latest release"))
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
pub(crate) fn checked_asset(
    http: &Http,
    repo: &str,
    release: &Value,
    name: &str,
) -> Result<String> {
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .context("the latest release has no tag_name")?;
    let digest = asset_digest(release, name)
        .with_context(|| format!("{repo} {tag} has no {name} with a sha256 digest"))?;
    let url = release_url(repo, tag, name);
    let body = http.get(&url)?;
    if sri::sri(&body) != sri::from_hex(digest)? {
        bail!("{url} does not match the digest GitHub recorded for it");
    }
    String::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))
}

/// The hex sha256 of a release asset, from its `digest` ("sha256:<hex>").
fn asset_digest<'a>(release: &'a Value, name: &str) -> Option<&'a str> {
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

    fn parse_sums(text: &str) -> Vec<(String, String)> {
        crate::update::parse_sha256sums(text)
    }
}
