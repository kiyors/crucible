//! `update node` also pins Vite+: the `vp` binary for each system and the
//! JavaScript packages it runs, which ship separately.
//!
//! Each GitHub release publishes `vp-checksums.txt` for its archives and
//! `vp-install-lock.yaml`, the pnpm lockfile vp's own installer uses for those
//! packages. Only the lockfile (checked against the checksums) is downloaded.
//! The checksums file itself is checked against the sha256 GitHub recorded for
//! the release asset before any of its hashes are trusted.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use super::{json, parse_sha256sums_checked, Plan};
use crate::http::Http;
use crate::pnpm_lock::{Graph, Lockfile};
use crate::source::{self, hash_of as source_hash_of, Source};
use crate::sri;

const REPO: &str = "voidzero-dev/vite-plus";
const LOCKFILE: &str = "vp-install-lock.yaml";
const SUMS: &str = "vp-checksums.txt";

/// Nix system -> Rust host triple of the release archive, in
/// `lib/common/systems.nix` order.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-unknown-linux-gnu"),
    ("aarch64-linux", "aarch64-unknown-linux-gnu"),
    ("aarch64-darwin", "aarch64-apple-darwin"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// The pinned format of `data/node/vite-plus.json`.
#[derive(Serialize)]
struct Pins {
    version: String,
    platforms: IndexMap<String, Source>,
    #[serde(flatten)]
    graph: Graph,
}

/// Fetch and validate `data/node/vite-plus.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let release = source::latest_release(http, REPO)?;
    let version = source::tag(&release)?;
    println!("vite-plus: {version}");

    let sums = checksums(http, &release)?;
    let hash_of =
        |file: &str| source_hash_of(&sums, file, &format!("vp-checksums.txt of v{version}"));

    let mut platforms = IndexMap::new();
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .context("the latest release has no tag_name")?;
    for (system, triple) in PLATFORMS {
        let file = format!("vp-{triple}.tar.gz");
        platforms.insert(
            system.to_string(),
            Source {
                url: release_url(tag, &file),
                hash: hash_of(&file)?,
            },
        );
    }

    let lock = download(http, tag, LOCKFILE)?;
    let graph = check_lock(lock.as_bytes(), &sums, &version)?;
    println!("  {} packages", graph.packages.len());

    let pins = Pins {
        version,
        platforms,
        graph,
    };
    Ok(vec![(root.join("data/node/vite-plus.json"), json(&pins)?)])
}

/// Fetch `vp-checksums.txt` only after checking its bytes against the digest
/// GitHub recorded for the asset, then parse every row strictly.
fn checksums(http: &Http, release: &Value) -> Result<Vec<(String, String)>> {
    let text = source::checked_asset(http, REPO, release, SUMS)?;
    parse_sha256sums_checked(&text)
}

/// The pure counterpart of [`checksums`]: verify an already-fetched body
/// against the release JSON so a captured file can be checked offline.
#[cfg(test)]
fn verify_sums(release: &Value, body: &[u8]) -> Result<Vec<(String, String)>> {
    let text = source::verified_asset(release, SUMS, body)?;
    parse_sha256sums_checked(&text)
}

/// The lockfile is the whole JS dependency graph, so check it against the
/// verified checksums *and* confirm its root tarball is the version being
/// pinned before letting it into `data/`. Pure: no network.
fn check_lock(lock: &[u8], sums: &[(String, String)], version: &str) -> Result<Graph> {
    if sri::sri(lock) != source_hash_of(sums, LOCKFILE, &format!("vp-checksums.txt of v{version}"))?
    {
        bail!("{LOCKFILE} of v{version} does not match vp-checksums.txt");
    }
    let lock = std::str::from_utf8(lock).with_context(|| format!("{LOCKFILE} is not UTF-8"))?;
    let graph = Lockfile::parse(lock)?.graph("vite-plus")?;
    let root_url = &graph.packages[&graph.root].url;
    if !root_url.ends_with(&format!("/vite-plus-{version}.tgz")) {
        bail!("{LOCKFILE} of v{version} installs {root_url}");
    }
    Ok(graph)
}

fn release_url(tag: &str, file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{file}")
}

fn download(http: &Http, tag: &str, file: &str) -> Result<String> {
    let url = release_url(tag, file);
    String::from_utf8(http.get(&url)?).with_context(|| format!("{url} is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "1.8.0";
    const TAG: &str = "v1.8.0";
    const LOCKSUM: &str = "723a40e3253f25b64dad638e498b063bf42d5d78521378a5a5421d0eddc8f7d4";
    const SUM_DIGEST: &str = "4209d36b652ceb1b4ca7d7e586d4bbd085486bfaf22b035f87433e7c5e735116";

    const LOCK: &[u8] = include_str!("../../tests/fixtures/vp-install-lock.yaml").as_bytes();
    const SUMS_BODY: &[u8] = include_str!("../../tests/fixtures/vp-checksums.txt").as_bytes();

    fn release(digest: &str) -> Value {
        serde_json::json!({
            "tag_name": TAG,
            "assets": [{ "name": SUMS, "digest": format!("sha256:{digest}") }]
        })
    }

    fn sums() -> Vec<(String, String)> {
        parse_sha256sums_checked(std::str::from_utf8(SUMS_BODY).unwrap()).unwrap()
    }

    #[test]
    fn builds_release_urls() {
        assert_eq!(
            release_url(TAG, "vp-aarch64-apple-darwin.tar.gz"),
            "https://github.com/voidzero-dev/vite-plus/releases/download/v1.8.0/vp-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            release_url(TAG, LOCKFILE),
            "https://github.com/voidzero-dev/vite-plus/releases/download/v1.8.0/vp-install-lock.yaml"
        );
    }

    #[test]
    fn the_fixture_sums_match_the_fixture_lockfile() {
        let sums = sums();
        assert_eq!(sums.len(), 4);
        assert_eq!(sums.iter().find(|(_, f)| f == LOCKFILE).unwrap().0, LOCKSUM);
        for (system, triple) in PLATFORMS {
            let file = format!("vp-{triple}.tar.gz");
            assert!(
                sums.iter().any(|(_, f)| f == &file),
                "no sums row for {system}"
            );
        }
    }

    #[test]
    fn accepts_a_lock_that_matches_the_sums_and_the_version() {
        let graph = check_lock(LOCK, &sums(), VERSION).unwrap();
        assert!(graph.root.starts_with("vite-plus@1.8.0_"));
        assert_eq!(
            graph.packages[&graph.root].url,
            "https://registry.npmjs.org/vite-plus/-/vite-plus-1.8.0.tgz"
        );
        assert_eq!(
            graph.packages[&graph.root].dependencies["vite"],
            "vite@8.3.8000"
        );
    }

    #[test]
    fn rejects_a_tampered_lock() {
        let tampered = String::from_utf8(LOCK.to_vec())
            .unwrap()
            .replace(">=22", ">=18");
        let err = check_lock(tampered.as_bytes(), &sums(), VERSION)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match vp-checksums.txt"), "{err}");
    }

    #[test]
    fn rejects_a_tampered_checksum() {
        let mut sums = sums();
        let row = sums.iter_mut().find(|(_, f)| f == LOCKFILE).unwrap();
        row.0 = "0".repeat(64);
        let err = check_lock(LOCK, &sums, VERSION).unwrap_err().to_string();
        assert!(err.contains("does not match vp-checksums.txt"), "{err}");
    }

    #[test]
    fn rejects_a_lock_for_another_version() {
        // The sums still match, but the root tarball names 1.8.0, not 1.8.1.
        let err = check_lock(LOCK, &sums(), "1.8.1").unwrap_err().to_string();
        assert!(err.contains("installs https://registry.npmjs.org"), "{err}");
    }

    #[test]
    fn verifies_the_checksums_against_the_release_asset_digest() {
        let sums = verify_sums(&release(SUM_DIGEST), SUMS_BODY).unwrap();
        assert_eq!(sums.len(), 4);
        let tampered = b"0000000000000000000000000000000000000000000000000000000000000000  vp-install-lock.yaml\n";
        let err = verify_sums(&release(SUM_DIGEST), tampered)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match the digest"), "{err}");
        assert!(verify_sums(&release("zz"), SUMS_BODY).is_err());
        // A verified file cannot hide a malformed row.
        let dirty = b"0000000000000000000000000000000000000000000000000000000000000000  vp-install-lock.yaml\nnot a row\n";
        // This body really is the release asset; its second row is malformed.
        let dirty_digest = "fc91e6cc39bdcef589d423d0eaf53611313b6fe287f69d8ee9a2d0824df0f61d";
        let err = verify_sums(&release(dirty_digest), dirty)
            .unwrap_err()
            .to_string();
        assert!(err.contains("line 2"), "{err}");
    }
}
