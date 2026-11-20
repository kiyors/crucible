//! Rustup channel names and the files that hold them, for `init` and `pin`.
//!
//! `manifest_url` mirrors `manifestUrl` in lib/rust/toolchain.nix, including
//! the dated forms (`nightly-2026-11-01`). It lives outside the `net` feature
//! because `init` writes the URL into a flake input without fetching it.

use std::path::Path;

use anyhow::{Context, Result};

const DIST: &str = "https://static.rust-lang.org/dist";

/// The toolchain file names rustup looks for in a directory, in the order
/// `doctor` prefers them.
pub const TOOLCHAIN_FILES: [&str; 2] = ["rust-toolchain.toml", "rust-toolchain"];

/// The channel a `rust-toolchain(.toml)` file names. As rustup and
/// lib/rust/toolchain.nix decide it: `rust-toolchain` without an extension may
/// still be TOML; only a single bare line is the legacy form.
pub fn toolchain_channel(path: &Path, text: &str) -> Result<String> {
    let text = text.trim();
    let legacy = path.extension().is_none() && !text.contains(['\n', '[', '=']);
    let channel = if legacy {
        Some(text.to_string())
    } else {
        toml::from_str::<toml::Value>(text)
            .with_context(|| format!("cannot parse {}", path.display()))?
            .get("toolchain")
            .and_then(|toolchain| toolchain.get("channel"))
            .and_then(|channel| channel.as_str())
            .map(str::to_owned)
    };
    channel
        .filter(|channel| !channel.is_empty())
        .with_context(|| format!("{} sets no toolchain.channel", path.display()))
}

/// Whether crucible pins `channel` in data/rust/, so a flake needs no
/// manifest input for it.
pub fn is_pinned(channel: &str) -> bool {
    matches!(channel, "stable" | "beta" | "nightly")
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
