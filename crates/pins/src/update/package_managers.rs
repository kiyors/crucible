//! `update node` pins exact package-manager releases: pnpm and bun from their
//! GitHub releases, yarn from npm's registry.
//!
//! pnpm publishes one self-contained archive per system (10 a raw executable,
//! 11 and 12 a tarball holding the binary and its `dist/` tree), and GitHub
//! lists each asset's sha256. That digest is checked against pnpm's GitHub
//! artifact attestation (see `upstream::attestation`) before it is pinned, so
//! the npm registry is not needed for pnpm at all. bun's release page lists
//! its zips with their sha256 in `SHASUMS256.txt`. A registry packument lists
//! yarn's releases with their URL and sha512 integrity, checked against the
//! registry's signature (see `upstream::npm`), as pnpm's own installer does.
//! Only metadata is downloaded here; the Nix side fetches the archives.

use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{json, parse_sha256sums, Plan};
use crate::http::Http;
use crate::source::{hash_of, Source};
use crate::{attestation, npm, sri};

const REGISTRY: &str = "https://registry.npmjs.org";
const PNPM_RELEASES: &str = "https://api.github.com/repos/pnpm/pnpm/releases?per_page=100";
const BUN_RELEASES: &str = "https://api.github.com/repos/oven-sh/bun/releases?per_page=100";
const BUN_RELEASES_DOWNLOAD: &str = "https://github.com/oven-sh/bun/releases/download";

/// One pnpm line: its major, how the asset is laid out, and the release asset
/// holding each system's build, in `lib/common/systems.nix` order. 10 ships a
/// single executable; 11 and 12 ship a tarball with the binary and `dist/`.
pub(crate) struct PnpmLine {
    pub(crate) major: &'static str,
    layout: &'static str,
    pub(crate) platforms: [(&'static str, &'static str); 3],
}

/// The pnpm lines upstream still releases, with their GitHub asset names.
pub(crate) const PNPM: [PnpmLine; 3] = [
    PnpmLine {
        major: "10",
        layout: "binary",
        // The dynamically linked assets are `pkg` executables: patchelf
        // corrupts the JS snapshot appended to them, so Linux uses the
        // static ones, which need no patching. macOS is Mach-O.
        platforms: [
            ("x86_64-linux", "pnpm-linuxstatic-x64"),
            ("aarch64-linux", "pnpm-linuxstatic-arm64"),
            ("aarch64-darwin", "pnpm-macos-arm64"),
        ],
    },
    PnpmLine {
        major: "11",
        layout: "tarball",
        platforms: [
            ("x86_64-linux", "pnpm-linux-x64.tar.gz"),
            ("aarch64-linux", "pnpm-linux-arm64.tar.gz"),
            ("aarch64-darwin", "pnpm-darwin-arm64.tar.gz"),
        ],
    },
    PnpmLine {
        major: "12",
        layout: "tarball",
        platforms: [
            ("x86_64-linux", "pnpm-linux-x64.tar.gz"),
            ("aarch64-linux", "pnpm-linux-arm64.tar.gz"),
            ("aarch64-darwin", "pnpm-darwin-arm64.tar.gz"),
        ],
    },
];
/// Nix system -> bun release zip, in `lib/common/systems.nix` order.
pub(crate) const BUN_ASSETS: [(&str, &str); 3] = [
    ("x86_64-linux", "bun-linux-x64.zip"),
    ("aarch64-linux", "bun-linux-aarch64.zip"),
    ("aarch64-darwin", "bun-darwin-aarch64.zip"),
];

// The literals stay for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks them against.
const _: () = assert!(
    crate::platforms::matches(BUN_ASSETS)
        && crate::platforms::matches(PNPM[0].platforms)
        && crate::platforms::matches(PNPM[1].platforms)
        && crate::platforms::matches(PNPM[2].platforms)
);
/// Yarn runs on Node.js, so one tarball covers every system. 1 is classic, 4
/// is Berry (`@yarnpkg/cli-dist`); both are installed by the same Nix module.
const YARN_MAJORS: [(&str, &str); 2] = [("1", "yarn"), ("4", "@yarnpkg%2Fcli-dist")];

/// The bun majors whose newest release is pinned; anything else keeps the
/// nixpkgs fallback in toolchain.nix.
const BUN_MAJORS: [&str; 1] = ["1"];

/// One GitHub release, as the releases API lists it.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<GithubAsset>,
}

/// One asset of a GitHub release, with the sha256 GitHub computed for it.
#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    /// `sha256:<hex>`, as GitHub reports it.
    digest: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct Packument {
    versions: IndexMap<String, PackumentVersion>,
    /// version -> publish time, for signatures by a retired npm key.
    #[serde(default)]
    time: IndexMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct PackumentVersion {
    dist: npm::Dist,
}

/// The pinned format of `data/node/package-managers.json`: name -> major -> pin.
#[derive(Debug, Serialize)]
struct Pins {
    pnpm: IndexMap<String, NativePin>,
    yarn: IndexMap<String, JsPin>,
    bun: IndexMap<String, NativePin>,
}

#[derive(Debug, Serialize)]
struct NativePin {
    version: String,
    platforms: IndexMap<String, Source>,
    /// How the archive is laid out: "binary" (one raw executable) or
    /// "tarball" (binary and `dist/`). Only pnpm sets this; bun's zip is
    /// handled by its name in the Nix module.
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsPin {
    version: String,
    url: String,
    hash: String,
}

/// Fetch and validate `data/node/package-managers.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let pins = Pins {
        pnpm: pnpm_pins(http)?,
        bun: bun_pins(http)?,
        yarn: js_pins(http, "yarn", &YARN_MAJORS)?,
    };
    Ok(vec![(
        root.join("data/node/package-managers.json"),
        json(&pins)?,
    )])
}

/// Every pnpm line: the newest stable release of each major, with that same
/// version's asset for every supported system. A release's assets are listed
/// together, so a missing one fails this run instead of building on some
/// hosts and not others.
fn pnpm_pins(http: &Http) -> Result<IndexMap<String, NativePin>> {
    let releases: Vec<GithubRelease> = serde_json::from_slice(&http.get(PNPM_RELEASES)?)
        .context("failed to parse the pnpm release list")?;
    let mut pins = IndexMap::new();
    for PnpmLine {
        major,
        layout,
        platforms,
    } in &PNPM
    {
        let release = newest_pnpm(&releases, major)?;
        let tag = &release.tag_name;
        let version = tag.trim_start_matches('v').to_string();
        println!("pnpm {major}: {version}");
        let mut sources = IndexMap::new();
        for (system, asset) in platforms {
            let found = release
                .assets
                .iter()
                .find(|candidate| &candidate.name == asset)
                .with_context(|| format!("{tag} has no {asset}"))?;
            let hex = found.digest.strip_prefix("sha256:").with_context(|| {
                format!(
                    "{tag}'s {asset} has digest {}, expected sha256:",
                    found.digest
                )
            })?;
            attestation::verify(http, hex, asset, tag)?;
            sources.insert(
                system.to_string(),
                Source {
                    url: found.browser_download_url.clone(),
                    hash: sri::from_hex(hex)?,
                },
            );
        }
        pins.insert(
            major.to_string(),
            NativePin {
                version,
                platforms: sources,
                layout: Some((*layout).to_string()),
            },
        );
    }
    Ok(pins)
}

/// The newest stable pnpm release of `major`: the first of that major, newest
/// first, that is neither a draft nor a prerelease.
fn newest_pnpm<'a>(releases: &'a [GithubRelease], major: &str) -> Result<&'a GithubRelease> {
    let prefix = format!("v{major}.");
    releases
        .iter()
        .find(|release| {
            !release.draft
                && !release.prerelease
                && release.tag_name.strip_prefix(&prefix).is_some_and(|rest| {
                    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
                })
        })
        .with_context(|| format!("GitHub has no stable pnpm {major} release"))
}

/// One bun major: the newest stable release of that major, with that same
/// version's zip and digest for every supported system, from the release's
/// `SHASUMS256.txt`. bun is not an npm package: it ships one zip per
/// platform, and GitHub lists them with their sha256.
fn bun_pins(http: &Http) -> Result<IndexMap<String, NativePin>> {
    let releases: Vec<BunRelease> = serde_json::from_slice(&http.get(BUN_RELEASES)?)
        .context("failed to parse the bun release list")?;
    let mut pins = IndexMap::new();
    for major in BUN_MAJORS {
        let release = newest_stable(&releases, major)?;
        let tag = release.tag_name.clone();
        let version = tag.trim_start_matches("bun-v").to_string();
        let sums = parse_sha256sums(&shasums(http, &tag)?);
        let mut platforms = IndexMap::new();
        for (system, asset) in BUN_ASSETS {
            platforms.insert(
                system.to_string(),
                Source {
                    url: format!("{BUN_RELEASES_DOWNLOAD}/{tag}/{asset}"),
                    hash: hash_of(&sums, asset, &format!("{tag}'s SHASUMS256.txt"))?,
                },
            );
        }
        println!("bun {major}: {version}");
        pins.insert(
            major.to_string(),
            NativePin {
                version,
                platforms,
                layout: None,
            },
        );
    }
    Ok(pins)
}

/// One bun release, as the release list API reports it.
#[derive(Debug, Deserialize)]
struct BunRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
}

/// The newest stable bun release of `major`: the first of that major,
/// newest first, that is neither a draft nor a prerelease.
fn newest_stable<'a>(releases: &'a [BunRelease], major: &str) -> Result<&'a BunRelease> {
    let prefix = format!("bun-v{major}.");
    releases
        .iter()
        .find(|release| {
            !release.draft && !release.prerelease && release.tag_name.starts_with(&prefix)
        })
        .with_context(|| format!("GitHub has no stable bun {major} release"))
}

/// `bun`'s `SHASUMS256.txt`, as the release page publishes it.
fn shasums(http: &Http, tag: &str) -> Result<String> {
    let url = format!("{BUN_RELEASES_DOWNLOAD}/{tag}/SHASUMS256.txt");
    let body = http
        .get(&url)
        .with_context(|| format!("GET {url} failed"))?;
    String::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))
}

/// One JavaScript package manager: the newest stable release of each major,
/// with its single tarball and integrity.
fn js_pins(http: &Http, name: &str, packages: &[(&str, &str)]) -> Result<IndexMap<String, JsPin>> {
    let mut pins = IndexMap::new();
    for (major, package) in packages {
        let packument = fetch(http, package)?;
        let (version, dist) = newest(&packument, major)
            .with_context(|| format!("{package} has no stable v{major} release"))?;
        println!("{name} {major}: {version}");
        pins.insert(
            major.to_string(),
            JsPin {
                version: version.to_string(),
                url: dist.tarball.clone(),
                hash: npm::checked_integrity(
                    package,
                    version,
                    dist,
                    packument.time.get(version).map(String::as_str),
                )?
                .to_string(),
            },
        );
    }
    Ok(pins)
}

fn fetch(http: &Http, package: &str) -> Result<Packument> {
    let url = format!("{REGISTRY}/{package}");
    serde_json::from_slice(&http.get(&url)?).with_context(|| format!("failed to parse {url}"))
}

/// The newest non-prerelease version of `major`, with its dist metadata.
fn newest<'a>(packument: &'a Packument, major: &str) -> Option<(&'a str, &'a npm::Dist)> {
    packument
        .versions
        .iter()
        .filter(|(version, _)| {
            let version = version.as_str();
            !version.contains('-') && version.split('.').next() == Some(major)
        })
        .max_by_key(|(version, _)| version_key(version.as_str()))
        .map(|(version, entry)| (version.as_str(), &entry.dist))
}

/// The numeric parts of a semver, for ordering releases of the same major.
fn version_key(version: &str) -> (u64, u64, u64) {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts = core.split('.');
    let mut next = || parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (next(), next(), next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packument(json: &str) -> Packument {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn picks_the_newest_stable_pnpm_release_of_a_major() {
        let releases: Vec<GithubRelease> = serde_json::from_str(
            r#"[
              { "tag_name": "v12.23.0", "prerelease": false, "draft": false, "assets": [] },
              { "tag_name": "v12.23.1-beta.1", "prerelease": true, "draft": false, "assets": [] },
              { "tag_name": "v11.29.0", "prerelease": false, "draft": false, "assets": [] },
              { "tag_name": "v12.24.0", "prerelease": false, "draft": true, "assets": [] },
              { "tag_name": "v12.22.0", "prerelease": false, "draft": false, "assets": [] }
            ]"#,
        )
        .unwrap();
        assert_eq!(newest_pnpm(&releases, "12").unwrap().tag_name, "v12.23.0");
        assert_eq!(newest_pnpm(&releases, "11").unwrap().tag_name, "v11.29.0");
        assert!(newest_pnpm(&releases, "10").is_err());
    }

    #[test]
    fn picks_the_newest_stable_release_of_a_major() {
        let p = packument(
            r#"{
              "versions": {
                "12.9.0": { "dist": { "integrity": "sha512-old", "tarball": "old.tgz" } },
                "12.21.0": { "dist": { "integrity": "sha512-new", "tarball": "new.tgz" } },
                "12.22.0-beta.1": { "dist": { "integrity": "sha512-beta", "tarball": "beta.tgz" } },
                "11.9.0": { "dist": { "integrity": "sha512-other", "tarball": "other.tgz" } }
              }
            }"#,
        );
        let (version, dist) = newest(&p, "12").unwrap();
        assert_eq!(version, "12.21.0");
        assert_eq!(dist.integrity, "sha512-new");
        assert!(newest(&p, "13").is_none());
    }

    #[test]
    fn rejects_an_integrity_that_is_not_sha512() {
        let dist = npm::Dist {
            integrity: "sha1-deadbeef".to_string(),
            tarball: "x.tgz".to_string(),
            signatures: Vec::new(),
        };
        assert!(npm::checked_integrity("pnpm", "1.0.0", &dist, None).is_err());
    }

    #[test]
    fn rejects_an_unsigned_integrity() {
        let dist = npm::Dist {
            integrity: "sha512-unsigned".to_string(),
            tarball: "x.tgz".to_string(),
            signatures: Vec::new(),
        };
        let err = npm::checked_integrity("pnpm", "1.0.0", &dist, None).unwrap_err();
        assert!(err.to_string().contains("no npm registry signature"));
    }

    // yarn 1.22.22's real registry signature, by the key retired in 2025 but
    // published while it was still current.
    const YARN_INTEGRITY: &str = "sha512-prL3kGtyG7o9Z9Sv8IPfBNrWTDmXB4Qbes8A9rEzt6wkJV8mUvoirjU0Mp3GGAU06Y0XQyA3/2/RQFVuK7MTfg==";
    const YARN_SIG: &str = "MEYCIQD8IYJ9/vVOO8pfUq+cj3OdBClW7fPnUosQ4TvgyO1XJAIhAOi8CE+iN39Td3aR4dJz2zLO6isnr+h44dxVeoYjDyta";
    const YARN_KEY: &str = "SHA256:jl3bwswu80PjjokCgh0o2w5c2U4LhQAE57gj9cz1kzA";
    const YARN_PUBLISHED: &str = "2024-03-09T21:35:40.868Z";

    #[test]
    fn accepts_yarns_real_signature_through_checked_integrity() {
        let dist = npm::Dist {
            integrity: YARN_INTEGRITY.to_string(),
            tarball: "yarn-1.22.22.tgz".to_string(),
            signatures: vec![npm::Signature {
                keyid: YARN_KEY.to_string(),
                sig: YARN_SIG.to_string(),
            }],
        };
        let hash = npm::checked_integrity("yarn", "1.22.22", &dist, Some(YARN_PUBLISHED)).unwrap();
        assert_eq!(hash, YARN_INTEGRITY);
        // The same retired-key signature is not trusted after its expiry.
        assert!(
            npm::checked_integrity("yarn", "1.22.22", &dist, Some("2026-01-01T00:00:00.000Z"))
                .is_err()
        );
    }
}
