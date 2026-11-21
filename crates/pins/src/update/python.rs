//! `update python`: pin the newest CPython of every supported line, and uv.
//!
//! CPython comes from astral-sh/python-build-standalone, the relocatable
//! builds uv itself installs. One release (a date tag) carries every line, and
//! its `SHA256SUMS` holds the hash of each archive, so nothing but metadata is
//! downloaded. That file is checked against the digest GitHub records for the
//! release asset before its hashes are used. Which lines are supported comes
//! from the release cycle python.org publishes for the devguide.
//!
//! The freethreaded builds have their own lines, keyed with the trailing "t"
//! next to the regular ones: their archives name the GIL builds' versions
//! with a "-freethreaded" infix.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::node::Source;
use super::{json, parse_sha256sums, Plan};
use crate::http::Http;
use crate::sri;

pub const PBS: &str = "astral-sh/python-build-standalone";
const UV: &str = "astral-sh/uv";
const RELEASE_CYCLE: &str = "https://peps.python.org/api/release-cycle.json";
/// The releases of the repo, newest first; each lists its assets' names.
const RELEASES: &str = "https://api.github.com/repos/astral-sh/python-build-standalone/releases";

/// Nix system -> Rust host triple of both projects' archives, in
/// `lib/common/systems.nix` order.
pub const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-unknown-linux-gnu"),
    ("aarch64-linux", "aarch64-unknown-linux-gnu"),
    ("aarch64-darwin", "aarch64-apple-darwin"),
];

/// One line of `release-cycle.json`.
#[derive(Debug, Deserialize)]
pub struct Line {
    /// "feature", "prerelease", "bugfix", "security" or "end-of-life".
    pub status: String,
    /// `YYYY-MM`.
    pub end_of_life: String,
}

/// The pinned format of `data/python/python.json`.
#[derive(Debug, Serialize)]
pub struct Pins {
    /// The python-build-standalone release every pin comes from.
    pub build: String,
    /// Line ("3.14") -> its newest release, oldest line first.
    pub releases: IndexMap<String, Pin>,
}

#[derive(Debug, Serialize)]
pub struct Pin {
    pub version: String,
    /// "bugfix" or "security", so a reader can tell which lines are winding down.
    pub status: String,
    pub end: String,
    /// True on the freethreaded line of the same versions (the line key ends
    /// in "t"), false on the regular one; serialized only when true.
    #[serde(default, skip_serializing_if = "is_false")]
    pub freethreaded: bool,
    pub platforms: IndexMap<String, Source>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// The pinned format of `data/python/uv.json`.
#[derive(Debug, Serialize)]
pub struct UvPins {
    pub version: String,
    pub platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/python/{python,uv}.json`, without writing them.
pub fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let cycle: IndexMap<String, Line> = serde_json::from_slice(&http.get(RELEASE_CYCLE)?)
        .context("failed to parse release-cycle.json")?;

    let release = latest_release(http, PBS)?;
    let build = tag(&release)?;
    println!("python-build-standalone: {build}");
    let sums = parse_sha256sums(&checked_asset(http, PBS, &release, "SHA256SUMS")?);

    let mut pins = Pins {
        build: build.clone(),
        releases: IndexMap::new(),
    };
    for (line, info) in supported(&cycle) {
        let version = newest(&sums, &line, &build).with_context(|| {
            format!("python-build-standalone {build} has no CPython {line} build for every system")
        })?;
        println!("python {line}: {version}");
        pins.releases.insert(
            line.clone(),
            Pin {
                platforms: sources(&sums, &version, &build, false)?,
                version,
                status: info.status.clone(),
                end: info.end_of_life.clone(),
                freethreaded: false,
            },
        );
        // The freethreaded line of the same versions, next to it.
        let ft_line = format!("{line}t");
        if let Some(ft_version) = newest(&sums, &ft_line, &build) {
            pins.releases.insert(
                ft_line,
                Pin {
                    platforms: sources(&sums, &ft_version, &build, true)?,
                    version: ft_version,
                    status: info.status.clone(),
                    end: info.end_of_life.clone(),
                    freethreaded: true,
                },
            );
        }
    }
    if pins.releases.is_empty() {
        bail!("release-cycle.json lists no supported Python line");
    }

    let release = latest_release(http, UV)?;
    let version = tag(&release)?;
    println!("uv: {version}");
    let uv_sums = parse_sha256sums(&checked_asset(http, UV, &release, "sha256.sum")?);
    let mut platforms = IndexMap::new();
    for (system, triple) in PLATFORMS {
        let file = format!("uv-{triple}.tar.gz");
        platforms.insert(
            system.to_string(),
            Source {
                url: release_url(UV, &version, &file),
                hash: hash_of(&uv_sums, &file, &version)?,
            },
        );
    }
    let uv = UvPins { version, platforms };

    Ok(vec![
        (root.join("data/python/python.json"), json(&pins)?),
        (root.join("data/python/uv.json"), json(&uv)?),
    ])
}

/// The lines python.org still fixes, oldest first: "bugfix" and "security".
/// Pre-releases and lines past their end are left out.
fn supported(cycle: &IndexMap<String, Line>) -> Vec<(String, &Line)> {
    let mut lines: Vec<(String, &Line)> = cycle
        .iter()
        .filter(|(_, line)| matches!(line.status.as_str(), "bugfix" | "security"))
        .map(|(name, line)| (name.clone(), line))
        .collect();
    lines.sort_by_key(|(name, _)| version_key(name));
    lines
}

/// The install_only archive of one release for one host. Stripped builds have
/// their own suffix, so they never match. The freethreaded ones name the GIL
/// build's version with a "-freethreaded" infix.
fn archive(version: &str, build: &str, triple: &str, freethreaded: bool) -> String {
    format!(
        "cpython-{version}+{build}-{triple}-{}install_only.tar.gz",
        if freethreaded { "freethreaded-" } else { "" }
    )
}

/// The newest release of `line` that has an archive for every system. A line
/// ending in "t" is the freethreaded one: its archives carry the
/// "-freethreaded" infix and name the GIL release's versions.
fn newest(sums: &[(String, String)], line: &str, build: &str) -> Option<String> {
    let freethreaded = line.ends_with('t');
    let base = line.strip_suffix('t').unwrap_or(line);
    let suffix = format!(
        "+{build}-{}-{}install_only.tar.gz",
        PLATFORMS[0].1,
        if freethreaded { "freethreaded-" } else { "" }
    );
    sums.iter()
        .filter_map(|(_, file)| {
            file.strip_prefix(&format!("cpython-{base}."))
                .and_then(|s| s.strip_suffix(&suffix))
        })
        // A patch number alone: "3.15.0a2" is a pre-release.
        .filter(|patch| !patch.is_empty() && patch.bytes().all(|b| b.is_ascii_digit()))
        .map(|patch| format!("{base}.{patch}"))
        .filter(|version| {
            PLATFORMS.iter().all(|(_, triple)| {
                let file = archive(version, build, triple, freethreaded);
                sums.iter().any(|(_, entry)| *entry == file)
            })
        })
        .max_by_key(|version| version_key(version))
}

/// `{ system: { url, hash } }` for one release, from its build's `SHA256SUMS`.
pub fn sources(
    sums: &[(String, String)],
    version: &str,
    build: &str,
    freethreaded: bool,
) -> Result<IndexMap<String, Source>> {
    let mut platforms = IndexMap::new();
    for (system, triple) in PLATFORMS {
        let file = archive(version, build, triple, freethreaded);
        platforms.insert(
            system.to_string(),
            Source {
                url: release_url(PBS, build, &file),
                hash: hash_of(sums, &file, build)?,
            },
        );
    }
    Ok(platforms)
}

/// The newest python-build-standalone release that carries `version`'s
/// `install_only` archives for every system in `PLATFORMS`, and the build tag
/// they are named with. `pin python` uses this to find the build that
/// contains a release the pins do not cover: it walks the repo's release
/// list from the newest, whose assets the list responses include.
pub fn find_release(http: &Http, version: &str) -> Result<(Value, String)> {
    let mut page = 1;
    loop {
        let releases: Vec<Value> =
            serde_json::from_slice(&http.get(&format!("{RELEASES}?per_page=10&page={page}"))?)
                .context("failed to parse the release list")?;
        if releases.is_empty() {
            bail!("no python-build-standalone release carries CPython {version}");
        }
        for release in releases {
            if let Some(build) = carries(&release, version) {
                return Ok((release, build));
            }
        }
        page += 1;
    }
}

/// The build tag when `release` carries the `install_only` archives of
/// `version` for every system in `PLATFORMS`: they are named
/// `cpython-{version}+{build}-{triple}-install_only.tar.gz`, so the build is
/// read from the names. Freethreaded names do not count, and neither does a
/// release missing one of the systems.
fn carries(release: &Value, version: &str) -> Option<String> {
    let prefix = format!("cpython-{version}+");
    let suffix = "-install_only.tar.gz";
    let assets = release.get("assets")?.as_array()?;
    let mut build = String::new();
    for (_, triple) in PLATFORMS {
        let name = assets
            .iter()
            .filter_map(|asset| asset.get("name")?.as_str())
            .find(|name| {
                name.starts_with(&prefix) && name.ends_with(&format!("-{triple}{suffix}"))
            })?;
        let file = name
            .strip_prefix(&prefix)?
            .strip_suffix(&format!("-{triple}{suffix}"))?;
        if build.is_empty() {
            build = file.to_string();
        } else if build != file {
            return None;
        }
    }
    Some(build)
}

fn hash_of(sums: &[(String, String)], file: &str, release: &str) -> Result<String> {
    let hex = sums
        .iter()
        .find(|(_, entry)| entry == file)
        .map(|(hex, _)| hex)
        .with_context(|| format!("the checksums of {release} have no {file}"))?;
    sri::from_hex(hex)
}

/// Must match `releaseUrl` in `lib/python/toolchain.nix`. GitHub serves the
/// `+` in python-build-standalone's names only percent-encoded.
pub fn release_url(repo: &str, tag: &str, file: &str) -> String {
    format!(
        "https://github.com/{repo}/releases/download/{tag}/{}",
        file.replace('+', "%2B")
    )
}

fn latest_release(http: &Http, repo: &str) -> Result<Value> {
    let body = http.get(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    serde_json::from_slice(&body)
        .with_context(|| format!("failed to parse {repo}'s latest release"))
}

fn tag(release: &Value) -> Result<String> {
    release
        .get("tag_name")
        .and_then(Value::as_str)
        .map(|tag| tag.strip_prefix('v').unwrap_or(tag).to_string())
        .context("the latest release has no tag_name")
}

/// A release's checksum file, after checking it against the sha256 GitHub
/// computed when it was uploaded: the hashes in it are only as good as the
/// file, and neither project signs it.
pub fn checked_asset(http: &Http, repo: &str, release: &Value, name: &str) -> Result<String> {
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

fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD: &str = "20261110";

    fn sums(versions: &[(&str, &[&str])]) -> Vec<(String, String)> {
        let hex = "3fe0d7c0538eb5d190c34582992d26230532551355cbd789fabc6591621cfc07";
        let mut text = String::new();
        for (version, triples) in versions {
            for triple in *triples {
                text.push_str(&format!(
                    "{hex}  {}\n",
                    archive(version, BUILD, triple, false)
                ));
            }
        }
        parse_sha256sums(&text)
    }

    fn ft_sums(versions: &[(&str, &[&str])]) -> Vec<(String, String)> {
        let hex = "3fe0d7c0538eb5d190c34582992d26230532551355cbd789fabc6591621cfc07";
        let mut text = String::new();
        for (version, triples) in versions {
            for triple in *triples {
                text.push_str(&format!(
                    "{hex}  {}\n",
                    archive(version, BUILD, triple, true)
                ));
            }
        }
        parse_sha256sums(&text)
    }

    const ALL: &[&str] = &[
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
    ];

    #[test]
    fn keeps_lines_that_still_get_fixes() {
        let cycle: IndexMap<String, Line> = serde_json::from_str(
            r#"{
              "3.16": { "status": "feature", "end_of_life": "2032-10" },
              "3.15": { "status": "bugfix", "end_of_life": "2031-10" },
              "3.9": { "status": "end-of-life", "end_of_life": "2025-10" },
              "3.13": { "status": "security", "end_of_life": "2029-10" }
            }"#,
        )
        .unwrap();
        let lines: Vec<String> = supported(&cycle)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        assert_eq!(lines, ["3.13", "3.15"]);
    }

    #[test]
    fn picks_the_newest_release_built_for_every_system() {
        let sums = sums(&[
            ("3.14.9", ALL),
            ("3.14.10", ALL),
            // Not for every system, so not pinnable.
            ("3.14.11", &ALL[..2]),
            ("3.15.0a2", ALL),
            ("3.1.4", ALL),
        ]);
        assert_eq!(newest(&sums, "3.14", BUILD).as_deref(), Some("3.14.10"));
        assert_eq!(newest(&sums, "3.15", BUILD), None);
        assert_eq!(newest(&sums, "3.1", BUILD).as_deref(), Some("3.1.4"));
    }

    #[test]
    fn ignores_stripped_builds_and_reads_freethreaded_lines() {
        // Stripped builds have no interpreter files, so the regular line
        // never sees them; the freethreaded line picks up the freethreaded
        // names, which name the GIL version.
        let text = "\
3fe0d7c0538eb5d190c34582992d26230532551355cbd789fabc6591621cfc07  cpython-3.14.10+20261110-x86_64-unknown-linux-gnu-freethreaded-install_only.tar.gz
3fe0d7c0538eb5d190c34582992d26230532551355cbd789fabc6591621cfc07  cpython-3.14.10+20261110-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz
";
        assert_eq!(newest(&parse_sha256sums(text), "3.14", BUILD), None);

        let sums = ft_sums(&[
            ("3.14.9", ALL),
            ("3.14.10", ALL),
            // Not for every system, so not pinnable.
            ("3.14.11", &ALL[..2]),
        ]);
        assert_eq!(newest(&sums, "3.14t", BUILD).as_deref(), Some("3.14.10"));
        assert_eq!(newest(&sums, "3.14", BUILD), None);
    }

    #[test]
    fn maps_sums_to_encoded_urls() {
        let platforms = sources(&sums(&[("3.14.10", ALL)]), "3.14.10", BUILD, false).unwrap();
        assert_eq!(
            platforms["aarch64-darwin"].url,
            "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/cpython-3.14.10%2B20261110-aarch64-apple-darwin-install_only.tar.gz"
        );
        assert_eq!(
            sources(&ft_sums(&[("3.14.10", ALL)]), "3.14.10", BUILD, true)
                .unwrap()["aarch64-darwin"]
                .url,
            "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/cpython-3.14.10%2B20261110-aarch64-apple-darwin-freethreaded-install_only.tar.gz"
        );
        assert!(platforms["x86_64-linux"].hash.starts_with("sha256-"));
        assert!(sources(&sums(&[("3.14.10", &ALL[..2])]), "3.14.10", BUILD, false).is_err());
    }

    #[test]
    fn reads_the_build_a_release_carries() {
        let release: Value = serde_json::from_str(
            r#"{
              "tag_name": "20251120",
              "assets": [
                { "name": "SHA256SUMS" },
                { "name": "cpython-3.14.0+20251120-x86_64-unknown-linux-gnu-install_only.tar.gz" },
                { "name": "cpython-3.14.0+20251120-aarch64-unknown-linux-gnu-install_only.tar.gz" },
                { "name": "cpython-3.14.0+20251120-aarch64-apple-darwin-install_only.tar.gz" }
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(carries(&release, "3.14.0").as_deref(), Some("20251120"));

        // A release that misses a system, or names it freethreaded, is not
        // the one that carries a GIL release of that version.
        let partial: Value = serde_json::from_str(
            r#"{
              "tag_name": "20251120",
              "assets": [
                { "name": "cpython-3.14.0+20251120-x86_64-unknown-linux-gnu-install_only.tar.gz" },
                { "name": "cpython-3.14.0+20251120-aarch64-unknown-linux-gnu-install_only.tar.gz" },
                { "name": "cpython-3.14.0+20251120-aarch64-apple-darwin-freethreaded-install_only.tar.gz" }
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(carries(&partial, "3.14.0"), None);
    }

    #[test]
    fn reads_an_assets_digest() {
        let release: Value = serde_json::from_str(
            r#"{ "tag_name": "0.13.9", "assets": [
              { "name": "uv-aarch64-apple-darwin.tar.gz", "digest": "sha256:00" },
              { "name": "sha256.sum", "digest": "sha256:ab12" },
              { "name": "old.sum", "digest": null }
            ] }"#,
        )
        .unwrap();
        assert_eq!(asset_digest(&release, "sha256.sum"), Some("ab12"));
        assert_eq!(asset_digest(&release, "old.sum"), None);
        assert_eq!(asset_digest(&release, "missing"), None);
        assert_eq!(tag(&release).unwrap(), "0.13.9");
    }
}
