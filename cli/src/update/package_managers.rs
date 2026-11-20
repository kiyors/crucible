//! `update node` also pins exact package-manager releases from npm's registry.
//!
//! The native pnpm and bun packages ship one tarball per system and the yarn
//! packages one JavaScript tarball for every system. A registry packument lists
//! every release with its tarball URL and sha512 integrity, so only metadata is
//! downloaded here: the Nix side fetches and unpacks the tarballs with those
//! hashes.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{json, Plan};
use crate::http::Http;

const REGISTRY: &str = "https://registry.npmjs.org";

/// Nix system -> npm package holding that system's native binary, in
/// `lib/common/systems.nix` order.
const PNPM_PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "@pnpm%2Fexe.linux-x64"),
    ("aarch64-linux", "@pnpm%2Fexe.linux-arm64"),
    ("aarch64-darwin", "@pnpm%2Fexe.darwin-arm64"),
];
const BUN_PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "@oven%2Fbun-linux-x64"),
    ("aarch64-linux", "@oven%2Fbun-linux-aarch64"),
    ("aarch64-darwin", "@oven%2Fbun-darwin-aarch64"),
];
/// Yarn runs on Node.js, so one tarball covers every system. 1 is classic, 4
/// is Berry (`@yarnpkg/cli-dist`); both are installed by the same Nix module.
const YARN_MAJORS: [(&str, &str); 2] = [("1", "yarn"), ("4", "@yarnpkg%2Fcli-dist")];

/// The majors whose newest release is pinned. These are the ones nixpkgs
/// carries today; anything else keeps the nixpkgs fallback in toolchain.nix.
const PNPM_MAJORS: [&str; 1] = ["12"];
const BUN_MAJORS: [&str; 1] = ["1"];

/// A major held at an exact release instead of following its newest one. cove
/// needs pnpm 12.21.0 specifically, and 12 has since moved on, so the pin
/// stays put even as newer 12.x releases appear.
const HELD: [(&str, &str, &str); 1] = [("pnpm", "12", "12.21.0")];

#[derive(Debug, Deserialize)]
struct Packument {
    versions: IndexMap<String, PackumentVersion>,
}

#[derive(Debug, Deserialize)]
struct PackumentVersion {
    dist: Dist,
}

#[derive(Debug, Deserialize)]
struct Dist {
    integrity: String,
    tarball: String,
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
}

#[derive(Debug, Serialize)]
struct JsPin {
    version: String,
    url: String,
    hash: String,
}

#[derive(Debug, Serialize)]
struct Source {
    url: String,
    hash: String,
}

/// Fetch and validate `data/node/package-managers.json`, without writing it.
pub fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let pins = Pins {
        pnpm: native_pins(http, "pnpm", &PNPM_PLATFORMS, &PNPM_MAJORS)?,
        bun: native_pins(http, "bun", &BUN_PLATFORMS, &BUN_MAJORS)?,
        yarn: js_pins(http, "yarn", &YARN_MAJORS)?,
    };
    Ok(vec![(
        root.join("data/node/package-managers.json"),
        json(&pins)?,
    )])
}

/// One native package manager: the newest stable release of each pinned major,
/// with that same version's tarball and integrity for every supported system.
fn native_pins(
    http: &Http,
    name: &str,
    platforms: &[(&str, &str)],
    majors: &[&str],
) -> Result<IndexMap<String, NativePin>> {
    let mut pins = IndexMap::new();
    for major in majors {
        let mut version: Option<String> = None;
        let mut sources = IndexMap::new();
        for (system, package) in platforms {
            let packument = fetch(http, package)?;
            let (found, dist) = pick(&packument, name, major, package)?;
            match &version {
                None => {
                    println!("{name} {major}: {found}");
                    version = Some(found.to_string());
                }
                // A release is only usable once every system ships it; a
                // half-published rollout would build on one host and fail on
                // another.
                Some(expected) if expected.as_str() != found => bail!(
                    "{name} {major}: {system} has {found}, but the other systems have {expected}"
                ),
                _ => {}
            }
            sources.insert(
                system.to_string(),
                Source {
                    url: dist.tarball.clone(),
                    hash: integrity(package, found, dist)?,
                },
            );
        }
        pins.insert(
            major.to_string(),
            NativePin {
                version: version.context("no systems configured")?,
                platforms: sources,
            },
        );
    }
    Ok(pins)
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
                hash: integrity(package, version, dist)?,
            },
        );
    }
    Ok(pins)
}

fn fetch(http: &Http, package: &str) -> Result<Packument> {
    let url = format!("{REGISTRY}/{package}");
    serde_json::from_slice(&http.get(&url)?).with_context(|| format!("failed to parse {url}"))
}

/// npm publishes SRI sha512 here; say so explicitly rather than waiting for a
/// fetchurl with an unexpected algorithm to fail in the Nix build.
fn integrity(package: &str, version: &str, dist: &Dist) -> Result<String> {
    if !dist.integrity.starts_with("sha512-") {
        bail!(
            "{package} {version} has integrity {}, expected a sha512- SRI hash",
            dist.integrity
        );
    }
    Ok(dist.integrity.clone())
}

/// The release to pin: an exact held version when one is declared, otherwise
/// the newest stable release of the major.
fn pick<'a>(
    packument: &'a Packument,
    name: &str,
    major: &str,
    package: &str,
) -> Result<(&'a str, &'a Dist)> {
    let held = HELD
        .iter()
        .find(|(n, m, _)| *n == name && *m == major)
        .map(|(_, _, version)| *version);
    if let Some(version) = held {
        let entry = packument
            .versions
            .get(version)
            .with_context(|| format!("{package} has no release {version}"))?;
        return Ok((version, &entry.dist));
    }
    newest(packument, major).with_context(|| format!("{package} has no stable v{major} release"))
}

/// The newest non-prerelease version of `major`, with its dist metadata.
fn newest<'a>(packument: &'a Packument, major: &str) -> Option<(&'a str, &'a Dist)> {
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
        let dist = Dist {
            integrity: "sha1-deadbeef".to_string(),
            tarball: "x.tgz".to_string(),
        };
        assert!(integrity("pnpm", "1.0.0", &dist).is_err());
    }
}
