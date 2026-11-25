//! `update rust`: pin the stable, beta and nightly channel manifests.
//!
//! Nightly often ships a component for some hosts only, so the pinned nightly
//! is the newest one that is complete on every host. Finding it means looking
//! at up to a month of manifests, which is why those fetches run in parallel.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::{json, Plan};
use crate::http::{Http, HttpError};
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

/// Fetch and validate stable, beta and nightly, without writing anything.
pub(crate) fn plan(http: &Http, root: &Path) -> Result<Plan> {
    let data = root.join("data").join("rust");
    let mut plan = Plan::new();

    for channel in ["stable", "beta"] {
        let manifest = fetch_manifest(http, &format!("{DIST}/channel-rust-{channel}.toml"))?;
        let trimmed = manifest.trim();
        println!("  {channel}: {} ({})", trimmed.version, trimmed.date);
        plan.push((data.join(format!("{channel}.json")), json(&trimmed)?));
    }

    let trimmed = complete_nightly(http)?.trim();
    println!("  nightly: {} ({})", trimmed.version, trimmed.date);
    plan.push((data.join("nightly.json"), json(&trimmed)?));
    Ok(plan)
}

fn fetch_manifest(http: &Http, url: &str) -> Result<Manifest> {
    let body = http.get(url)?;
    parse_manifest(url, &body)
}

fn parse_manifest(url: &str, body: &[u8]) -> Result<Manifest> {
    let text = std::str::from_utf8(body).with_context(|| format!("{url} is not UTF-8"))?;
    toml::from_str(text).with_context(|| format!("failed to parse {url}"))
}

/// The newest nightly that has every required component on every host.
fn complete_nightly(http: &Http) -> Result<Manifest> {
    let latest = fetch_manifest(http, &format!("{DIST}/channel-rust-nightly.toml"))?;
    let start = parse_date(&latest.date)
        .with_context(|| format!("nightly manifest has no usable date: {}", latest.date))?;

    let date = latest.date.clone();
    if let Some(manifest) = pick_nightly(vec![(date, inspect(latest))])? {
        return Ok(manifest);
    }

    // Newest first, so the first complete batch wins and the later days are
    // never fetched. Within a batch, `pick_nightly` stops at the first complete
    // date and only tolerates gaps and incomplete dates ahead of it.
    let days: Vec<u32> = (1..=MAX_DAYS).collect();
    for batch in days.chunks(WORKERS) {
        if let Some(manifest) = pick_nightly(fetch_batch(http, start, batch))? {
            return Ok(manifest);
        }
    }

    bail!("no complete nightly in the last {MAX_DAYS} days")
}

/// How one dated manifest fetch turned out.
///
/// Kept separate from the walk so the "newer than the pick" rule is a pure
/// decision that can be tested without a network.
enum NightlyOutcome {
    /// The archive has no manifest for this date (404): a normal gap.
    Missing,
    /// The manifest exists but lacks required components on some host.
    Incomplete(Vec<String>),
    // Boxed: a manifest is large and the other outcomes are not.
    Complete(Box<Manifest>),
    /// Anything else: transport, non-404 HTTP or parse failure.
    Failed(anyhow::Error),
}

fn inspect(manifest: Manifest) -> NightlyOutcome {
    let missing = manifest.missing(&REQUIRED);
    if missing.is_empty() {
        NightlyOutcome::Complete(Box::new(manifest))
    } else {
        NightlyOutcome::Incomplete(missing)
    }
}

/// Pick the newest complete nightly from `results`, which must be newest first.
///
/// A gap or an incomplete manifest is a normal miss. A failure is not: it may
/// hide a complete nightly that is newer than the pick, and falling back to an
/// older one would silently move the pin backwards. So any failure newer than
/// the pick fails the whole update with its date and URL.
fn pick_nightly(results: Vec<(String, NightlyOutcome)>) -> Result<Option<Manifest>> {
    for (date, outcome) in results {
        match outcome {
            NightlyOutcome::Complete(manifest) => {
                println!("  nightly: {date} is complete");
                return Ok(Some(*manifest));
            }
            NightlyOutcome::Missing => continue,
            NightlyOutcome::Incomplete(missing) => {
                println!("  nightly: {date} lacks {}", missing.join(", "));
            }
            NightlyOutcome::Failed(err) => {
                bail!(
                    "crucible: nightly {date} ({}) failed: {err:#}; check the fetch and retry \
                     `crucible update`",
                    daily_url(&date)
                );
            }
        }
    }
    Ok(None)
}

fn daily_url(date: &str) -> String {
    format!("{DIST}/{date}/channel-rust-nightly.toml")
}

/// Fetch up to a batch of dated manifests concurrently, newest first. Only a
/// 404 means "no such nightly"; everything else is reported as a failure for
/// `pick_nightly` to judge.
fn fetch_batch(http: &Http, start: i64, days: &[u32]) -> Vec<(String, NightlyOutcome)> {
    // Boxed so one job per date can be handed to `join_fetches`; each worker
    // owns its `Http` clone, so no borrow can outlive this call.
    let jobs = days.iter().map(|&day| {
        let date = date_of(start - i64::from(day));
        let http = http.clone();
        let fetch_date = date.clone();
        (
            date,
            Box::new(move || fetch_dated(&http, &fetch_date)) as FetchJob,
        )
    });
    join_fetches(jobs.collect())
}

type FetchJob = Box<dyn FnOnce() -> NightlyOutcome + Send>;

/// Run the fetch jobs and turn a dead thread into a `Failed` date. A panic in
/// a worker must not become a process panic: `pick_nightly` has to judge it
/// like any other failure newer than the pick, and falling back silently
/// would move the pin backwards.
fn join_fetches(jobs: Vec<(String, FetchJob)>) -> Vec<(String, NightlyOutcome)> {
    let handles: Vec<_> = jobs
        .into_iter()
        .map(|(date, job)| (date, std::thread::spawn(job)))
        .collect();
    handles
        .into_iter()
        .map(|(date, handle)| match handle.join() {
            Ok(outcome) => (date, outcome),
            Err(_) => (
                date,
                NightlyOutcome::Failed(anyhow::anyhow!("nightly fetch thread panicked")),
            ),
        })
        .collect()
}

fn fetch_dated(http: &Http, date: &str) -> NightlyOutcome {
    let url = daily_url(date);
    let body = match http.get(&url) {
        Ok(body) => body,
        Err(HttpError::NotFound { .. }) => return NightlyOutcome::Missing,
        Err(err) => return NightlyOutcome::Failed(err.into()),
    };
    match parse_manifest(&url, &body) {
        Ok(manifest) => inspect(manifest),
        Err(err) => NightlyOutcome::Failed(err),
    }
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

    fn complete(date: &str) -> NightlyOutcome {
        NightlyOutcome::Complete(Box::new(Manifest {
            date: date.to_string(),
            ..Default::default()
        }))
    }

    #[test]
    fn picks_the_newest_complete_one_after_gaps_and_incomplete_dates() {
        let picked = pick_nightly(vec![
            ("2026-11-10".to_string(), NightlyOutcome::Missing),
            (
                "2026-11-09".to_string(),
                NightlyOutcome::Incomplete(vec!["cargo/x86_64-unknown-linux-gnu".to_string()]),
            ),
            ("2026-11-08".to_string(), complete("2026-11-08")),
            ("2026-11-07".to_string(), complete("2026-11-07")),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(picked.date, "2026-11-08");
    }

    #[test]
    fn a_failure_newer_than_the_pick_fails_the_run() {
        let err = pick_nightly(vec![
            ("2026-11-10".to_string(), NightlyOutcome::Missing),
            (
                "2026-11-09".to_string(),
                NightlyOutcome::Failed(anyhow::anyhow!("connection reset")),
            ),
            ("2026-11-08".to_string(), complete("2026-11-08")),
        ])
        .unwrap_err()
        .to_string();

        assert!(err.contains("2026-11-09"), "{err}");
        assert!(
            err.contains("https://static.rust-lang.org/dist/2026-11-09/channel-rust-nightly.toml"),
            "{err}"
        );
    }

    #[test]
    fn a_panicked_fetch_thread_is_a_failure_newer_than_the_pick() {
        let results = join_fetches(vec![
            (
                "2026-11-10".to_string(),
                Box::new(|| panic!("fetch exploded")),
            ),
            (
                "2026-11-09".to_string(),
                Box::new(|| complete("2026-11-09")),
            ),
        ]);
        assert!(matches!(results[0].1, NightlyOutcome::Failed(_)));
        let err = pick_nightly(results).unwrap_err().to_string();
        assert!(err.contains("2026-11-10"), "{err}");
        assert!(err.contains("nightly fetch thread panicked"), "{err}");
    }

    #[test]
    fn gaps_and_incomplete_dates_alone_mean_no_pick() {
        assert!(pick_nightly(vec![
            ("2026-11-10".to_string(), NightlyOutcome::Missing),
            (
                "2026-11-09".to_string(),
                NightlyOutcome::Incomplete(vec!["rustc/x".to_string()]),
            ),
        ])
        .unwrap()
        .is_none());
    }

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
