//! `update node`: pin the newest release of every supported Node.js line.
//!
//! nodejs.org's `index.json` lists releases without hashes; each release's
//! `SHASUMS256.txt` has them, so no tarball has to be downloaded. Which lines
//! are supported comes from the Release working group's `schedule.json`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{json, parse_sha256sums, Fetch, Plan};
use crate::http::Http;
use crate::source::{self, hash_of, Source};

pub const DIST: &str = "https://nodejs.org/dist";
const SCHEDULE: &str = "https://raw.githubusercontent.com/nodejs/Release/main/schedule.json";

/// Nix system -> nodejs.org platform, in `lib/common/systems.nix` order.
pub(crate) const PLATFORMS: [(&str, &str); 3] = [
    ("x86_64-linux", "linux-x64"),
    ("aarch64-linux", "linux-arm64"),
    ("aarch64-darwin", "darwin-arm64"),
];

// The literal stays for the text-based sync check; `platforms` is the
// canonical system list that the compiler checks it against.
const _: () = assert!(crate::platforms::matches(PLATFORMS));

/// One entry of `index.json`, newest first upstream.
#[derive(Debug, Deserialize)]
struct Release {
    version: String,
    date: String,
    /// The LTS codename, or `false`.
    lts: Value,
    #[serde(default)]
    npm: Option<String>,
    #[serde(default)]
    modules: Option<String>,
}

/// One release line of `schedule.json`. Dates are `YYYY-MM-DD`, which compare
/// correctly as strings.
#[derive(Debug, Deserialize)]
struct Line {
    start: String,
    end: String,
    #[serde(default)]
    lts: Option<String>,
}

/// The pinned format of `data/node/node.json`.
#[derive(Debug, Serialize)]
struct Pins {
    /// The newest line in long-term support.
    lts: String,
    /// The newest line released at all.
    current: String,
    /// Major -> its newest release, oldest line first.
    releases: IndexMap<String, Pin>,
}

#[derive(Debug, Serialize)]
struct Pin {
    version: String,
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    lts: Option<String>,
    /// End of upstream support, so `doctor` can warn before it arrives.
    end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    npm: Option<String>,
    /// `process.versions.modules`, the ABI native addons are built for.
    #[serde(skip_serializing_if = "Option::is_none")]
    modules: Option<String>,
    platforms: IndexMap<String, Source>,
}

/// Fetch and validate `data/node/node.json`, without writing it.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let releases: Vec<Release> = serde_json::from_slice(&http.get(&format!("{DIST}/index.json"))?)
        .context("failed to parse index.json")?;
    let schedule: IndexMap<String, Line> =
        serde_json::from_slice(&http.get(SCHEDULE)?).context("failed to parse schedule.json")?;

    let today = today();
    let mut pins = Pins {
        lts: String::new(),
        current: String::new(),
        releases: IndexMap::new(),
    };
    for (major, line) in supported(&schedule, &today) {
        let release = newest(&releases, major)
            .with_context(|| format!("index.json has no stable v{major} release"))?;
        let version = release.version.trim_start_matches('v').to_string();
        println!("node {major}: {version}");

        let sums = parse_sha256sums(&shasums(http, &version)?);
        let pin = Pin {
            platforms: sources(&sums, &version)?,
            version,
            date: release.date.clone(),
            lts: release.lts.as_str().map(str::to_owned),
            end: line.end.clone(),
            npm: release.npm.clone(),
            modules: release.modules.clone(),
        };
        // Lines come oldest first, so the last one that qualifies wins.
        if line.lts.as_deref().is_some_and(|lts| lts <= today.as_str()) {
            pins.lts = major.to_string();
        }
        pins.current = major.to_string();
        pins.releases.insert(major.to_string(), pin);
    }
    if pins.lts.is_empty() {
        bail!("schedule.json lists no supported LTS line");
    }

    Ok(vec![(root.join("data/node/node.json"), json(&pins)?)])
}

/// `{ system: { url, hash } }` for one release, from its `SHASUMS256.txt`.
pub(crate) fn sources(
    sums: &[(String, String)],
    version: &str,
) -> Result<IndexMap<String, Source>> {
    let mut platforms = IndexMap::new();
    for (system, platform) in PLATFORMS {
        let file = format!("node-v{version}-{platform}.tar.xz");
        platforms.insert(
            system.to_string(),
            Source {
                url: format!("{DIST}/v{version}/{file}"),
                hash: hash_of(sums, &file, &format!("SHASUMS256.txt of v{version}"))?,
            },
        );
    }
    Ok(platforms)
}

pub(crate) fn shasums<F: Fetch>(http: &F, version: &str) -> Result<String> {
    let url = format!("{DIST}/v{version}/SHASUMS256.txt");
    let body = http.get(&url)?;
    // The hashes are only as trustworthy as the file they came from, so check
    // its detached signature before `update` or `pin` can write a pin.
    let signature = http.get(&format!("{url}.sig"))?;
    crate::gpg::verify_shasums(version, &body, &signature)?;
    String::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))
}

/// Re-verify every pinned Node.js release: its `SHASUMS256.txt` must still
/// carry the detached signature from the vendored keyring, and each pinned
/// hash must still match that signed file. The version comes from `data/`, not
/// from `index.json`'s newest entry.
pub(crate) fn verify<F: Fetch>(http: &F, root: &Path) -> Result<()> {
    let pins = source::read_json(root, "data/node/node.json")?;
    let mut failures = Vec::new();
    for (major, pin) in pins
        .get("releases")
        .and_then(Value::as_object)
        .context("data/node/node.json has no releases")?
    {
        let Some(version) = pin.get("version").and_then(Value::as_str) else {
            failures.push(format!("node {major} has no version"));
            continue;
        };
        if let Err(err) = verify_release(http, pin, version) {
            failures.push(format!("node {version} ({major}): {err:#}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

fn verify_release<F: Fetch>(http: &F, pin: &Value, version: &str) -> Result<()> {
    let sums = parse_sha256sums(&shasums(http, version)?);
    for (system, source) in sources(&sums, version)? {
        let pinned = source::platform_hash(pin, &system)?;
        if source.hash != pinned {
            bail!(
                "{system}: pinned {pinned}, but the signed SHASUMS256.txt says {}",
                source.hash
            );
        }
    }
    Ok(())
}

/// The lines released by `today` and not yet past their end, oldest first.
fn supported<'a>(schedule: &'a IndexMap<String, Line>, today: &str) -> Vec<(u32, &'a Line)> {
    let mut lines: Vec<(u32, &Line)> = schedule
        .iter()
        .filter_map(|(name, line)| Some((name.strip_prefix('v')?.parse().ok()?, line)))
        .filter(|(_, line)| line.start.as_str() <= today && today < line.end.as_str())
        .collect();
    lines.sort_by_key(|(major, _)| *major);
    lines
}

/// The newest release of `major` that is not a pre-release.
fn newest(releases: &[Release], major: u32) -> Option<&Release> {
    let prefix = format!("v{major}.");
    releases
        .iter()
        .filter(|release| release.version.starts_with(&prefix) && !release.version.contains('-'))
        .max_by_key(|release| version_key(&release.version))
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Today in UTC as `YYYY-MM-DD`, from the days since the epoch (Howard
/// Hinnant's `civil_from_days`), to avoid a date crate for one comparison.
fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    civil(i64::try_from(secs / 86_400).unwrap_or(0))
}

fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_days_to_dates() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(19_782), "2024-02-29");
        assert_eq!(civil(20_776), "2026-11-19");
    }

    #[test]
    fn keeps_released_lines_that_have_not_ended() {
        let schedule: IndexMap<String, Line> = serde_json::from_str(
            r#"{
              "v0.10": { "start": "2013-03-11", "end": "2016-10-31" },
              "v24": { "start": "2025-05-06", "lts": "2025-10-28", "end": "2028-04-30" },
              "v25": { "start": "2025-10-15", "end": "2026-06-01" },
              "v22": { "start": "2024-04-24", "lts": "2024-10-29", "end": "2027-04-30" },
              "v27": { "start": "2027-04-22", "end": "2030-04-30" }
            }"#,
        )
        .unwrap();
        let majors: Vec<u32> = supported(&schedule, "2026-11-19")
            .into_iter()
            .map(|(major, _)| major)
            .collect();
        assert_eq!(majors, [22, 24]);
    }

    #[test]
    fn picks_the_newest_stable_release_of_a_line() {
        let releases: Vec<Release> = serde_json::from_str(
            r#"[
              { "version": "v27.0.0-alpha.1", "date": "2026-11-16", "lts": false },
              { "version": "v24.9.0", "date": "2026-01-01", "lts": "Krypton" },
              { "version": "v24.22.3", "date": "2026-11-10", "lts": "Krypton" },
              { "version": "v2.4.0", "date": "2016-01-01", "lts": false }
            ]"#,
        )
        .unwrap();
        assert_eq!(newest(&releases, 24).unwrap().version, "v24.22.3");
        assert!(newest(&releases, 27).is_none());
    }

    #[test]
    fn maps_shasums_to_platform_sources() {
        let sums = parse_sha256sums(
            "\
af6202f5370a3a5209e85dc543758dfd9f44c8726626a236730cd603a67a8559  node-v26.13.1-darwin-arm64.tar.xz
89e1853138db51e8b82d3e98ba9c0d3f2956a28e5e55ea1b7e12d25aae92d5dd  node-v26.13.1-linux-arm64.tar.xz
5c063e46bab0fbc1c17cc445d7ff129cbdcd0471be6ed83b6ab864f8143bc864  node-v26.13.1-linux-x64.tar.xz
",
        );
        let platforms = sources(&sums, "26.13.1").unwrap();
        assert_eq!(
            platforms["aarch64-darwin"].url,
            "https://nodejs.org/dist/v26.13.1/node-v26.13.1-darwin-arm64.tar.xz"
        );
        assert!(platforms["x86_64-linux"].hash.starts_with("sha256-"));
        assert!(sources(&sums[..2], "26.13.1").is_err());
    }
}
