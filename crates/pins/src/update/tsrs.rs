//! `update tsrs`: pin the release npm's `latest` tag points at, from the
//! per-platform `@tsrs/*` packages.
//!
//! tsrs publishes no GitHub release assets. The `tsrs` package is a Node.js
//! launcher whose optional dependencies hold one native executable per
//! platform, next to the `lib.*.d.ts` files it reads from beside itself. Those
//! platform packages need no Node.js, so they are what is pinned, by the
//! sha512 integrity the registry records for each tarball, once the
//! registry's signature over it checks out; only metadata is downloaded here.
//!
//! The tag, not the highest version, decides: tsrs versions are prereleases
//! such as `7.1.0-beta.tsrs4`, and SemVer compares `tsrs10` before `tsrs9`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{json, Plan};
use crate::http::Http;
use crate::npm;
use crate::source::Source;

const REGISTRY: &str = "https://registry.npmjs.org";

/// Nix system -> the npm package holding that system's executable, in
/// `lib/common/systems.nix` order. The Linux packages are the glibc builds.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "@tsrs/linux-x64-gnu"),
    ("aarch64-linux", "@tsrs/linux-arm64-gnu"),
    ("aarch64-darwin", "@tsrs/darwin-arm64"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// The parts of the `tsrs` packument this reads.
#[derive(Debug, Deserialize)]
struct Packument {
    #[serde(rename = "dist-tags")]
    dist_tags: IndexMap<String, String>,
    versions: IndexMap<String, Manifest>,
}

/// One version's manifest, from the packument or the registry's
/// `<package>/<version>` document.
#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: IndexMap<String, String>,
    dist: npm::Dist,
}

/// The pinned format of `data/tsrs/tsrs.json`.
#[derive(Debug, Serialize)]
struct Pins {
    version: String,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/tsrs/tsrs.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let packument: Packument = get(http, &format!("{REGISTRY}/tsrs"))?;
    let launcher = latest(&packument)?;
    let version = launcher.version.clone();
    println!("tsrs: {version}");

    let mut platforms = IndexMap::new();
    for (system, package) in PLATFORMS {
        expect_dependency(launcher, package)?;
        let manifest: Manifest = get(
            http,
            &format!("{REGISTRY}/{}/{version}", package.replace('/', "%2F")),
        )?;
        platforms.insert(system.to_string(), source(package, &version, &manifest)?);
    }
    Ok(vec![(
        root.join("data/tsrs/tsrs.json"),
        json(&Pins { version, platforms })?,
    )])
}

fn get<T: for<'de> Deserialize<'de>>(http: &Http, url: &str) -> Result<T> {
    serde_json::from_slice(&http.get(url)?).with_context(|| format!("failed to parse {url}"))
}

/// The manifest of the version npm's `latest` tag names.
fn latest(packument: &Packument) -> Result<&Manifest> {
    let version = packument
        .dist_tags
        .get("latest")
        .context("the tsrs packument has no latest tag")?;
    packument.versions.get(version).with_context(|| {
        format!("the tsrs packument tags {version} as latest but lists no such version")
    })
}

/// The launcher must install exactly this release's executable for the
/// platform; anything else means a half-published or renamed release.
fn expect_dependency(launcher: &Manifest, package: &str) -> Result<()> {
    match launcher.optional_dependencies.get(package) {
        Some(version) if *version == launcher.version => Ok(()),
        Some(version) => bail!(
            "tsrs {} installs {package}@{version}, not the same release",
            launcher.version
        ),
        None => bail!(
            "tsrs {} has no {package} optional dependency; check whether the platform packages were renamed",
            launcher.version
        ),
    }
}

/// One platform's tarball, by its registry integrity. npm publishes SRI
/// sha512 here; say so explicitly rather than waiting for a fetchurl with an
/// unexpected algorithm to fail in the Nix build.
fn source(package: &str, version: &str, manifest: &Manifest) -> Result<Source> {
    if manifest.version != version {
        bail!(
            "crucible: {package} {version} reports version {}",
            manifest.version
        );
    }
    Ok(Source {
        url: manifest.dist.tarball.clone(),
        // tsrs is newer than npm's key rotation, so no retired-key
        // signature needs its publish time.
        hash: npm::checked_integrity(package, version, &manifest.dist, None)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packument() -> Packument {
        serde_json::from_str(
            r#"{
              "dist-tags": { "latest": "7.1.0-beta.tsrs9" },
              "versions": {
                "7.1.0-beta.tsrs9": {
                  "version": "7.1.0-beta.tsrs9",
                  "optionalDependencies": {
                    "@tsrs/linux-x64-gnu": "7.1.0-beta.tsrs9",
                    "@tsrs/darwin-arm64": "7.1.0-beta.tsrs8"
                  },
                  "dist": { "integrity": "sha512-nine", "tarball": "nine.tgz" }
                },
                "7.1.0-beta.tsrs10": {
                  "version": "7.1.0-beta.tsrs10",
                  "dist": { "integrity": "sha512-ten", "tarball": "ten.tgz" }
                }
              }
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn follows_the_latest_tag_rather_than_semver() {
        assert_eq!(latest(&packument()).unwrap().version, "7.1.0-beta.tsrs9");
    }

    #[test]
    fn checks_the_launcher_installs_the_same_release() {
        let packument = packument();
        let launcher = latest(&packument).unwrap();
        assert!(expect_dependency(launcher, "@tsrs/linux-x64-gnu").is_ok());
        assert!(expect_dependency(launcher, "@tsrs/darwin-arm64").is_err());
        assert!(expect_dependency(launcher, "@tsrs/linux-arm64-gnu").is_err());
    }

    #[test]
    fn rejects_an_integrity_that_is_not_sha512() {
        let manifest = Manifest {
            version: "1.0.0".to_string(),
            optional_dependencies: IndexMap::new(),
            dist: npm::Dist {
                integrity: "sha1-deadbeef".to_string(),
                tarball: "x.tgz".to_string(),
                signatures: Vec::new(),
            },
        };
        assert!(source("@tsrs/darwin-arm64", "1.0.0", &manifest).is_err());
        assert!(source("@tsrs/darwin-arm64", "2.0.0", &manifest).is_err());
    }

    // A real npm signature (pnpm 11.29.0) exercises source()'s verify path;
    // tsrs's own signatures change with each release, so it is not pinned as
    // a fixture here. The helper itself is package-agnostic.
    const SIGNED_INTEGRITY: &str = "sha512-2RP5s3zV/meABS6Al3Ttq7e9z2mm5tm4/Ts4vAUvqKfYkbbCAaOPtQSgf/zi5ejCYu9U+49X22LyLTU6MLyKlg==";
    const SIGNED_SIG: &str = "MEYCIQCzLZw9XFDgpVrQ1WaKxkh7x0XkoSA2ipD/WIsexKzOEAIhAIHETzPKLjbep48f/J9O9tIC/hNhYzPMPxgBIgp3Zpfj";
    const SIGNED_KEY: &str = "SHA256:DhQ8wR5APBvFHLF/+Tc+AYvPOdTpcIDqOhxsBHRwC7U";

    #[test]
    fn accepts_a_real_signature_through_source() {
        let manifest = Manifest {
            version: "11.29.0".to_string(),
            optional_dependencies: IndexMap::new(),
            dist: npm::Dist {
                integrity: SIGNED_INTEGRITY.to_string(),
                tarball: "signed.tgz".to_string(),
                signatures: vec![npm::Signature {
                    keyid: SIGNED_KEY.to_string(),
                    sig: SIGNED_SIG.to_string(),
                }],
            },
        };
        let source = source("pnpm", "11.29.0", &manifest).unwrap();
        assert_eq!(source.hash, SIGNED_INTEGRITY);
        assert_eq!(source.url, "signed.tgz");
    }

    #[test]
    fn rejects_a_signature_over_another_version() {
        let manifest = Manifest {
            version: "11.29.0".to_string(),
            optional_dependencies: IndexMap::new(),
            dist: npm::Dist {
                integrity: SIGNED_INTEGRITY.to_string(),
                tarball: "signed.tgz".to_string(),
                signatures: vec![npm::Signature {
                    keyid: SIGNED_KEY.to_string(),
                    sig: SIGNED_SIG.to_string(),
                }],
            },
        };
        assert!(source("pnpm", "11.29.1", &manifest).is_err());
    }
}
