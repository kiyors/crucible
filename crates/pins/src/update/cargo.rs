//! `update cargo`: pin a huacnlee/cargo release.
//!
//! Every release publishes `SHA256SUMS`, so no tarball has to be downloaded to
//! learn its hash.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{json, parse_sha256sums, Plan};
use crate::http::Http;
use crate::sri;

const REPO: &str = "huacnlee/cargo";

/// Nix system -> Rust host triple, in `lib/common/systems.nix` order.
const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-unknown-linux-gnu"),
    ("aarch64-linux", "aarch64-unknown-linux-gnu"),
    ("aarch64-darwin", "aarch64-apple-darwin"),
];

#[derive(Serialize)]
struct Sources {
    version: String,
    platforms: Map<String, Value>,
}

/// Fetch and validate `data/rust/cargo.json`, without writing it.
pub fn plan(http: &Http, version: Option<String>, root: &Path) -> Result<Plan> {
    let path = root.join("data/rust/cargo.json");
    let current = read_version(&path)?;
    let version = match version {
        Some(version) => version,
        None => latest_version(http)?,
    };
    println!("huacnlee/cargo: {current} -> {version}");

    let sums = parse_sha256sums(&sha256sums(http, &version)?);
    let mut platforms = Map::new();
    for (system, triple) in PLATFORMS {
        let file = format!("cargo-{version}-{triple}.tar.gz");
        let hex = sums
            .iter()
            .find(|(_, entry)| *entry == file)
            .map(|(hex, _)| hex)
            .with_context(|| format!("SHA256SUMS of v{version} has no {file}"))?;
        let hash = sri::from_hex(hex)?;
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

fn sha256sums(http: &Http, version: &str) -> Result<String> {
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/SHA256SUMS");
    String::from_utf8(http.get(&url)?).with_context(|| format!("{url} is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_release_urls() {
        assert_eq!(
            release_url("0.103.0-1", "cargo-0.103.0-1-aarch64-apple-darwin.tar.gz"),
            "https://github.com/huacnlee/cargo/releases/download/v0.103.0-1/cargo-0.103.0-1-aarch64-apple-darwin.tar.gz"
        );
    }
}
