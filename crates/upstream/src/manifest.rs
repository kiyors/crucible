//! A channel manifest from `static.rust-lang.org`, and the subset of it the
//! flake consumes.
//!
//! Package and target order is kept from the manifest: `data/rust/*.json` is a
//! human-readable pin, and reordering it on every update would bury the real
//! change in noise.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// The host triples of `lib/common/systems.nix`, in the same order.
pub const HOSTS: [&str; 3] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
];

// The literal stays because the source sync test reads it as text; the
// canonical table in `platforms` is what a compiler checks it against.
const _: () = assert!(crate::platforms::same(&HOSTS, &crate::platforms::hosts()));

/// One component of an upstream `channel-rust-*.toml`. Only `pkg` and its
/// versions are read outside this module (by `pin rust` and the updater);
/// everything else is internal to `trim` and `missing`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub pkg: IndexMap<String, Package>,
    #[serde(default)]
    pub profiles: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub renames: IndexMap<String, Rename>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Package {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    target: IndexMap<String, Target>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Target {
    #[serde(default)]
    available: bool,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    xz_url: Option<String>,
    #[serde(default)]
    xz_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rename {
    to: String,
}

/// The pinned format of `data/rust/*.json`. `serde` reads every field here, so
/// none of them need to be visible outside this module.
#[derive(Clone, Debug, Serialize)]
pub struct Trimmed {
    pub date: String,
    pub version: String,
    profiles: IndexMap<String, Vec<String>>,
    renames: IndexMap<String, Rename>,
    pkg: IndexMap<String, TrimmedPackage>,
}

#[derive(Clone, Debug, Serialize)]
struct TrimmedPackage {
    version: String,
    target: IndexMap<String, Artifact>,
}

#[derive(Clone, Debug, Serialize)]
struct Artifact {
    url: Option<String>,
    hash: Option<String>,
}

impl Manifest {
    /// Keep what a toolchain on one of our hosts can use: every component for a
    /// host triple, `rust-src` (target `"*"`), and `rust-std` for every target
    /// so cross builds work. Only xz tarballs, and only what is available.
    pub fn trim(&self) -> Trimmed {
        let mut pkg = IndexMap::new();
        for (name, package) in &self.pkg {
            // Whole-toolchain and attestation bundles; nothing installs them.
            if name == "rust" || name == "reproducible-artifacts" {
                continue;
            }

            let mut target = IndexMap::new();
            for (triple, src) in &package.target {
                let wanted =
                    name == "rust-std" || triple == "*" || HOSTS.contains(&triple.as_str());
                if !wanted || !src.available {
                    continue;
                }
                target.insert(
                    triple.clone(),
                    Artifact {
                        url: src.xz_url.clone().or_else(|| src.url.clone()),
                        hash: src.xz_hash.clone().or_else(|| src.hash.clone()),
                    },
                );
            }

            if !target.is_empty() {
                pkg.insert(
                    name.clone(),
                    TrimmedPackage {
                        version: package.version.clone(),
                        target,
                    },
                );
            }
        }

        Trimmed {
            date: self.date.clone(),
            // `importManifest` in lib/rust/toolchain.nix wants just the number, but
            // upstream packs the commit and date into the same string.
            version: self
                .pkg
                .get("rustc")
                .map(|pkg| {
                    pkg.version
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_default(),
            profiles: self.profiles.clone(),
            renames: self.renames.clone(),
            pkg,
        }
    }

    /// Which required components are missing on at least one host, as
    /// `component/host` pairs. A target of `"*"` counts for every host.
    pub fn missing(&self, required: &[&str]) -> Vec<String> {
        let mut missing = Vec::new();
        for name in required {
            for host in HOSTS {
                let available = self
                    .pkg
                    .get(*name)
                    .and_then(|pkg| pkg.target.get(host).or_else(|| pkg.target.get("*")))
                    .is_some_and(|src| src.available);
                if !available {
                    missing.push(format!("{name}/{host}"));
                }
            }
        }
        missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> Manifest {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn nightly_completeness_counts_star_and_hosts() {
        let manifest = manifest(include_str!("../tests/fixtures/channel.toml"));
        // rustc lists every host directly, rust-src only has "*", which counts
        // for all of them.
        assert!(manifest.missing(&["rustc", "rust-src"]).is_empty());

        // cargo only ships one host, so the other two are incomplete.
        assert_eq!(
            manifest.missing(&["cargo"]),
            [
                "cargo/x86_64-unknown-linux-gnu",
                "cargo/aarch64-unknown-linux-gnu",
            ]
        );

        // A pkg that is absent entirely is missing on every host.
        assert_eq!(
            manifest.missing(&["clippy-preview"]),
            [
                "clippy-preview/x86_64-unknown-linux-gnu",
                "clippy-preview/aarch64-unknown-linux-gnu",
                "clippy-preview/aarch64-apple-darwin",
            ]
        );

        // rust-std exists for aarch64-linux but is marked unavailable there, so
        // rust-std's target does not fall back to some other host. It also has
        // no aarch64-darwin tarball in this fixture.
        assert_eq!(
            manifest.missing(&["rust-std"]),
            [
                "rust-std/aarch64-unknown-linux-gnu",
                "rust-std/aarch64-apple-darwin",
            ]
        );
    }

    #[test]
    fn unavailable_targets_are_missing() {
        let mut manifest = manifest(include_str!("../tests/fixtures/channel.toml"));
        let rustc = manifest.pkg.get_mut("rustc").unwrap();
        rustc
            .target
            .get_mut("x86_64-unknown-linux-gnu")
            .unwrap()
            .available = false;
        assert_eq!(
            manifest.missing(&["rustc"]),
            ["rustc/x86_64-unknown-linux-gnu"]
        );
    }
}
