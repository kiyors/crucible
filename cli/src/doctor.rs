//! `nix-cargo doctor`: explain why a machine is not using the fork the way
//! this flake intends.
//!
//! Every check mirrors something the README warns about: other `cargo` binaries
//! earlier or later on PATH, rustup's proxies, settings the fork ignores, and
//! caches that end up storing everything twice.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;

const FORK: &str = "huacnlee/cargo";

pub fn run() -> Result<ExitCode> {
    let cwd = env::current_dir()?;
    let mut checks = Checks::default();

    let cargo = check_cargo(&mut checks);
    check_rustup(&mut checks);
    check_env(&mut checks, cargo.as_deref());
    check_rust_src(&mut checks);
    check_toolchain_file(&mut checks, &cwd);
    check_targets_dir(&mut checks);

    Ok(if checks.errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[derive(Default)]
struct Checks {
    errors: u32,
}

impl Checks {
    fn ok(&self, message: String) {
        Self::line("ok", &message);
    }

    fn warn(&self, message: String) {
        Self::line("warn", &message);
    }

    fn info(&self, message: String) {
        Self::line("info", &message);
    }

    fn error(&mut self, message: String) {
        self.errors += 1;
        Self::line("error", &message);
    }

    fn line(level: &str, message: &str) {
        println!("{level:<5} {message}");
    }
}

/// The first cargo on PATH must be the fork, and any other cargo is a trap: it
/// either hides the fork or will hide it as soon as it moves earlier. Returns
/// the cargo that wins.
fn check_cargo(checks: &mut Checks) -> Option<PathBuf> {
    let cargos = executables("cargo");
    let Some(first) = cargos.first() else {
        checks.error("no cargo on PATH".to_string());
        return None;
    };

    match Command::new(first).arg("-V").output() {
        Ok(output) if output.status.success() => {
            let version = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            if version.contains(FORK) {
                checks.ok(format!("{} is {FORK} ({version})", first.display()));
            } else {
                checks.error(format!(
                    "{} is not {FORK} ({version}); put the flake's toolchain first on PATH",
                    first.display()
                ));
            }
        }
        Ok(output) => checks.error(format!(
            "{} -V failed: {}",
            first.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => checks.error(format!("cannot run {}: {err}", first.display())),
    }

    if cargos.len() == 1 {
        checks.ok("no other cargo on PATH".to_string());
    }
    for other in &cargos[1..] {
        checks.warn(format!(
            "another cargo is on PATH: {} (the first one, {}, wins)",
            other.display(),
            first.display()
        ));
    }
    Some(first.clone())
}

/// rustup's proxies replace the toolchain's binaries, so its presence is
/// usually an accident waiting to happen.
fn check_rustup(checks: &mut Checks) {
    match executables("rustup").first() {
        Some(rustup) => checks.warn(format!(
            "rustup is on PATH ({}); its proxies replace the fork",
            rustup.display()
        )),
        None => checks.ok("rustup is not on PATH".to_string()),
    }
}

fn check_env(checks: &mut Checks, cargo: Option<&Path>) {
    match env_value("CARGO_INCREMENTAL") {
        Some(value) if value == "0" => checks
            .warn("CARGO_INCREMENTAL=0 disables the fork's shared incremental caches".to_string()),
        Some(value) => checks.ok(format!("CARGO_INCREMENTAL={value}")),
        None => checks.ok("CARGO_INCREMENTAL is not set".to_string()),
    }

    match env_value("RUSTC_WRAPPER") {
        Some(wrapper) => checks.warn(format!(
            "RUSTC_WRAPPER={wrapper}; sccache and kache store every dependency a second time"
        )),
        None => checks.ok("RUSTC_WRAPPER is not set".to_string()),
    }

    let target_dirs: Vec<_> = ["CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"]
        .into_iter()
        .filter_map(|name| env_value(name).map(|value| format!("{name}={value}")))
        .collect();
    if target_dirs.is_empty() {
        checks.ok("CARGO_TARGET_DIR and CARGO_BUILD_TARGET_DIR are not set".to_string());
    } else {
        checks.warn(format!(
            "{}; the fork always builds in ./target and ignores it",
            target_dirs.join(", ")
        ));
    }

    // nix-cargo's wrapper sets CARGO_FORK_AUTO_UPDATE=0 for its own process,
    // so the shell's value only matters for a cargo from outside the store.
    let from_store = cargo
        .and_then(|cargo| fs::canonicalize(cargo).ok())
        .is_some_and(|cargo| cargo.starts_with("/nix/store"));
    match env_value("CARGO_FORK_AUTO_UPDATE") {
        _ if from_store => {
            checks.ok("self-update is off: the cargo in use is from the Nix store".to_string())
        }
        Some(value) if value == "0" => checks.ok("CARGO_FORK_AUTO_UPDATE=0".to_string()),
        Some(value) => checks.info(format!(
            "CARGO_FORK_AUTO_UPDATE={value}; the fork may try to replace itself"
        )),
        None => checks.info(
            "CARGO_FORK_AUTO_UPDATE is not 0; the fork may try to replace itself".to_string(),
        ),
    }
}

/// rust-analyzer indexes the standard library from RUST_SRC_PATH, so it has to
/// be the active rustc's own; another release's sources give wrong types and
/// missing items rather than an error.
fn check_rust_src(checks: &mut Checks) {
    let sysroot_src = rustc_sysroot().map(|sysroot| sysroot.join("lib/rustlib/src/rust/library"));
    match env_value("RUST_SRC_PATH") {
        None => match sysroot_src.filter(|src| src.exists()) {
            Some(src) => checks.ok(format!(
                "RUST_SRC_PATH is not set; rustc's sysroot has the sources ({})",
                src.display()
            )),
            None => checks.info("RUST_SRC_PATH is not set and rustc has no rust-src".to_string()),
        },
        Some(value) => {
            let path = PathBuf::from(&value);
            if !path.exists() {
                checks.error(format!(
                    "RUST_SRC_PATH points at {}, which does not exist",
                    path.display()
                ));
            } else if sysroot_src
                .as_deref()
                .is_some_and(|src| !same_path(src, &path))
            {
                checks.warn(format!(
                    "RUST_SRC_PATH ({}) is not the active rustc's sources ({}); rust-analyzer will index another release",
                    path.display(),
                    sysroot_src.unwrap_or_default().display()
                ));
            } else {
                checks.ok(format!("RUST_SRC_PATH matches rustc ({})", path.display()));
            }
        }
    }
}

fn rustc_sysroot() -> Option<PathBuf> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    Some(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Whether two source trees are the same sources. Toolchains are symlink trees,
/// so two of them can hold the same rust-src under different directories: a
/// file inside resolves to the one copy both point at.
fn same_path(a: &Path, b: &Path) -> bool {
    let probe = |dir: &Path| fs::canonicalize(dir.join("core/src/lib.rs")).ok();
    match (probe(a), probe(b)) {
        (Some(a), Some(b)) => a == b,
        _ => fs::canonicalize(a).ok() == fs::canonicalize(b).ok(),
    }
}

/// A `rust-toolchain` file overrides the dev shell; if it names another
/// release, the shell's rustc and the project's expectations disagree.
fn check_toolchain_file(checks: &mut Checks, cwd: &Path) {
    let Some(path) = ["rust-toolchain.toml", "rust-toolchain"]
        .into_iter()
        .map(|name| cwd.join(name))
        .find(|path| path.is_file())
    else {
        checks.ok("no rust-toolchain file in the current directory".to_string());
        return;
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let channel = if text.contains("[toolchain]") {
        match toml::from_str::<toml::Value>(&text) {
            Ok(value) => value
                .get("toolchain")
                .and_then(|toolchain| toolchain.get("channel"))
                .and_then(|channel| channel.as_str())
                .map(str::to_owned),
            Err(err) => {
                checks.error(format!("cannot parse {}: {err}", path.display()));
                return;
            }
        }
    } else {
        Some(text.trim().to_string())
    };
    let Some(channel) = channel.filter(|channel| !channel.is_empty()) else {
        checks.error(format!("{} sets no toolchain.channel", path.display()));
        return;
    };

    if !is_numbered(&channel) {
        checks.info(format!(
            "{} pins channel {channel}, which is not a numbered channel; not compared with rustc",
            path.display()
        ));
        return;
    }

    let rustc = match Command::new("rustc").arg("-V").output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string(),
        Ok(output) => {
            checks.error(format!(
                "rustc -V failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            return;
        }
        Err(err) => {
            checks.error(format!("cannot run rustc: {err}"));
            return;
        }
    };

    // "1.99" and "1.99.0" both pin the 1.99 series, so a prefix match is right;
    // "1.99.1" against "1.99.0" is not.
    if rustc == channel || rustc.starts_with(&format!("{channel}.")) {
        checks.ok(format!(
            "{} pins {channel}, which matches rustc {rustc}",
            path.display()
        ));
    } else {
        checks.error(format!(
            "{} pins {channel}, but rustc is {rustc}",
            path.display()
        ));
    }
}

/// The fork's build caches can grow without bound; this is the number to look
/// at when a disk fills up.
fn check_targets_dir(checks: &mut Checks) {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        checks.info("HOME is not set; cannot size ~/.cargo/targets".to_string());
        return;
    };
    let dir = home.join(".cargo").join("targets");
    if !dir.is_dir() {
        checks.info(format!("{} does not exist yet", dir.display()));
        return;
    }
    match dir_size(&dir) {
        Ok(size) => checks.info(format!("{} holds {}", dir.display(), human_size(size))),
        Err(err) => checks.warn(format!("cannot size {}: {err}", dir.display())),
    }
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn is_numbered(channel: &str) -> bool {
    channel
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
        && channel
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

/// Every executable called `name` on PATH, in PATH order, with symlinked
/// duplicates (nix profiles) collapsed.
fn executables(name: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let Some(path) = env::var_os("PATH") else {
        return found;
    };
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if !is_executable(&candidate) {
            continue;
        }
        let key = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        if seen.insert(key) {
            found.push(candidate);
        }
    }
    found
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

/// Sum the files under `root`, skipping symlinks so a link to a shared cache
/// is not counted twice.
fn dir_size(root: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(human_size(1536), "1.5 KiB");
    }

    #[test]
    fn numbered_channels_are_the_ones_to_compare() {
        assert!(is_numbered("1.99"));
        assert!(is_numbered("1.99.0"));
        assert!(!is_numbered("stable"));
        assert!(!is_numbered("nightly-2026-11-01"));
    }
}
