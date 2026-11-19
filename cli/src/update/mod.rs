//! `nix-cargo update`: rewrite `sources.json` and `data/*.json`.
//!
//! Both writers go through one atomic write helper because a half-written pin
//! is worse than a stale one: the flake would fail to evaluate.

pub mod cargo;
pub mod rust;

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Serialize;

/// What `update` refreshes; everything by default, as the scheduled job runs it.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Scope {
    All,
    Cargo,
    Rust,
}

pub fn run(scope: Scope, version: Option<String>, root: &Path) -> Result<()> {
    let http = crate::http::Http::new();
    match scope {
        Scope::All => {
            cargo::update(&http, version, root)?;
            rust::update(&http, root)
        }
        Scope::Cargo => cargo::update(&http, version, root),
        Scope::Rust => rust::update(&http, root),
    }
}

/// Write `value` as 2-space JSON with a trailing newline, the shape every file
/// in the repo already has.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    write_atomic(path, json.as_bytes())
}

/// Replace `path` in one step, so a crashed or interrupted run never leaves a
/// partially written manifest behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(dir)?;

    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("manifest"),
        std::process::id()
    ));
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}
