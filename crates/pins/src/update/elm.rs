//! `update elm`: pin the npm packages that carry the Elm compiler binary.
//!
//! Elm publishes no compiler release assets we could anchor directly. The
//! `elm` launcher package has one optional dependency per platform, and those
//! `@elm_binaries/*` packages hold the executable. As with tsrs, only registry
//! metadata is downloaded: the tarball URL and its sha512 integrity come from
//! the registry, and the integrity is trusted only after npm's ECDSA signature
//! over `<name>@<version>:<integrity>` checks out against a pinned key.
//!
//! npm wraps the compiler as `elm@0.19.3-0` while elm.json and the compiler's
//! `--version` say `0.19.3`. Pins are stored in the elm.json form; the `-0`
//! packaging suffix is stripped on the way in and restored when asking the
//! registry about a release.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{json, Fetch, Plan};
use crate::http::Http;
use crate::npm;
use crate::source::{self, Source};

/// Mirrored by `lib/elm/toolchain.nix` for its `hashes` escape hatch.
pub(crate) const REGISTRY: &str = "https://registry.npmjs.org";

/// The launcher package whose optional dependencies name the binaries.
pub(crate) const LAUNCHER: &str = "elm";

/// The suffix npm adds to a compiler version to make the launcher release.
/// elm.json never carries it, so it is stripped before a version is pinned.
const NPM_SUFFIX: &str = "-0";

/// Nix system -> the npm package holding that system's executable, in
/// `lib/common/systems.nix` order. The Linux packages are the glibc builds.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "@elm_binaries/linux_x64"),
    ("aarch64-linux", "@elm_binaries/linux_arm64"),
    ("aarch64-darwin", "@elm_binaries/darwin_arm64"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// The parts of the `elm` packument this reads.
#[derive(Debug, Deserialize)]
struct Packument {
    #[serde(rename = "dist-tags")]
    dist_tags: IndexMap<String, String>,
    versions: IndexMap<String, Manifest>,
    /// Publish times, needed only when a signature is by a retired npm key.
    #[serde(default)]
    time: IndexMap<String, String>,
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

/// The pinned format of `data/elm/elm.json`: the elm.json-form compiler
/// version -> the release and its per-system sources.
#[derive(Debug, Serialize)]
struct Pins(IndexMap<String, Pin>);

#[derive(Debug, Serialize)]
struct Pin {
    version: String,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/elm/elm.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let packument: Packument = get(http, &format!("{REGISTRY}/{LAUNCHER}"))?;
    let pins = pins(http, &packument)?;
    let version = pins
        .0
        .keys()
        .next()
        .context("crucible: the elm packument has no release to pin")?;
    println!("elm: {version}");
    Ok(vec![(root.join("data/elm/elm.json"), json(&pins)?)])
}

fn pins(http: &Http, packument: &Packument) -> Result<Pins> {
    let launcher = latest(packument)?;
    let compiler = compiler_version(&launcher.version)?.to_string();
    let platforms = sources(http, packument, launcher)?;
    let mut map = IndexMap::new();
    map.insert(
        compiler.clone(),
        Pin {
            version: compiler,
            platforms,
        },
    );
    Ok(Pins(map))
}

/// The verified sources of an arbitrary compiler release, for `pin elm`.
/// Both `0.19.3` and the npm form `0.19.3-0` name the same release; the
/// packument decides, rather than assuming every release uses `-0`.
pub(crate) fn pin_sources(
    http: &Http,
    version: &str,
) -> Result<(String, IndexMap<String, Source>)> {
    let packument: Packument = get(http, &format!("{REGISTRY}/{LAUNCHER}"))?;
    let npm_version = packument
        .versions
        .keys()
        .find(|candidate| {
            *candidate == version || compiler_version(candidate).ok() == Some(version)
        })
        .with_context(|| {
            format!("crucible: elm has no compiler release {version}; check {REGISTRY}/{LAUNCHER}")
        })?
        .clone();
    let launcher = packument
        .versions
        .get(&npm_version)
        .context("crucible: the elm packument lost a version it just listed")?;
    let platforms = sources(http, &packument, launcher)?;
    Ok((compiler_version(&npm_version)?.to_string(), platforms))
}

fn get<F: Fetch, T: for<'de> Deserialize<'de>>(http: &F, url: &str) -> Result<T> {
    serde_json::from_slice(&http.get(url)?).with_context(|| format!("failed to parse {url}"))
}

/// Re-verify each pinned platform package's npm registry signature over the
/// integrity in `data/`. The pinned compiler version selects the release from
/// the packument; the registry's `latest` tag is deliberately not consulted,
/// so a newer release cannot invalidate this pin.
pub(crate) fn verify<F: Fetch>(http: &F, root: &Path) -> Result<()> {
    let pins = source::read_json(root, "data/elm/elm.json")?;
    let packument: Packument = get(http, &format!("{REGISTRY}/{LAUNCHER}"))?;
    let mut failures = Vec::new();
    for (compiler, pin) in pins
        .as_object()
        .context("data/elm/elm.json is not an object")?
    {
        let npm_version = match packument
            .versions
            .keys()
            .find(|candidate| compiler_version(candidate).ok() == Some(compiler.as_str()))
        {
            Some(version) => version,
            None => {
                failures.push(format!(
                    "elm {compiler}: no npm release ending in {NPM_SUFFIX} for this compiler"
                ));
                continue;
            }
        };
        let published = packument.time.get(npm_version).map(String::as_str);
        for (system, package) in PLATFORMS {
            let url = format!("{REGISTRY}/{}/{}", package.replace('/', "%2F"), npm_version);
            let manifest: Manifest = match get(http, &url) {
                Ok(manifest) => manifest,
                Err(err) => {
                    failures.push(format!("elm {compiler} {system}: {err:#}"));
                    continue;
                }
            };
            let expected =
                match npm::checked_integrity(package, npm_version, &manifest.dist, published) {
                    Ok(integrity) => integrity.to_string(),
                    Err(err) => {
                        failures.push(format!("elm {compiler} {system}: {err:#}"));
                        continue;
                    }
                };
            let pinned = match source::platform_hash(pin, system) {
                Ok(pinned) => pinned,
                Err(err) => {
                    failures.push(format!("elm {compiler} {system}: {err:#}"));
                    continue;
                }
            };
            if pinned != expected {
                failures.push(format!(
                    "elm {compiler} {system}: pinned {pinned}, signed registry integrity is {expected}"
                ));
            }
            let pinned_url = pin
                .get("platforms")
                .and_then(|platforms| platforms.get(system))
                .and_then(|source| source.get("url"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if pinned_url != manifest.dist.tarball {
                failures.push(format!(
                    "elm {compiler} {system}: pinned {pinned_url}, registry serves {}",
                    manifest.dist.tarball
                ));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

/// The manifest of the version npm's `latest` tag names.
fn latest(packument: &Packument) -> Result<&Manifest> {
    let version = packument
        .dist_tags
        .get("latest")
        .context("the elm packument has no latest tag")?;
    packument.versions.get(version).with_context(|| {
        format!("the elm packument tags {version} as latest but lists no such version")
    })
}

/// npm's `elm@<compiler>-0` packaging suffix; elm.json stores `<compiler>`.
/// A version without the suffix is not an elm.json form and is rejected
/// rather than stored under a key the compiler would never ask for.
fn compiler_version(npm_version: &str) -> Result<&str> {
    npm_version.strip_suffix(NPM_SUFFIX).with_context(|| {
        format!("crucible: elm npm version {npm_version} has no {NPM_SUFFIX} suffix to strip")
    })
}

/// The launcher must install exactly this release's executable for every
/// system crucible builds for; a missing or mismatched optional dependency
/// means a half-published release or renamed platform packages.
fn expect_dependencies(launcher: &Manifest) -> Result<()> {
    for (_, package) in PLATFORMS {
        match launcher.optional_dependencies.get(package) {
            Some(version) if *version == launcher.version => {}
            Some(version) => bail!(
                "elm {} installs {package}@{version}, not the same release",
                launcher.version
            ),
            None => bail!(
                "elm {} has no {package} optional dependency; check whether the platform packages were renamed",
                launcher.version
            ),
        }
    }
    Ok(())
}

/// Every platform's tarball, verified and ready to pin.
fn sources(
    http: &Http,
    packument: &Packument,
    launcher: &Manifest,
) -> Result<IndexMap<String, Source>> {
    let npm_version = &launcher.version;
    expect_dependencies(launcher)?;
    let published = packument.time.get(npm_version).map(String::as_str);
    let mut platforms = IndexMap::new();
    for (system, package) in PLATFORMS {
        let manifest: Manifest = get(
            http,
            &format!("{REGISTRY}/{}/{}", package.replace('/', "%2F"), npm_version),
        )?;
        platforms.insert(
            system.to_string(),
            source(package, npm_version, &manifest, published)?,
        );
    }
    Ok(platforms)
}

/// One platform's tarball, by the sha512 integrity npm signed over it.
fn source(
    package: &str,
    npm_version: &str,
    manifest: &Manifest,
    published: Option<&str>,
) -> Result<Source> {
    if manifest.version != npm_version {
        bail!(
            "crucible: {package} {npm_version} reports version {}",
            manifest.version
        );
    }
    Ok(Source {
        url: manifest.dist.tarball.clone(),
        hash: npm::checked_integrity(package, npm_version, &manifest.dist, published)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, integrity: &str, deps: &[(&str, &str)]) -> Manifest {
        Manifest {
            version: version.to_string(),
            optional_dependencies: deps
                .iter()
                .map(|(package, version)| (package.to_string(), version.to_string()))
                .collect(),
            dist: npm::Dist {
                integrity: integrity.to_string(),
                tarball: "https://example.test/x.tgz".to_string(),
                signatures: Vec::new(),
            },
        }
    }

    #[test]
    fn maps_the_npm_version_to_the_elm_json_version() {
        assert_eq!(compiler_version("0.19.3-0").unwrap(), "0.19.3");
        assert!(compiler_version("0.19.3").is_err(), "no suffix to strip");
    }

    #[test]
    fn requires_all_three_platform_dependencies() {
        let deps: Vec<(&str, &str)> = PLATFORMS
            .iter()
            .map(|(_, package)| (*package, "0.19.3-0"))
            .collect();
        let launcher = manifest("0.19.3-0", "sha512-x", &deps);
        assert!(expect_dependencies(&launcher).is_ok());
    }

    #[test]
    fn rejects_a_missing_or_mismatched_platform_dependency() {
        let mut deps: Vec<(&str, &str)> = PLATFORMS
            .iter()
            .map(|(_, package)| (*package, "0.19.3-0"))
            .collect();
        let missing = deps.pop().unwrap();
        let launcher = manifest("0.19.3-0", "sha512-x", &deps);
        let err = expect_dependencies(&launcher).unwrap_err().to_string();
        assert!(err.contains(missing.0), "{err}");

        deps.push((missing.0, "0.19.3-1"));
        let launcher = manifest("0.19.3-0", "sha512-x", &deps);
        let err = expect_dependencies(&launcher).unwrap_err().to_string();
        assert!(err.contains("not the same release"), "{err}");
    }

    #[test]
    fn rejects_an_integrity_that_is_not_sha512() {
        let manifest = manifest("0.19.3-0", "sha1-deadbeef", &[]);
        let err = source("@elm_binaries/linux_x64", "0.19.3-0", &manifest, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("expected a sha512"), "{err}");
    }

    // The real `@elm_binaries/linux_x64@0.19.3-0` registry signature, so the
    // verify path is exercised with the package this updater actually pins.
    const SIGNED_INTEGRITY: &str = "sha512-x+WXNkRZUOuyGPdcRo137rNW1+7KBVXLf2L9oul21tj3MKpE7HC7Kl5VpyWj859YLdYv9M42S2O6YiO9t+MJeQ==";
    const SIGNED_KEY: &str = "SHA256:DhQ8wR5APBvFHLF/+Tc+AYvPOdTpcIDqOhxsBHRwC7U";
    const SIGNED_SIGS: [&str; 2] = [
        "MEQCIBK+Ocff1tZnEYtkuG8k8LyKOebZlJdmPQXi3zT4P9aYAiA91/D4IxqZevUnoZxvhNmtZOx7GTP4VGJlk2VQ8tfZGA==",
        "MEUCIBmQyZMDHiyNZ2CTJQksNDDsMUUGH+xMg2wdBoVGCn1PAiEAuKaAQ3bnx99Ef777CehSlFCd2zujq/UiBjKjoD2VhDQ=",
    ];

    fn signed() -> Manifest {
        let mut manifest = manifest("0.19.3-0", SIGNED_INTEGRITY, &[]);
        manifest.dist.signatures = SIGNED_SIGS
            .iter()
            .map(|sig| npm::Signature {
                keyid: SIGNED_KEY.to_string(),
                sig: sig.to_string(),
            })
            .collect();
        manifest
    }

    #[test]
    fn accepts_a_real_signature_through_source() {
        let source = source("@elm_binaries/linux_x64", "0.19.3-0", &signed(), None).unwrap();
        assert_eq!(source.hash, SIGNED_INTEGRITY);
        assert_eq!(source.url, "https://example.test/x.tgz");
    }

    #[test]
    fn rejects_a_signature_over_another_version() {
        assert!(source("@elm_binaries/linux_x64", "0.19.3-1", &signed(), None,).is_err());
    }
}
