//! `update rust`: pin the stable, beta and nightly channel manifests.
//!
//! Nightly often ships a component for some hosts only, so the pinned nightly
//! is the newest one that is complete on every host. Finding it means looking
//! at up to a month of manifests, which is why those fetches run in parallel.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::write_json;
use crate::http::Http;
use crate::manifest::Manifest;

const DIST: &str = "https://static.rust-lang.org/dist";

/// Cranelift is here because huacnlee/cargo builds local crates with it on a
/// nightly rustc.
const REQUIRED: [&str; 9] = [
    "rustc",
    "cargo",
    "rust-std",
    "rust-src",
    "clippy-preview",
    "rustfmt-preview",
    "llvm-tools-preview",
    "rust-analyzer-preview",
    "rustc-codegen-cranelift-preview",
];

const MAX_DAYS: u32 = 30;
const WORKERS: usize = 6;

pub fn update(http: &Http, root: &Path) -> Result<()> {
    let data = root.join("data");
    fs::create_dir_all(&data)?;

    for channel in ["stable", "beta"] {
        let manifest = fetch_manifest(http, &format!("{DIST}/channel-rust-{channel}.toml"))?;
        let trimmed = manifest.trim();
        println!("  {channel}: {} ({})", trimmed.version, trimmed.date);
        write_json(&data.join(format!("{channel}.json")), &trimmed)?;
    }

    let trimmed = complete_nightly(http)?.trim();
    println!("  nightly: {} ({})", trimmed.version, trimmed.date);
    write_json(&data.join("nightly.json"), &trimmed)
}

fn fetch_manifest(http: &Http, url: &str) -> Result<Manifest> {
    let body = http.get(url)?;
    let text = String::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {url}"))
}

/// The newest nightly that has every required component on every host.
fn complete_nightly(http: &Http) -> Result<Manifest> {
    let latest = fetch_manifest(http, &format!("{DIST}/channel-rust-nightly.toml"))?;
    let start = parse_date(&latest.date)
        .with_context(|| format!("nightly manifest has no usable date: {}", latest.date))?;

    let latest_date = latest.date.clone();
    if let Some(manifest) = complete(latest, &latest_date) {
        return Ok(manifest);
    }

    // Newest first, so the first complete batch wins and the later days are
    // never fetched.
    let days: Vec<u32> = (1..=MAX_DAYS).collect();
    for batch in days.chunks(WORKERS) {
        for (day, manifest) in fetch_batch(http, start, batch) {
            if let Some(manifest) = manifest {
                if let Some(manifest) = complete(manifest, &date_of(start - i64::from(day))) {
                    return Ok(manifest);
                }
            }
        }
    }

    bail!("no complete nightly in the last {MAX_DAYS} days")
}

/// Report whether `manifest` is complete, and return it if it is.
fn complete(manifest: Manifest, date: &str) -> Option<Manifest> {
    let missing = manifest.missing(&REQUIRED);
    if missing.is_empty() {
        println!("  nightly: {date} is complete");
        Some(manifest)
    } else {
        println!("  nightly: {date} lacks {}", missing.join(", "));
        None
    }
}

/// Fetch up to a batch of dated manifests concurrently. A failed candidate is
/// skipped: a gap in the archive is not fatal while an older complete nightly
/// still exists.
fn fetch_batch(http: &Http, start: i64, days: &[u32]) -> Vec<(u32, Option<Manifest>)> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = days
            .iter()
            .map(|&day| {
                let http = http.clone();
                let url = format!(
                    "{DIST}/{}/channel-rust-nightly.toml",
                    date_of(start - i64::from(day))
                );
                scope.spawn(move || (day, fetch_manifest(&http, &url).ok()))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("nightly fetch thread panicked"))
            .collect()
    })
}

/// Days since the Unix epoch, so "30 days back" needs no date library.
fn parse_date(date: &str) -> Result<i64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next().context("missing year")?.parse()?;
    let month: i64 = parts.next().context("missing month")?.parse()?;
    let day: i64 = parts.next().context("missing day")?.parse()?;
    if parts.next().is_some() {
        bail!("unexpected trailing text");
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        bail!("date is out of range");
    }
    Ok(days_from_civil(year, month, day))
}

fn date_of(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

// Howard Hinnant's civil calendar algorithms: exact for any proleptic
// Gregorian date and short enough to keep the dependency list free of chrono.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let era_day = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + era_day - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_back_over_month_and_year_boundaries() {
        let start = parse_date("2026-11-12").unwrap();
        assert_eq!(date_of(start - 1), "2026-11-11");
        assert_eq!(date_of(start - 12), "2026-10-31");
        assert_eq!(date_of(start - 30), "2026-10-13");

        let start = parse_date("2027-01-02").unwrap();
        assert_eq!(date_of(start - 2), "2026-12-31");
    }

    #[test]
    fn handles_leap_days() {
        let start = parse_date("2028-03-01").unwrap();
        assert_eq!(date_of(start - 1), "2028-02-29");
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn rejects_malformed_dates() {
        assert!(parse_date("2026-11").is_err());
        assert!(parse_date("2026-11-12-1").is_err());
        assert!(parse_date("2026-13-01").is_err());
    }
}
