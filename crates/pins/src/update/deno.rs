//! `update deno`: pin the newest stable release of each pinned major, from
//! denoland/deno's GitHub releases.
//!
//! Deno is not an npm package manager, so its pins are separate from the
//! package-manager ones. Each release ships one zip per platform holding the
//! deno executable, and the GitHub API lists the assets with their sha256
//! digests, so only metadata is downloaded here: the Nix side fetches and
//! unpacks the zips with those hashes.
//!
//! GitHub's recorded digest is the trust anchor: GitHub computes it when the
//! asset is uploaded, and an asset without one is fatal rather than falling
//! back to an unverified source. Deno does not publish an attestation or a
//! detached signature we could check locally, so there is deliberately no
//! attestation call here; this is the same trust tier as bun's checked
//! `SHASUMS256.txt`, not a GPG-equivalent independent anchor.

use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{json, Plan};
use crate::http::Http;
use crate::source::Source;
use crate::sri;

/// denoland/deno's release list: newest first, with each release's assets.
const RELEASES: &str = "https://api.github.com/repos/denoland/deno/releases?per_page=100";

/// Nix system -> the deno release zip, in `lib/common/systems.nix` order.
pub(crate) const DENO_ASSETS: [(&str, &str); 3] = [
    ("x86_64-linux", "deno-x86_64-unknown-linux-gnu.zip"),
    ("aarch64-linux", "deno-aarch64-unknown-linux-gnu.zip"),
    ("aarch64-darwin", "deno-aarch64-apple-darwin.zip"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(DENO_ASSETS));

/// The majors whose newest stable release is pinned. Deno 2 is the current
/// line; 1.46.3 was the last 1.x release, a year and a half before 2.0.
const DENO_MAJORS: [&str; 1] = ["2"];

/// One denoland/deno release, as the release list API reports it.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<Asset>,
}

/// One asset of a denoland/deno release; `digest` is null on the releases
/// that predate GitHub computing one.
#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
}

/// The pinned format of `data/deno/deno.json`: major -> the newest stable
/// release, with its per-system sources.
#[derive(Debug, Serialize)]
struct Pins(IndexMap<String, Pin>);

#[derive(Debug, Serialize)]
struct Pin {
    version: String,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/deno/deno.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let releases: Vec<Release> = serde_json::from_slice(&http.get(RELEASES)?)
        .context("failed to parse the deno release list")?;
    let mut pins = IndexMap::new();
    for major in DENO_MAJORS {
        let release = newest_stable(&releases, major)?;
        let version = release.tag_name.trim_start_matches('v').to_string();
        let mut platforms = IndexMap::new();
        for (system, asset) in DENO_ASSETS {
            // GitHub's recorded digest is the anchor; never fall back to a
            // hash from anywhere else when it is missing.
            let digest = release
                .assets
                .iter()
                .find(|a| a.name == asset)
                .and_then(|a| a.digest.as_deref())
                .and_then(|digest| digest.strip_prefix("sha256:"))
                .with_context(|| {
                    format!(
                        "{} has no {asset} asset with a sha256 digest",
                        release.tag_name
                    )
                })?;
            platforms.insert(
                system.to_string(),
                Source {
                    url: format!(
                        "https://github.com/denoland/deno/releases/download/{}/{asset}",
                        release.tag_name
                    ),
                    hash: sri::from_hex(digest)?,
                },
            );
        }
        println!("deno {major}: {version}");
        pins.insert(major.to_string(), Pin { version, platforms });
    }
    Ok(vec![(root.join("data/deno/deno.json"), json(&Pins(pins))?)])
}

/// The newest stable denoland/deno release of `major`: the first of that
/// major, newest first, that is neither a draft nor a prerelease.
fn newest_stable<'a>(releases: &'a [Release], major: &str) -> Result<&'a Release> {
    let prefix = format!("v{major}.");
    releases
        .iter()
        .find(|release| {
            !release.draft && !release.prerelease && release.tag_name.starts_with(&prefix)
        })
        .with_context(|| format!("GitHub has no stable deno {major} release"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_newest_stable_release_of_a_major() {
        let releases: Vec<Release> = serde_json::from_str(
            r#"[
              { "tag_name": "v2.9.8", "prerelease": false, "draft": false, "assets": [] },
              { "tag_name": "v2.9.7-rc.1", "prerelease": true, "draft": false, "assets": [] },
              { "tag_name": "v2.9.6", "prerelease": false, "draft": false, "assets": [] },
              { "tag_name": "v2.9.7", "prerelease": false, "draft": true, "assets": [] },
              { "tag_name": "v1.46.3", "prerelease": false, "draft": false, "assets": [] }
            ]"#,
        )
        .unwrap();
        let release = newest_stable(&releases, "2").unwrap();
        assert_eq!(release.tag_name, "v2.9.8");
        assert!(newest_stable(&releases, "3").is_err());
    }
}
