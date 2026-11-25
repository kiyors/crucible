//! `update zig`: pin the newest tagged release of every Zig minor line.
//!
//! ziglang.org's `index.json` lists every tagged release with the SHA-256 of
//! each platform's tarball, so no tarball has to be downloaded. Master is a
//! rolling snapshot and is not pinned.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use super::{json, Fetch, Plan};
use crate::http::Http;
use crate::source::{self, Source};
use crate::sri;

const DIST: &str = "https://ziglang.org/download";

/// Nix system -> ziglang.org platform, in `lib/common/systems.nix` order, as
/// `PLATFORMS` in `lib/zig/toolchain.nix`.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "x86_64-linux"),
    ("aarch64-linux", "aarch64-linux"),
    ("aarch64-darwin", "aarch64-macos"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// The pinned format of `data/zig/zig.json`.
#[derive(Debug, Serialize)]
struct Pins {
    /// The line of the newest pinned release.
    stable: String,
    /// Minor line ("0.15") -> its newest patch release.
    releases: IndexMap<String, Pin>,
}

#[derive(Debug, Serialize)]
struct Pin {
    version: String,
    date: String,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/zig/zig.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let index: IndexMap<String, Value> =
        serde_json::from_slice(&http.get(&format!("{DIST}/index.json"))?)
            .context("failed to parse index.json")?;

    // Newest patch per minor: `pin` keeps the newest release of each line, so
    // a later patch replaces the earlier one.
    let mut lines: IndexMap<String, Pin> = IndexMap::new();
    for (key, entry) in index.iter() {
        // Modern entries carry "version"; older ones name it in the key, as
        // in "0.15.2".
        let version = entry.get("version").and_then(Value::as_str).unwrap_or(key);
        let Some(line) = minor_line(version) else {
            continue;
        };
        // A minor whose newest patch has no build for any supported system
        // is not pinned: `zig.nix` throws when building it, so it cannot be
        // used anyway (0.1.1 has only a Windows build).
        if PLATFORMS
            .iter()
            .all(|&(_, platform)| entry.get(platform).is_none())
        {
            println!("zig {line}: no supported system build; not pinned");
            continue;
        }
        if let Some(existing) = lines.get(&line) {
            if version_key(version) <= version_key(&existing.version) {
                continue;
            }
        }
        lines.insert(line, pin_release(version, entry)?);
    }
    if lines.is_empty() {
        bail!("ziglang.org's index.json lists no tagged releases we can build for");
    }

    // Oldest line first, like data/node/node.json.
    let mut releases: Vec<(String, Pin)> = lines.into_iter().collect();
    releases.sort_by_key(|entry| version_key(&entry.0));
    let stable = releases
        .iter()
        .max_by_key(|(_, pin)| version_key(&pin.version))
        .map(|(line, _)| line.clone())
        .unwrap_or_default();
    let pins = Pins {
        stable,
        releases: IndexMap::from_iter(releases),
    };
    for (line, pin) in &pins.releases {
        println!("zig {line}: {}", pin.version);
    }
    Ok(vec![(root.join("data/zig/zig.json"), json(&pins)?)])
}

/// Check that each pinned hash still matches the entry for that pinned version
/// in ziglang.org's index. This is **drift detection, not origin
/// authentication**: the index is plain HTTPS JSON with no signature, so a
/// forged index could satisfy it too. The version comes from `data/`, so new
/// releases appended to the index do not change what is checked.
pub(crate) fn verify<F: Fetch>(http: &F, root: &Path) -> Result<()> {
    let pins = source::read_json(root, "data/zig/zig.json")?;
    let index: IndexMap<String, Value> =
        serde_json::from_slice(&http.get(&format!("{DIST}/index.json"))?)
            .context("failed to parse index.json")?;
    let mut failures = Vec::new();
    for (line, pin) in pins
        .get("releases")
        .and_then(Value::as_object)
        .context("data/zig/zig.json has no releases")?
    {
        let Some(version) = pin.get("version").and_then(Value::as_str) else {
            failures.push(format!("zig {line} has no version"));
            continue;
        };
        let Some((key, entry)) = index.iter().find(|(key, entry)| {
            entry.get("version").and_then(Value::as_str).unwrap_or(key) == version
        }) else {
            failures.push(format!("ziglang.org's index has no {version}"));
            continue;
        };
        let upstream = match pin_release(version, entry) {
            Ok(upstream) => upstream,
            Err(err) => {
                failures.push(format!("zig {version} ({key}): {err:#}"));
                continue;
            }
        };
        for (system, source) in upstream.platforms {
            let pinned = match source::platform_hash(pin, &system) {
                Ok(pinned) => pinned,
                Err(err) => {
                    failures.push(format!("zig {version} {system}: {err:#}"));
                    continue;
                }
            };
            if pinned != source.hash {
                failures.push(format!(
                    "zig {version} {system}: pinned {pinned}, ziglang.org's index says {}",
                    source.hash
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

/// `{ system: { url, hash } }` for a tagged release, from index.json's
/// per-platform entries. A release the index has no build for on a supported
/// system still gets pinned with the platforms it has: `zig.nix` throws for
/// the ones it lacks, so a toolchain can never build there silently.
fn pin_release(version: &str, entry: &Value) -> Result<Pin> {
    let mut platforms = IndexMap::new();
    for (system, platform) in PLATFORMS {
        let Some(source) = entry.get(platform) else {
            continue;
        };
        platforms.insert(
            system.to_string(),
            Source {
                url: source
                    .get("tarball")
                    .and_then(Value::as_str)
                    .with_context(|| {
                        format!("ziglang.org's index.json has no tarball for {platform}")
                    })?
                    .to_string(),
                hash: sri::from_hex(source.get("shasum").and_then(Value::as_str).with_context(
                    || format!("ziglang.org's index.json has no shasum for {platform}"),
                )?)?,
            },
        );
    }
    Ok(Pin {
        version: version.to_string(),
        date: entry
            .get("date")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        platforms,
    })
}

/// The minor line of a tagged release: "0.15.2" -> "0.15". Versions that are
/// not all-numeric release versions (master's "0.18.0-dev....") have none.
fn minor_line(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if minor.is_empty() || !minor.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match parts.next() {
        // "0.15.2": the line is the first two parts.
        Some(patch) => {
            if patch.is_empty()
                || !patch.bytes().all(|b| b.is_ascii_digit())
                || parts.next().is_some()
            {
                None
            } else {
                Some(format!("{major}.{minor}"))
            }
        }
        // "0.15" is its own line.
        None => Some(version.to_string()),
    }
}

/// A version as integers, for comparing releases: "0.15.2" -> [0, 15, 2].
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_minor_lines() {
        assert_eq!(minor_line("0.15.2"), Some("0.15".to_string()));
        assert_eq!(minor_line("0.2.0"), Some("0.2".to_string()));
        assert_eq!(minor_line("0.1.1"), Some("0.1".to_string()));
        // A version with only major.minor is its own line.
        assert_eq!(minor_line("0.15"), Some("0.15".to_string()));
        // Master and other non-tagged versions have no line.
        assert_eq!(minor_line("0.18.0-dev.1244+b5ffcbd70"), None);
        assert_eq!(minor_line("master"), None);
        assert_eq!(minor_line("v0.15.2"), None);
        assert_eq!(minor_line(""), None);
    }

    #[test]
    fn sorts_versions() {
        assert!(version_key("0.15.2") > version_key("0.15.1"));
        assert!(version_key("0.16.0") > version_key("0.15.2"));
        assert!(version_key("0.2.0") < version_key("0.17.1"));
    }

    #[test]
    fn parses_index_entries() {
        // A modern entry: "version", "date" and platform entries.
        let entry: Value = serde_json::from_str(
            r#"{
                "version": "0.15.2",
                "date": "2025-10-11",
                "src": { "tarball": "https://ziglang.org/download/0.15.2/zig-0.15.2.tar.xz", "shasum": "d9b30c7aa983fcff5eed2084d54ae83eaafe7ff3a84d8fb754d854165a6e521c", "size": "21359884" },
                "docs": "https://ziglang.org/documentation/0.15.2/",
                "x86_64-linux": { "tarball": "https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz", "shasum": "02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239", "size": "57411036" },
                "aarch64-linux": { "tarball": "https://ziglang.org/download/0.15.2/zig-aarch64-linux-0.15.2.tar.xz", "shasum": "958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f", "size": "52977364" },
                "aarch64-macos": { "tarball": "https://ziglang.org/download/0.15.2/zig-aarch64-macos-0.15.2.tar.xz", "shasum": "3cc2bab367e185cdfb27501c4b30b1b0653c28d9f73df8dc91488e66ece5fa6b", "size": "50635984" }
            }"#,
        )
        .unwrap();
        assert_eq!(entry.get("version").and_then(Value::as_str), Some("0.15.2"));
        let modern = pin_release("0.15.2", &entry).unwrap();
        assert_eq!(modern.version, "0.15.2");
        assert_eq!(modern.date, "2025-10-11");
        assert_eq!(
            modern.platforms["x86_64-linux"].url,
            "https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz"
        );
        // Data platforms are keyed by the Nix system (see lib/zig/zig.nix).
        assert!(modern.platforms["x86_64-linux"].hash.starts_with("sha256-"));
        assert!(modern.platforms["aarch64-darwin"]
            .hash
            .starts_with("sha256-"));
        assert!(modern.platforms["aarch64-linux"]
            .hash
            .starts_with("sha256-"));

        // An entry without a "version" field: the data uses the key.
        let entry: Value = serde_json::from_str(
            r#"{ "date": "2018-03-15", "x86_64-linux": { "tarball": "https://ziglang.org/download/0.2.0/zig-linux-x86_64-0.2.0.tar.xz", "shasum": "209c6fb745d42474c0a73d6f291c7ae3a38b6a1b6b641eea285a7f840cc1a890", "size": "22551928" } }"#,
        )
        .unwrap();
        assert_eq!(entry.get("version").and_then(Value::as_str), None);
        let legacy = pin_release("0.2.0", &entry).unwrap();
        assert_eq!(legacy.version, "0.2.0");
        assert_eq!(legacy.date, "2018-03-15");
        assert_eq!(
            legacy.platforms["x86_64-linux"].url,
            "https://ziglang.org/download/0.2.0/zig-linux-x86_64-0.2.0.tar.xz"
        );

        // A release with no build for any supported system gets none.
        let entry: Value = serde_json::from_str(
            r#"{ "date": "2017-10-17", "x86_64-windows": { "tarball": "https://ziglang.org/download/0.1.1/zig-win64-0.1.1.zip", "shasum": "6fc88bef531af7e567fe30bf60da1487b86833cbee84c7a2f3e317030aa5b660", "size": "19757776" } }"#,
        )
        .unwrap();
        assert!(PLATFORMS
            .iter()
            .all(|&(_, platform)| entry.get(platform).is_none()));
        assert!(pin_release("0.1.1", &entry).unwrap().platforms.is_empty());
    }
}
