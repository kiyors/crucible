//! `update node` also pins Vite+: the `vp` binary for each system and the
//! JavaScript packages it runs, which ship separately.
//!
//! Each GitHub release publishes `vp-checksums.txt` for its archives and
//! `vp-install-lock.yaml`, the pnpm lockfile vp's own installer uses for those
//! packages. Only the lockfile (checked against the checksums) is downloaded.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use super::node::Source;
use super::{json, parse_sha256sums, Plan};
use crate::http::Http;
use crate::pnpm_lock::{Graph, Lockfile};
use crate::sri;

const REPO: &str = "voidzero-dev/vite-plus";
const LOCKFILE: &str = "vp-install-lock.yaml";

/// Nix system -> Rust host triple of the release archive, in
/// `lib/common/systems.nix` order.
const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-unknown-linux-gnu"),
    ("aarch64-linux", "aarch64-unknown-linux-gnu"),
    ("aarch64-darwin", "aarch64-apple-darwin"),
];

/// The pinned format of `data/node/vite-plus.json`.
#[derive(Serialize)]
struct Pins {
    version: String,
    platforms: IndexMap<String, Source>,
    #[serde(flatten)]
    graph: Graph,
}

/// Fetch and validate `data/node/vite-plus.json`, without writing it.
pub fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let version = latest_version(http)?;
    println!("vite-plus: {version}");

    let sums = parse_sha256sums(&download(http, &version, "vp-checksums.txt")?);
    let hash_of = |file: &str| -> Result<String> {
        let hex = sums
            .iter()
            .find(|(_, entry)| entry == file)
            .map(|(hex, _)| hex)
            .with_context(|| format!("vp-checksums.txt of v{version} has no {file}"))?;
        sri::from_hex(hex)
    };

    let mut platforms = IndexMap::new();
    for (system, triple) in PLATFORMS {
        let file = format!("vp-{triple}.tar.gz");
        platforms.insert(
            system.to_string(),
            Source {
                url: release_url(&version, &file),
                hash: hash_of(&file)?,
            },
        );
    }

    let lock = download(http, &version, LOCKFILE)?;
    if sri::sri(lock.as_bytes()) != hash_of(LOCKFILE)? {
        bail!("{LOCKFILE} of v{version} does not match vp-checksums.txt");
    }
    let graph = Lockfile::parse(&lock)?.graph("vite-plus")?;
    let root_url = &graph.packages[&graph.root].url;
    if !root_url.ends_with(&format!("/vite-plus-{version}.tgz")) {
        bail!("{LOCKFILE} of v{version} installs {root_url}");
    }
    println!("  {} packages", graph.packages.len());

    let pins = Pins {
        version,
        platforms,
        graph,
    };
    Ok(vec![(root.join("data/node/vite-plus.json"), json(&pins)?)])
}

fn latest_version(http: &Http) -> Result<String> {
    let body = http.get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))?;
    let json: Value =
        serde_json::from_slice(&body).context("failed to parse the latest release")?;
    let tag = json
        .get("tag_name")
        .and_then(Value::as_str)
        .context("the latest release has no tag_name")?;
    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

fn release_url(version: &str, file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{file}")
}

fn download(http: &Http, version: &str, file: &str) -> Result<String> {
    let url = release_url(version, file);
    String::from_utf8(http.get(&url)?).with_context(|| format!("{url} is not UTF-8"))
}
