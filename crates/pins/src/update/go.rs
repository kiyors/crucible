//! `update go`: pin the newest release of every supported Go line, and
//! `pin go`: print the per-system hashes of a release the pins do not cover.
//!
//! go.dev's release index lists every file of every release with its sha256,
//! so no archive has to be downloaded. Go supports each major release until
//! there are two newer ones, so the two newest lines with a stable release are
//! the supported ones; the index has no schedule beyond that.

use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{json, Plan};
use crate::http::Http;
use crate::source::Source;
use crate::sri;

/// Every release, stable or not, with its files. Must stay the source of the
/// hashes `pin go` prints.
const INDEX: &str = "https://go.dev/dl/?mode=json&include=all";
/// Where archives are served from without go.dev's redirect, as `downloadUrl`
/// in lib/go/toolchain.nix builds it.
pub const DL: &str = "https://dl.google.com/go";
/// "Each major Go release is supported until there are two newer major
/// releases" (go.dev/doc/devel/release).
const SUPPORTED_LINES: usize = 2;

/// Nix system -> Go `GOOS-GOARCH`, in `lib/common/systems.nix` order.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "linux-amd64"),
    ("aarch64-linux", "linux-arm64"),
    ("aarch64-darwin", "darwin-arm64"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// One entry of the index, newest first upstream. Only `version` is read
/// outside this module, by `pin go`.
#[derive(Debug, Deserialize)]
pub struct Release {
    /// `go1.27.4`, `go1.27rc3`, or `go1.20` for the first release of a line
    /// before Go 1.21.
    pub(crate) version: String,
    stable: bool,
    files: Vec<File>,
}

#[derive(Debug, Deserialize)]
struct File {
    filename: String,
    sha256: String,
    /// `archive`, `installer` or `source`.
    kind: String,
}

/// The pinned format of `data/go/go.json`.
#[derive(Debug, Serialize)]
struct Pins {
    /// The newest line.
    stable: String,
    /// Line (`1.27`) -> its newest release, oldest line first.
    releases: IndexMap<String, Pin>,
}

#[derive(Debug, Serialize)]
struct Pin {
    version: String,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/go/go.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let pins = pins(&index(http)?)?;
    for (line, pin) in &pins.releases {
        println!("go {line}: {}", pin.version);
    }
    Ok(vec![(root.join("data/go/go.json"), json(&pins)?)])
}

/// The go.dev index, with every file of every release. `pin::go` reads the
/// hashes of an unpinned release from it too.
pub(crate) fn index(http: &Http) -> Result<Vec<Release>> {
    serde_json::from_slice(&http.get(INDEX)?).context("failed to parse the go.dev release index")
}

/// The newest stable release of each supported line, oldest line first.
fn pins(releases: &[Release]) -> Result<Pins> {
    let mut newest: IndexMap<(u64, u64), (&Release, u64)> = IndexMap::new();
    for release in releases.iter().filter(|release| release.stable) {
        let Some((major, minor, patch)) = parse_version(&release.version) else {
            continue;
        };
        let entry = newest.entry((major, minor)).or_insert((release, patch));
        if patch > entry.1 {
            *entry = (release, patch);
        }
    }
    newest.sort_keys();
    if newest.len() < SUPPORTED_LINES {
        bail!(
            "the go.dev index lists {} stable lines, fewer than the {SUPPORTED_LINES} Go supports",
            newest.len()
        );
    }

    let mut pins = Pins {
        stable: String::new(),
        releases: IndexMap::new(),
    };
    for ((major, minor), (release, _)) in newest.iter().skip(newest.len() - SUPPORTED_LINES) {
        let line = format!("{major}.{minor}");
        pins.releases.insert(
            line.clone(),
            Pin {
                version: release.version.trim_start_matches("go").to_string(),
                platforms: sources(release)?,
            },
        );
        pins.stable = line;
    }
    Ok(pins)
}

/// `{ system: { url, hash } }` for one release, from the index's hashes.
pub(crate) fn sources(release: &Release) -> Result<IndexMap<String, Source>> {
    let mut platforms = IndexMap::new();
    for (system, platform) in PLATFORMS {
        let filename = format!("{}.{platform}.tar.gz", release.version);
        let file = release
            .files
            .iter()
            .find(|file| file.kind == "archive" && file.filename == filename)
            .with_context(|| format!("{} has no {filename}", release.version))?;
        platforms.insert(
            system.to_string(),
            Source {
                url: format!("{DL}/{filename}"),
                hash: sri::from_hex(&file.sha256)
                    .with_context(|| format!("{filename} has a malformed sha256"))?,
            },
        );
    }
    Ok(platforms)
}

/// `go1.27.4` -> (1, 27, 4), and `go1.20` (the first release of a line before
/// Go 1.21) -> (1, 20, 0). Pre-releases are `None`.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.strip_prefix("go")?.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = match parts.next() {
        Some(patch) => patch.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// An exact release such as `1.25.3`, `go` prefix optional; `1.27rc1` too,
/// since the index lists pre-releases. Lines are already pinned or too old.
pub(crate) fn exact_version(version: &str) -> Result<&str> {
    let version = version.strip_prefix("go").unwrap_or(version);
    let valid = match version
        .split_once("rc")
        .or_else(|| version.split_once("beta"))
    {
        Some((line, n)) => {
            line.split('.').count() == 2 && all_numbers(line) && all_numbers(n) && !n.is_empty()
        }
        None => version.split('.').count() == 3 && all_numbers(version),
    };
    if !valid {
        bail!("\"{version}\" is not an exact Go release such as 1.25.3; lines (1.27) are already pinned");
    }
    Ok(version)
}

fn all_numbers(text: &str) -> bool {
    text.split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "7226736c2339719cda28fbf3a54d71aa63bcefe376e98cef00034268dab24840";

    fn release(version: &str, stable: bool) -> Release {
        let files = PLATFORMS
            .iter()
            .map(|(_, platform)| File {
                filename: format!("{version}.{platform}.tar.gz"),
                sha256: HEX.to_string(),
                kind: "archive".to_string(),
            })
            .collect();
        Release {
            version: version.to_string(),
            stable,
            files,
        }
    }

    #[test]
    fn parses_release_names() {
        assert_eq!(parse_version("go1.27.4"), Some((1, 27, 4)));
        assert_eq!(parse_version("go1.20"), Some((1, 20, 0)));
        assert_eq!(parse_version("go1.27rc3"), None);
        assert_eq!(parse_version("1.27.4"), None);
    }

    #[test]
    fn pins_the_newest_patch_of_the_two_newest_lines() {
        let releases = [
            release("go1.28rc1", false),
            release("go1.27.10", true),
            release("go1.27.9", true),
            release("go1.26.11", true),
            release("go1.26.2", true),
            release("go1.25.12", true),
            release("go1.20", true),
        ];
        let pins = pins(&releases).unwrap();
        assert_eq!(pins.stable, "1.27");
        let lines: Vec<(&str, &str)> = pins
            .releases
            .iter()
            .map(|(line, pin)| (line.as_str(), pin.version.as_str()))
            .collect();
        assert_eq!(lines, [("1.26", "1.26.11"), ("1.27", "1.27.10")]);
        assert_eq!(
            pins.releases["1.27"].platforms["aarch64-darwin"].url,
            "https://dl.google.com/go/go1.27.10.darwin-arm64.tar.gz"
        );
        assert!(pins.releases["1.27"].platforms["x86_64-linux"]
            .hash
            .starts_with("sha256-"));
    }

    #[test]
    fn needs_an_archive_for_every_system() {
        let mut partial = release("go1.27.4", true);
        partial
            .files
            .retain(|file| !file.filename.contains("darwin"));
        assert!(sources(&partial).is_err());

        // An installer of the same name is not an archive.
        let mut installer = release("go1.27.4", true);
        installer.files[0].kind = "installer".to_string();
        assert!(sources(&installer).is_err());
    }

    #[test]
    fn fails_with_too_few_lines() {
        assert!(pins(&[release("go1.27.4", true), release("go1.28rc1", false)]).is_err());
    }

    #[test]
    fn accepts_only_exact_go_releases() {
        assert_eq!(exact_version("go1.25.3").unwrap(), "1.25.3");
        assert_eq!(exact_version("1.27rc1").unwrap(), "1.27rc1");
        assert_eq!(exact_version("1.27beta2").unwrap(), "1.27beta2");
        for version in [
            "1.27", "stable", "1.27.x", "1.27rc", "rc1", "1..3", "1.2.3.4",
        ] {
            assert!(exact_version(version).is_err(), "{version}");
        }
    }
}
