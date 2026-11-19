//! `crucible pin rust`: turn a rustup channel into the two things a Nix user needs
//! to pin it: a `flake = false` input, or the manifest's SRI hash.
//!
//! The URL rules mirror `manifestUrl` in lib/rust/toolchain.nix, including the dated
//! forms (`nightly-2026-11-01`), so the hashes and the flake agree.

use anyhow::{Context, Result};

use crate::http::Http;
use crate::manifest::Manifest;
use crate::sri;

const DIST: &str = "https://static.rust-lang.org/dist";

pub fn run(channel: &str) -> Result<()> {
    let url = manifest_url(channel)?;
    let http = Http::new();
    let body = http.get(&url)?;

    let text = String::from_utf8(body.clone()).with_context(|| format!("{url} is not UTF-8"))?;
    let manifest: Manifest =
        toml::from_str(&text).with_context(|| format!("failed to parse {url}"))?;
    let rustc = manifest
        .pkg
        .get("rustc")
        .with_context(|| format!("{url} has no pkg.rustc"))?
        .version
        .clone();
    let hash = sri::sri(&body);

    println!("rustc: {rustc}");
    println!("manifest: {url}");
    println!("sha256: {hash}");
    println!();
    println!("rust-manifest = {{");
    println!("  url = \"{url}\";");
    println!("  flake = false;");
    println!("}};");
    println!();
    println!("toolchain {{");
    println!("  channel = \"{channel}\";");
    println!("  sha256 = \"{hash}\";");
    println!("}};");
    Ok(())
}

/// The manifest URL for a rustup channel, or an error for anything rustup
/// would not accept.
pub fn manifest_url(channel: &str) -> Result<String> {
    let (name, date) = split_channel(channel)?;
    Ok(match date {
        Some(date) => format!("{DIST}/{date}/channel-rust-{name}.toml"),
        None => format!("{DIST}/channel-rust-{name}.toml"),
    })
}

/// `(name, date)` from `name[-YYYY-MM-DD]`, with `name` one of the forms the
/// `manifestUrl` regex allows.
fn split_channel(channel: &str) -> Result<(&str, Option<&str>)> {
    // `split_at` panics when the offset lands inside a character, so reject
    // anything non-ASCII before computing the suffix length.
    if !channel.is_ascii() {
        return Err(invalid(channel));
    }

    let (name, date) = if channel.len() > 11 {
        let (name, tail) = channel.split_at(channel.len() - 11);
        match tail.strip_prefix('-') {
            Some(date) if valid_date(date) && valid_name(name) => (name, Some(date)),
            _ => (channel, None),
        }
    } else {
        (channel, None)
    };

    if !valid_name(name) {
        return Err(invalid(channel));
    }
    Ok((name, date))
}

/// The one error rustup's channel parser would produce for a bad name.
fn invalid(channel: &str) -> anyhow::Error {
    anyhow::anyhow!("\"{channel}\" is not a channel rustup understands")
}

fn valid_name(name: &str) -> bool {
    if matches!(name, "stable" | "beta" | "nightly") {
        return true;
    }
    let parts: Vec<&str> = name.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_date(date: &str) -> bool {
    let digits = |range: std::ops::Range<usize>| {
        date.get(range)
            .is_some_and(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    };
    date.len() == 10
        && digits(0..4)
        && date.as_bytes()[4] == b'-'
        && digits(5..7)
        && date.as_bytes()[7] == b'-'
        && digits(8..10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_manifest_urls_like_toolchain_nix() {
        let cases = [
            (
                "stable",
                "https://static.rust-lang.org/dist/channel-rust-stable.toml",
            ),
            (
                "beta",
                "https://static.rust-lang.org/dist/channel-rust-beta.toml",
            ),
            (
                "1.99",
                "https://static.rust-lang.org/dist/channel-rust-1.99.toml",
            ),
            (
                "1.99.0",
                "https://static.rust-lang.org/dist/channel-rust-1.99.0.toml",
            ),
            (
                "nightly-2026-11-01",
                "https://static.rust-lang.org/dist/2026-11-01/channel-rust-nightly.toml",
            ),
            (
                "beta-2026-11-14",
                "https://static.rust-lang.org/dist/2026-11-14/channel-rust-beta.toml",
            ),
            (
                "1.99.0-2026-11-01",
                "https://static.rust-lang.org/dist/2026-11-01/channel-rust-1.99.0.toml",
            ),
        ];
        for (channel, url) in cases {
            assert_eq!(manifest_url(channel).unwrap(), url, "{channel}");
        }
    }

    #[test]
    fn rejects_non_ascii_without_panicking() {
        // 11 bytes of suffix would slice inside `é`, which used to panic.
        for channel in ["é0123456789", "nightly-2026-11-0é", "é"] {
            let err = manifest_url(channel).unwrap_err().to_string();
            assert_eq!(
                err,
                format!("\"{channel}\" is not a channel rustup understands")
            );
        }
    }

    #[test]
    fn rejects_what_rustup_rejects() {
        for channel in [
            "",
            "1",
            "1.99.0.1",
            "1.99.",
            "nightly-2026-11-1",
            "nightly-2026-1-01",
            "nightly-2026-11-01-x",
            "nightly-2026-11-01-",
            "nutty",
        ] {
            assert!(
                manifest_url(channel).is_err(),
                "{channel} should be rejected"
            );
        }
    }
}
