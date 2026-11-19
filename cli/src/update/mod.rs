//! `nix-cargo update`: rewrite `sources.json` and `data/*.json`.
//!
//! Every pin is fetched and checked before the first file is touched, so a
//! failed update leaves the previous pins intact instead of a mixed set. Each
//! file is then replaced atomically, in a fixed order: a half-written pin is
//! worse than a stale one because the flake would fail to evaluate.

pub mod cargo;
pub mod rust;

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Serialize;

/// What `update` refreshes; everything by default, as the scheduled job runs it.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Scope {
    All,
    Cargo,
    Rust,
}

/// The files a run will write, already serialized, in write order.
type Plan = Vec<(PathBuf, Vec<u8>)>;

/// The lock lives in the repo root the run modifies, next to the pins.
const LOCK: &str = ".nix-cargo-update.lock";

pub fn run(scope: Scope, version: Option<String>, root: &Path) -> Result<()> {
    let _lock = Lock::acquire(root)?;
    let http = crate::http::Http::new();

    // Plan everything first: sources.json, then stable, beta and nightly, so a
    // fetch failure anywhere aborts before any pin changes.
    let plan = match scope {
        Scope::All => {
            let mut plan = cargo::plan(&http, version, root)?;
            plan.extend(rust::plan(&http, root)?);
            plan
        }
        Scope::Cargo => cargo::plan(&http, version, root)?,
        Scope::Rust => rust::plan(&http, root)?,
    };

    for (path, bytes) in plan {
        write_atomic(&path, &bytes)?;
    }
    Ok(())
}

/// A run-wide exclusive lock, created next to the pins.
///
/// `create_new` is the atomic test-and-create, and `Drop` removes the file on
/// every exit path, including a failed fetch.
#[must_use = "the lock is released when the guard is dropped"]
struct Lock {
    path: PathBuf,
}

impl Lock {
    fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).with_context(|| format!("failed to create {}", root.display()))?;
        let path = root.join(LOCK);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => bail!(
                "another update is already running: {} exists (remove it if no update is running)",
                path.display()
            ),
            Err(err) => Err(err).with_context(|| format!("failed to create {}", path.display())),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// `value` as 2-space JSON with a trailing newline, the shape every file in the
/// repo already has.
fn json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    Ok(json.into_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nix-cargo-update-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_lock_is_exclusive_and_removed_on_drop() {
        let root = temp_dir("lock");
        let lock = Lock::acquire(&root).unwrap();
        assert!(root.join(LOCK).is_file());
        assert!(Lock::acquire(&root).is_err(), "a second run must fail");

        drop(lock);
        assert!(!root.join(LOCK).exists(), "the guard must remove the lock");

        // And the next run succeeds again.
        drop(Lock::acquire(&root).unwrap());
        fs::remove_dir_all(&root).unwrap();
    }
}
