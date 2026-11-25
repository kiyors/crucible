//! Vite+'s `vp-install-lock.yaml`, a pnpm v9 lockfile, and the dependency
//! graph `data/node/vite-plus.json` keeps of it.
//!
//! Every package in the lockfile carries the registry's integrity hash, so the
//! flake fetches each tarball by that hash and lays out `node_modules` itself.
//! Installing through pnpm instead would need a fixed-output derivation, whose
//! hash can only be learned by building it once.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REGISTRY: &str = "https://registry.npmjs.org";

/// A lockfile platform, in npm's `os`/`cpu`/`libc` vocabulary. No caller
/// outside `upstream` reads these fields; the updater only needs `Graph`.
pub struct Platform {
    pub(crate) system: &'static str,
    pub(crate) os: &'static str,
    pub(crate) cpu: &'static str,
    /// `None` where npm has no libc to compare (macOS).
    pub(crate) libc: Option<&'static str>,
}

/// The systems of `lib/common/systems.nix`, in the same order.
pub const PLATFORMS: [Platform; 3] = [
    Platform {
        system: "x86_64-linux",
        os: "linux",
        cpu: "x64",
        libc: Some("glibc"),
    },
    Platform {
        system: "aarch64-linux",
        os: "linux",
        cpu: "arm64",
        libc: Some("glibc"),
    },
    Platform {
        system: "aarch64-darwin",
        os: "darwin",
        cpu: "arm64",
        libc: None,
    },
];

// The literal stays because the source sync test reads `system:` fields as
// text; the canonical table in `platforms` is what a compiler checks it against.
const _: () = assert!(crate::platforms::same(
    &[
        PLATFORMS[0].system,
        PLATFORMS[1].system,
        PLATFORMS[2].system,
    ],
    &crate::platforms::SYSTEMS
));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lockfile {
    pub lockfile_version: String,
    #[serde(default)]
    pub importers: IndexMap<String, Importer>,
    #[serde(default)]
    pub packages: IndexMap<String, Package>,
    #[serde(default)]
    pub snapshots: IndexMap<String, Snapshot>,
}

#[derive(Debug, Deserialize)]
pub struct Importer {
    #[serde(default)]
    pub dependencies: IndexMap<String, ImporterDependency>,
}

#[derive(Debug, Deserialize)]
pub struct ImporterDependency {
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub resolution: Resolution,
    #[serde(default)]
    pub engines: IndexMap<String, String>,
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub cpu: Vec<String>,
    #[serde(default)]
    pub libc: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Resolution {
    pub integrity: Option<String>,
    pub tarball: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub dependencies: IndexMap<String, String>,
    #[serde(default)]
    pub optional_dependencies: IndexMap<String, String>,
}

/// The pinned format: what `lib/node/vite-plus.nix` reads.
#[derive(Debug, Serialize)]
pub struct Graph {
    /// The directory of the root package.
    pub root: String,
    /// The root package's `engines`, so the flake can check a Node against it.
    pub engines: IndexMap<String, String>,
    /// Sorted by directory, so an update's diff shows what moved.
    pub packages: BTreeMap<String, Node>,
}

/// One package snapshot, installed at `node_modules/.pnpm/<id>/node_modules/<name>`.
#[derive(Debug, Serialize)]
pub struct Node {
    pub name: String,
    pub url: String,
    pub hash: String,
    /// Link name -> directory of the snapshot it resolves to.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
    /// The systems that install it; absent when all of them do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub systems: Option<Vec<String>>,
}

impl Lockfile {
    pub fn parse(text: &str) -> Result<Self> {
        let lock: Self =
            serde_yaml_ng::from_str(text).context("failed to parse the pnpm lockfile")?;
        if lock.lockfile_version.split('.').next() != Some("9") {
            bail!(
                "lockfileVersion {} is not supported (expected 9.x)",
                lock.lockfile_version
            );
        }
        Ok(lock)
    }

    /// The graph below the root importer's dependency `root`, per system.
    pub fn graph(&self, root: &str) -> Result<Graph> {
        let reference = &self
            .importers
            .get(".")
            .and_then(|importer| importer.dependencies.get(root))
            .with_context(|| format!("the lockfile's root importer has no {root}"))?
            .version;
        let root_key = snapshot_key(root, reference)?;

        // Walk the graph once per system: a package is installed where some
        // path of installable packages reaches it, which is more than its own
        // os/cpu fields say for the dependencies of a platform-only package.
        let mut reached: IndexMap<String, Vec<&'static str>> = IndexMap::new();
        for platform in &PLATFORMS {
            let mut stack = vec![root_key.clone()];
            let mut seen = std::collections::HashSet::new();
            while let Some(key) = stack.pop() {
                if !seen.insert(key.clone()) {
                    continue;
                }
                reached
                    .entry(key.clone())
                    .or_default()
                    .push(platform.system);
                let snapshot = self.snapshot(&key)?;
                for (name, reference, optional) in snapshot.edges() {
                    let target = snapshot_key(name, reference)?;
                    if self.package(&target)?.installable(platform) {
                        stack.push(target);
                    } else if !optional {
                        bail!(
                            "{key} needs {target}, which does not install on {}",
                            platform.system
                        );
                    }
                }
            }
        }

        let mut packages = BTreeMap::new();
        for (key, systems) in &reached {
            let (name, version, _) = split_key(key)?;
            let package = self.package(key)?;
            let mut dependencies = BTreeMap::new();
            for (link, reference, _) in self.snapshot(key)?.edges() {
                let target = snapshot_key(link, reference)?;
                // Dependencies that install on no system this one installs on
                // were skipped by the walk above.
                if reached.contains_key(&target) {
                    dependencies.insert(link.to_string(), id(&target)?);
                }
            }
            let systems = (systems.len() < PLATFORMS.len())
                .then(|| systems.iter().map(|system| (*system).to_string()).collect());
            packages.insert(
                id(key)?,
                Node {
                    name: name.to_string(),
                    url: package.url(name, version),
                    hash: package.hash(key)?,
                    dependencies,
                    systems,
                },
            );
        }

        Ok(Graph {
            root: id(&root_key)?,
            engines: self.package(&root_key)?.engines.clone(),
            packages,
        })
    }

    fn package(&self, key: &str) -> Result<&Package> {
        let (name, version, _) = split_key(key)?;
        self.packages
            .get(&format!("{name}@{version}"))
            .with_context(|| format!("the lockfile has no package {name}@{version}"))
    }

    fn snapshot(&self, key: &str) -> Result<&Snapshot> {
        self.snapshots
            .get(key)
            .with_context(|| format!("the lockfile has no snapshot {key}"))
    }
}

impl Snapshot {
    /// `(link name, reference, optional)` for every dependency.
    fn edges(&self) -> impl Iterator<Item = (&str, &str, bool)> {
        let required = self
            .dependencies
            .iter()
            .map(|(name, reference)| (name.as_str(), reference.as_str(), false));
        let optional = self
            .optional_dependencies
            .iter()
            .map(|(name, reference)| (name.as_str(), reference.as_str(), true));
        required.chain(optional)
    }
}

impl Package {
    /// npm's rule: every listed field must allow the platform, and a `!x`
    /// entry rules `x` out.
    fn installable(&self, platform: &Platform) -> bool {
        allows(&self.os, platform.os)
            && allows(&self.cpu, platform.cpu)
            && platform.libc.is_none_or(|libc| allows(&self.libc, libc))
    }

    fn url(&self, name: &str, version: &str) -> String {
        if let Some(tarball) = &self.resolution.tarball {
            return tarball.clone();
        }
        let basename = name.rsplit('/').next().unwrap_or(name);
        format!("{REGISTRY}/{name}/-/{basename}-{version}.tgz")
    }

    /// The registry's integrity, already SRI. Only sha512 is accepted: older
    /// packages may still carry a bare sha1.
    fn hash(&self, key: &str) -> Result<String> {
        match &self.resolution.integrity {
            Some(integrity) if integrity.starts_with("sha512-") => Ok(integrity.clone()),
            Some(integrity) => bail!("{key} has a non-sha512 integrity: {integrity}"),
            None => bail!("{key} has no integrity"),
        }
    }
}

fn allows(list: &[String], value: &str) -> bool {
    let (excluded, included): (Vec<&String>, Vec<&String>) =
        list.iter().partition(|entry| entry.starts_with('!'));
    (included.is_empty() || included.iter().any(|entry| *entry == value))
        && !excluded.iter().any(|entry| &entry[1..] == value)
}

/// The snapshot a dependency resolves to. A reference is a version with an
/// optional peer suffix (`5.1.0(vitest@5.1.0)`), or for an aliased dependency
/// the target's own key (`vite: '@voidzero-dev/vite-plus-core@8.3.8000'`).
fn snapshot_key(name: &str, reference: &str) -> Result<String> {
    if reference.starts_with("link:") || reference.starts_with("file:") {
        bail!("{name} is a local dependency ({reference}), which has no registry tarball");
    }
    if reference.starts_with(|c: char| c.is_ascii_digit()) {
        Ok(format!("{name}@{reference}"))
    } else {
        Ok(reference.to_string())
    }
}

/// `(name, version, peer suffix)` of `name@version(peers...)`.
fn split_key(key: &str) -> Result<(&str, &str, &str)> {
    // A scoped name starts with `@`, so the separator is the first `@` after it.
    let at = key
        .get(1..)
        .and_then(|rest| rest.find('@'))
        .map(|at| at + 1)
        .with_context(|| format!("{key} is not a name@version"))?;
    let (name, rest) = (&key[..at], &key[at + 1..]);
    let (version, peers) = rest.split_at(rest.find('(').unwrap_or(rest.len()));
    Ok((name, version, peers))
}

/// A short, path-safe directory for a snapshot. The same package resolved
/// against different peers is a different snapshot, so the peer suffix goes
/// in as a hash, as pnpm does.
fn id(key: &str) -> Result<String> {
    let (name, version, peers) = split_key(key)?;
    let base = format!("{}@{version}", name.replace('/', "+"));
    if peers.is_empty() {
        return Ok(base);
    }
    let digest = Sha256::digest(peers.as_bytes());
    let hex: String = digest[..5]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("{base}_{hex}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_scoped_and_peered_keys() {
        assert_eq!(split_key("a@1.0.0").unwrap(), ("a", "1.0.0", ""));
        assert_eq!(
            split_key("@scope/a@1.0.0(b@2.0.0(c@3.0.0))").unwrap(),
            ("@scope/a", "1.0.0", "(b@2.0.0(c@3.0.0))")
        );
        assert!(split_key("nameless").is_err());
    }

    #[test]
    fn resolves_versions_and_aliases() {
        assert_eq!(
            snapshot_key("a", "1.0.0(b@2.0.0)").unwrap(),
            "a@1.0.0(b@2.0.0)"
        );
        assert_eq!(
            snapshot_key("vite", "@voidzero-dev/vite-plus-core@8.3.8000").unwrap(),
            "@voidzero-dev/vite-plus-core@8.3.8000"
        );
        assert!(snapshot_key("a", "link:../a").is_err());
    }

    #[test]
    fn follows_npm_platform_rules() {
        let list = |entries: &[&str]| entries.iter().map(|e| (*e).to_string()).collect::<Vec<_>>();
        assert!(allows(&[], "linux"));
        assert!(allows(&list(&["linux", "darwin"]), "darwin"));
        assert!(!allows(&list(&["win32"]), "linux"));
        assert!(!allows(&list(&["!linux"]), "linux"));
        assert!(allows(&list(&["!win32"]), "linux"));
    }

    #[test]
    fn builds_the_graph_per_system() {
        let lock = Lockfile::parse(
            "\
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      app:
        specifier: 1.0.0
        version: 1.0.0(peer@2.0.0)
packages:
  app@1.0.0:
    resolution: {integrity: sha512-app}
    engines: {node: '>=22'}
  core@3.0.0:
    resolution: {integrity: sha512-core}
  peer@2.0.0:
    resolution: {integrity: sha512-peer}
  '@n/bin-darwin@1.0.0':
    resolution: {integrity: sha512-darwin}
    os: [darwin]
    cpu: [arm64]
  '@n/bin-linux-gnu@1.0.0':
    resolution: {integrity: sha512-gnu}
    os: [linux]
    libc: [glibc]
  '@n/bin-linux-musl@1.0.0':
    resolution: {integrity: sha512-musl}
    os: [linux]
    libc: [musl]
snapshots:
  app@1.0.0(peer@2.0.0):
    dependencies:
      vite: core@3.0.0
      peer: 2.0.0
    optionalDependencies:
      '@n/bin-darwin': 1.0.0
      '@n/bin-linux-gnu': 1.0.0
      '@n/bin-linux-musl': 1.0.0
  core@3.0.0: {}
  peer@2.0.0: {}
  '@n/bin-darwin@1.0.0':
    optional: true
  '@n/bin-linux-gnu@1.0.0':
    optional: true
  '@n/bin-linux-musl@1.0.0':
    optional: true
",
        )
        .unwrap();
        let graph = lock.graph("app").unwrap();

        assert!(graph.root.starts_with("app@1.0.0_"));
        assert_eq!(graph.engines["node"], ">=22");
        let ids: Vec<&str> = graph.packages.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "@n+bin-darwin@1.0.0",
                "@n+bin-linux-gnu@1.0.0",
                graph.root.as_str(),
                "core@3.0.0",
                "peer@2.0.0"
            ]
        );

        let root = &graph.packages[&graph.root];
        assert_eq!(root.url, "https://registry.npmjs.org/app/-/app-1.0.0.tgz");
        assert_eq!(root.hash, "sha512-app");
        // The alias links under its own name, to the target package.
        assert_eq!(root.dependencies["vite"], "core@3.0.0");
        assert!(root.systems.is_none());
        // Musl builds install on no system; the rest only where they run.
        assert!(!root.dependencies.contains_key("@n/bin-linux-musl"));
        let darwin = &graph.packages["@n+bin-darwin@1.0.0"];
        assert_eq!(
            darwin.systems.as_deref(),
            Some(&["aarch64-darwin".to_string()][..])
        );
        assert_eq!(
            darwin.url,
            "https://registry.npmjs.org/@n/bin-darwin/-/bin-darwin-1.0.0.tgz"
        );
        assert_eq!(
            graph.packages["@n+bin-linux-gnu@1.0.0"].systems.as_deref(),
            Some(&["x86_64-linux".to_string(), "aarch64-linux".to_string()][..])
        );
    }

    #[test]
    fn peer_variants_get_distinct_directories() {
        assert_eq!(id("@a/b@1.0.0").unwrap(), "@a+b@1.0.0");
        let one = id("a@1.0.0(b@1.0.0)").unwrap();
        let two = id("a@1.0.0(b@2.0.0)").unwrap();
        assert!(one.starts_with("a@1.0.0_"));
        assert_ne!(one, two);
    }
}
