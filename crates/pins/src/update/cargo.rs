//! `update cargo`: pin a huacnlee/cargo release.
//!
//! Every release publishes `SHA256SUMS`, so no tarball has to be downloaded to
//! learn its hash. The file itself is checked against the sha256 GitHub
//! recorded for that release asset, the same anchor python/uv use: the hashes
//! in it are only as good as the file they came in.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{json, parse_sha256sums_checked, Fetch, Plan};
use crate::http::Http;
use crate::source::{self, hash_of};

const REPO: &str = "huacnlee/cargo";
const SUMS: &str = "SHA256SUMS";

/// Nix system -> Rust host triple, in `lib/common/systems.nix` order.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-unknown-linux-gnu"),
    ("aarch64-linux", "aarch64-unknown-linux-gnu"),
    ("aarch64-darwin", "aarch64-apple-darwin"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

#[derive(Serialize)]
struct Sources {
    version: String,
    platforms: Map<String, Value>,
}

/// Fetch and validate `data/rust/cargo.json`, without writing it.
pub(crate) fn plan(http: &Http, version: Option<String>, root: &Path) -> Result<Plan> {
    let path = root.join("data/rust/cargo.json");
    let current = read_version(&path)?;
    let release = release(http, version.as_deref())?;
    let version = source::tag(&release)?;
    println!("huacnlee/cargo: {current} -> {version}");

    let sums = checksums(http, &release)?;
    let mut platforms = Map::new();
    for (system, triple) in PLATFORMS {
        let file = format!("cargo-{version}-{triple}.tar.gz");
        let hash = hash_of(&sums, &file, &format!("SHA256SUMS of v{version}"))?;
        println!("  {system}: {hash}");

        let platform = Map::from_iter([
            (
                "url".to_string(),
                Value::String(release_url(&version, &file)),
            ),
            ("hash".to_string(), Value::String(hash)),
        ]);
        platforms.insert(system.to_string(), Value::Object(platform));
    }

    let bytes = json(&Sources { version, platforms })?;
    Ok(vec![(path, bytes)])
}

fn read_version(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let json: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    json.get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{} has no version", path.display()))
}

/// An explicit `--version` pins that tag rather than whatever is newest; both
/// carry the same asset digest in their release JSON.
fn release(http: &Http, version: Option<&str>) -> Result<Value> {
    let endpoint = match version {
        Some(version) => format!("releases/tags/v{version}"),
        None => "releases/latest".to_string(),
    };
    let body = http.get(&format!("https://api.github.com/repos/{REPO}/{endpoint}"))?;
    serde_json::from_slice(&body).context("failed to parse the release")
}

fn release_url(version: &str, file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{file}")
}

/// Fetch `SHA256SUMS`, refusing it unless its bytes match the digest GitHub
/// recorded for the asset, then parse every row strictly.
fn checksums<F: Fetch>(http: &F, release: &Value) -> Result<Vec<(String, String)>> {
    let text = source::checked_asset(http, REPO, release, SUMS)?;
    parse_sha256sums_checked(&text)
}

/// Re-verify the pin in `root` against upstream, without picking a new
/// release. The pinned version names the tag; its `SHA256SUMS` is still checked
/// against the asset digest GitHub recorded before any row is trusted.
pub(crate) fn verify<F: Fetch>(http: &F, root: &Path) -> Result<()> {
    let pins = source::read_json(root, "data/rust/cargo.json")?;
    let version = pins
        .get("version")
        .and_then(Value::as_str)
        .context("data/rust/cargo.json has no version")?;
    let release = source::release_by_tag(http, REPO, &format!("v{version}"))?;
    let sums = checksums(http, &release)?;
    for (system, triple) in PLATFORMS {
        let file = format!("cargo-{version}-{triple}.tar.gz");
        let upstream = hash_of(&sums, &file, &format!("SHA256SUMS of v{version}"))?;
        let pinned = source::platform_hash(&pins, system)?;
        if upstream != pinned {
            bail!(
                "cargo {version} {system}: pinned {pinned}, but v{version}'s SHA256SUMS says {upstream}"
            );
        }
    }
    Ok(())
}

/// The pure counterpart of [`checksums`]: verify an already-fetched body
/// against the release JSON and parse it, so a captured `SHA256SUMS` can be
/// checked offline.
#[cfg(test)]
fn verify_sums(release: &Value, body: &[u8]) -> Result<Vec<(String, String)>> {
    let text = source::verified_asset(release, SUMS, body)?;
    parse_sha256sums_checked(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "5c635caa1ce13ffc61036e5d19be120cec7a066829d4f1c77a6907a5780ec623";
    const BODY: &[u8] =
        b"4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd  a.tar.gz\n";

    fn release(digest: &str) -> Value {
        serde_json::json!({
            "tag_name": "v0.103.0-1",
            "assets": [{ "name": "SHA256SUMS", "digest": format!("sha256:{digest}") }]
        })
    }

    #[test]
    fn builds_release_urls() {
        assert_eq!(
            release_url("0.103.0-1", "cargo-0.103.0-1-aarch64-apple-darwin.tar.gz"),
            "https://github.com/huacnlee/cargo/releases/download/v0.103.0-1/cargo-0.103.0-1-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn verifies_the_sums_against_the_release_asset_digest() {
        let sums = verify_sums(&release(DIGEST), BODY).unwrap();
        assert_eq!(sums.len(), 1);
        assert_eq!(sums[0].1, "a.tar.gz");

        let tampered =
            b"4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd  b.tar.gz\n";
        assert!(verify_sums(&release(DIGEST), tampered).is_err());
        assert!(verify_sums(&release("zz"), BODY).is_err());
        assert!(verify_sums(&release(DIGEST), b"not a sums row\n").is_err());
    }
}
