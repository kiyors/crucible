//! Rust: the fork of cargo, the rustc and rustup that can shadow it, the
//! variables and caches the fork ignores, and the toolchain file that names
//! the release the project expects.

use std::fs;
use std::path::{Path, PathBuf};

use super::{find_nearest_file, Checks, Env};

const FORK: &str = "huacnlee/cargo";

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let cargo = check_cargo(checks, env);
    let rustc = check_rustc(checks, env);
    check_rustup(checks, env);
    check_env(checks, env, cargo.as_deref());
    check_rust_src(checks, env);
    check_hm_session(checks, env);
    check_cc(checks, env);
    check_toolchain_file(checks, cwd, rustc.as_deref());
    check_targets_dir(checks, env);
}

/// The first cargo on PATH must be the fork, and any other cargo is a trap: it
/// either hides the fork or will hide it as soon as it moves earlier. Returns
/// the cargo that wins.
fn check_cargo(checks: &mut Checks, env: &Env) -> Option<PathBuf> {
    let cargos = env.executables("cargo");
    let Some(first) = cargos.first() else {
        checks.error("no cargo on PATH".to_string());
        return None;
    };

    match env.command("cargo").arg("-V").output() {
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
            "cargo -V failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => checks.error(format!("cannot run cargo: {err}")),
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

/// A compiler must exist even when cargo does: build scripts, rust-analyzer and
/// the toolchain-file check below all need `rustc`, and a missing one makes
/// every pin look fine while nothing compiles. Returns rustc's `-V` line.
fn check_rustc(checks: &mut Checks, env: &Env) -> Option<String> {
    match env.command("rustc").arg("-V").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            checks.ok(format!("rustc is {version}"));
            Some(version)
        }
        Ok(output) => {
            checks.error(format!(
                "rustc -V failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            None
        }
        Err(err) => {
            checks.error(format!("cannot run rustc: {err}"));
            None
        }
    }
}

/// rustup's proxies replace the toolchain's binaries, so its presence is
/// usually an accident waiting to happen.
fn check_rustup(checks: &mut Checks, env: &Env) {
    match env.executables("rustup").first() {
        Some(rustup) => checks.warn(format!(
            "rustup is on PATH ({}); its proxies replace the fork",
            rustup.display()
        )),
        None => checks.ok("rustup is not on PATH".to_string()),
    }
}

fn check_env(checks: &mut Checks, env: &Env, cargo: Option<&Path>) {
    match env.var("CARGO_INCREMENTAL") {
        Some(value) if value == "0" => checks
            .warn("CARGO_INCREMENTAL=0 disables the fork's shared incremental caches".to_string()),
        Some(value) => checks.ok(format!("CARGO_INCREMENTAL={value}")),
        None => checks.ok("CARGO_INCREMENTAL is not set".to_string()),
    }

    match env.var("RUSTC_WRAPPER") {
        Some(wrapper) => checks.warn(format!(
            "RUSTC_WRAPPER={wrapper}; sccache and kache store every dependency a second time"
        )),
        None => checks.ok("RUSTC_WRAPPER is not set".to_string()),
    }

    let target_dirs: Vec<_> = ["CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"]
        .into_iter()
        .filter_map(|name| env.var(name).map(|value| format!("{name}={value}")))
        .collect();
    if target_dirs.is_empty() {
        checks.ok("CARGO_TARGET_DIR and CARGO_BUILD_TARGET_DIR are not set".to_string());
    } else {
        checks.warn(format!(
            "{}; the fork always builds in ./target and ignores it",
            target_dirs.join(", ")
        ));
    }

    // crucible's wrapper sets CARGO_FORK_AUTO_UPDATE=0 for its own process,
    // so the shell's value only matters for a cargo from outside the store.
    let from_store = cargo
        .and_then(|cargo| fs::canonicalize(cargo).ok())
        .is_some_and(|cargo| cargo.starts_with("/nix/store"));
    match env.var("CARGO_FORK_AUTO_UPDATE") {
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
fn check_rust_src(checks: &mut Checks, env: &Env) {
    let sysroot_src =
        rustc_sysroot(env).map(|sysroot| sysroot.join("lib/rustlib/src/rust/library"));
    match env.var("RUST_SRC_PATH") {
        None => match sysroot_src.filter(|src| has_std_sources(src)) {
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
            } else if !has_std_sources(&path) {
                // A directory that exists but holds no `core` is not usable
                // sources, so it must not count as a match.
                checks.warn(format!(
                    "RUST_SRC_PATH ({}) has no standard library sources (core/src/lib.rs is missing)",
                    path.display()
                ));
            } else if sysroot_src
                .as_deref()
                .is_some_and(|src| has_std_sources(src) && !same_path(src, &path))
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

/// Variables the checks above judge, and that Home Manager configs commonly
/// set. Not CARGO_FORK_AUTO_UPDATE: crucible's cargo wrapper exports it to
/// everything cargo runs, and it does no harm when stale.
const SESSION_VARS: [&str; 5] = [
    "RUSTC_WRAPPER",
    "CARGO_INCREMENTAL",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "RUST_SRC_PATH",
];

/// A shell keeps the session variables it started with: hm-session-vars.sh
/// only runs once per session, and tmux or zellij servers hand their old
/// environment to every new pane. So a variable a switch removed (a kache
/// RUSTC_WRAPPER, the module's old RUST_SRC_PATH) lives on and the checks above
/// blame the current config for it. This is only a hint: the variable may as
/// well come from a shell rc file or a dev shell.
fn check_hm_session(checks: &mut Checks, env: &Env) {
    if env.var("__HM_SESS_VARS_SOURCED").is_none() {
        return;
    }
    // Without the file there is nothing to compare against (no Home Manager
    // profile here, CI), and silence beats a guess.
    let Some(text) = hm_session_vars_files(env)
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
    else {
        return;
    };
    // A dev shell's toolchain sets RUST_SRC_PATH itself.
    let in_nix_shell = env.var("IN_NIX_SHELL").is_some();
    let stale = stale_session_vars(&text, |name| {
        env.var(name).is_some() && !(in_nix_shell && name == "RUST_SRC_PATH")
    });
    if !stale.is_empty() {
        let (verb, pronoun) = if stale.len() == 1 {
            ("is", "it")
        } else {
            ("are", "them")
        };
        checks.info(format!(
            "{} {verb} set but not in the current hm-session-vars.sh; if Home Manager set \
             {pronoun} before the last switch, this shell predates it: open a new login shell \
             (and restart tmux or zellij, which keep their old environment)",
            stale.join(", "),
        ));
    }
}

/// Where Home Manager installs its session variables: nix-darwin and NixOS
/// (`useUserPackages`), a standalone profile, and its XDG variant.
fn hm_session_vars_files(env: &Env) -> Vec<PathBuf> {
    const FILE: &str = "etc/profile.d/hm-session-vars.sh";
    let mut files = Vec::new();
    if let Some(user) = env.var("USER") {
        files.push(Path::new("/etc/profiles/per-user").join(user).join(FILE));
    }
    if let Some(home) = env.var_os("HOME").map(PathBuf::from) {
        files.push(home.join(".nix-profile").join(FILE));
        files.push(home.join(".local/state/nix/profile").join(FILE));
    }
    files
}

/// The SESSION_VARS that `is_set` but that `hm_session_vars` (the script's
/// text) does not export.
fn stale_session_vars(hm_session_vars: &str, is_set: impl Fn(&str) -> bool) -> Vec<&'static str> {
    SESSION_VARS
        .into_iter()
        .filter(|name| is_set(name))
        .filter(|name| {
            !hm_session_vars.lines().any(|line| {
                let line = line.trim_start();
                let line = line.strip_prefix("export ").unwrap_or(line);
                line.strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with('='))
            })
        })
        .collect()
}

/// On macOS, build scripts that compile C or Objective-C (aws-lc-sys, objc2,
/// anything using Apple frameworks) need Apple's clang; a Homebrew or Nix GCC
/// first on PATH as `cc` fails them with unrecognised flags.
fn check_cc(checks: &mut Checks, env: &Env) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(cc) = env.executables("cc").into_iter().next() else {
        checks.info("no cc on PATH; rustc links and build scripts compile with it".to_string());
        return;
    };
    let version = match std::process::Command::new(&cc).arg("--version").output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(err) => {
            checks.warn(format!("cannot run {}: {err}", cc.display()));
            return;
        }
    };
    let first_line = version.lines().next().unwrap_or_default();
    if is_gcc(&version) {
        checks.warn(format!(
            "{} is GCC ({first_line}); crates such as aws-lc-sys and Objective-C build scripts \
             need Apple clang: put /usr/bin earlier on PATH or set CC=/usr/bin/cc",
            cc.display()
        ));
    } else {
        checks.ok(format!("{} is {first_line}", cc.display()));
    }
}

/// Whether `cc --version` output is GCC's. Any clang (Apple's or a Nix
/// wrapper's) says so in its first line, while GCC names itself or its
/// copyright holder.
fn is_gcc(version: &str) -> bool {
    let version = version.to_lowercase();
    !version.contains("clang")
        && (version.contains("gcc") || version.contains("free software foundation"))
}

/// rust-analyzer needs `core` to be unpacked; a `library` directory on its own
/// is not sources.
fn has_std_sources(path: &Path) -> bool {
    path.join("core/src/lib.rs").is_file()
}

fn rustc_sysroot(env: &Env) -> Option<PathBuf> {
    let output = env
        .command("rustc")
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

/// The toolchain file rustup would use: the nearest `rust-toolchain.toml` or
/// `rust-toolchain` in `dir` or one of its ancestors.
fn find_toolchain_file(dir: &Path) -> Option<PathBuf> {
    find_nearest_file(dir, &["rust-toolchain.toml", "rust-toolchain"])
}

/// A toolchain file names the release the project expects; if the shell's
/// rustc is another one, the two disagree.
fn check_toolchain_file(checks: &mut Checks, cwd: &Path, rustc: Option<&str>) {
    let Some(path) = find_toolchain_file(cwd) else {
        checks.ok("no rust-toolchain file here or in a parent directory".to_string());
        return;
    };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    // As rustup and lib/rust/toolchain.nix decide it: `rust-toolchain` without an
    // extension may still be TOML; only a single bare line is the legacy form.
    let text = text.trim();
    let legacy = path.extension().is_none() && !text.contains(['\n', '[', '=']);
    let channel = if legacy {
        Some(text.to_string())
    } else {
        match toml::from_str::<toml::Value>(text) {
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

    // A missing rustc was already reported by check_rustc.
    let Some(rustc) = rustc.and_then(|line| line.split_whitespace().nth(1)) else {
        return;
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

fn is_numbered(channel: &str) -> bool {
    channel
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
        && channel
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

/// The fork's build caches can grow without bound; this is the number to look
/// at when a disk fills up.
fn check_targets_dir(checks: &mut Checks, env: &Env) {
    let Some(home) = env.var_os("HOME").map(PathBuf::from) else {
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
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn finds_the_nearest_toolchain_file_upwards() {
        let root =
            std::env::temp_dir().join(format!("crucible-doctor-rust-{}", std::process::id()));
        let nested = root.join("crates/app/src");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_toolchain_file(&nested), None);

        fs::write(root.join("rust-toolchain"), "stable\n").unwrap();
        assert_eq!(
            find_toolchain_file(&nested),
            Some(root.join("rust-toolchain"))
        );

        // The nearer file wins, and .toml wins within one directory.
        let app = root.join("crates/app");
        fs::write(app.join("rust-toolchain"), "1.99\n").unwrap();
        fs::write(
            app.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.99\"\n",
        )
        .unwrap();
        assert_eq!(
            find_toolchain_file(&nested),
            Some(app.join("rust-toolchain.toml"))
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_session_variables_the_last_switch_dropped() {
        let hm = "\
# Only source this once.
if [ -n \"${__HM_SESS_VARS_SOURCED-}\" ]; then return; fi
export __HM_SESS_VARS_SOURCED=1
export CARGO_HOME=\"/home/me/.cargo\"
export CARGO_INCREMENTAL=\"1\"
export RUSTC_WRAPPER_LOG=\"debug\"
";
        let set = ["RUSTC_WRAPPER", "CARGO_INCREMENTAL", "RUST_SRC_PATH"];
        // RUSTC_WRAPPER_LOG must not count as RUSTC_WRAPPER.
        assert_eq!(
            stale_session_vars(hm, |name| set.contains(&name)),
            ["RUSTC_WRAPPER", "RUST_SRC_PATH"]
        );
        assert!(stale_session_vars(hm, |_| false).is_empty());
        assert!(stale_session_vars(hm, |name| name == "CARGO_INCREMENTAL").is_empty());
    }

    #[test]
    fn tells_gcc_from_clang() {
        assert!(is_gcc(
            "gcc-14 (Homebrew GCC 14.2.0) 14.2.0\nCopyright (C) 2024 Free Software Foundation, Inc.\n"
        ));
        assert!(is_gcc(
            "cc (GCC) 14.2.0\nCopyright (C) 2024 Free Software Foundation, Inc.\n"
        ));
        assert!(!is_gcc(
            "Apple clang version 21.0.0 (clang-2100.4.1.1)\nTarget: arm64-apple-darwin27.2.0\n"
        ));
        assert!(!is_gcc(
            "clang version 19.1.7\nTarget: arm64-apple-darwin\n"
        ));
    }

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

    #[test]
    #[cfg(unix)]
    fn checks_the_fork_and_another_cargo() {
        let root = scratch("rust-cargo");
        let (fork, other, project) = (root.join("fork"), root.join("other"), root.join("p"));
        write_executable(
            &fork.join("cargo"),
            b"#!/bin/sh\necho 'cargo 1.99.0 (huacnlee/cargo 1.99.0)'\n",
        );
        write_executable(
            &fork.join("rustc"),
            b"#!/bin/sh\nif [ \"$1\" = -V ]; then echo 'rustc 1.99.0 (abcdef 2026-01-01)'; exit 0; fi\n\
              if [ \"$1\" = --print ]; then echo /nonexistent/sysroot; exit 0; fi\nexit 1\n",
        );
        write_executable(&other.join("cargo"), b"#!/bin/sh\necho 'cargo 1.99.0'\n");
        fs::create_dir_all(&project).unwrap();

        let env = fake_env(&[&fork, &other], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "is huacnlee/cargo (cargo 1.99.0 (huacnlee/cargo 1.99.0))",
        );
        assert_line(&lines, "another cargo is on PATH: ");
        assert_eq!(errors, 0);

        // A cargo that is not the fork is an error even when it is first.
        let env = fake_env(&[&other], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "is not huacnlee/cargo (cargo 1.99.0)");
        assert_line(&lines, "rustup is not on PATH");
        assert_eq!(errors, 2);

        // With no cargo at all, the missing compiler is reported once.
        let env = fake_env(&[&root.join("empty")], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "no cargo on PATH");
        assert_eq!(errors, 2);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn checks_rustc_rustup_and_the_cargo_variables() {
        let root = scratch("rust-tools");
        let (bin, project) = (root.join("bin"), root.join("p"));
        write_executable(
            &bin.join("rustc"),
            b"#!/bin/sh\nif [ \"$1\" = -V ]; then echo 'rustc 1.99.0 (abcdef 2026-01-01)'; exit 0; fi\n\
              if [ \"$1\" = --print ]; then echo /nonexistent/sysroot; exit 0; fi\nexit 1\n",
        );
        write_executable(&bin.join("rustup"), b"#!/bin/sh\necho rustup 1.28\n");
        write_executable(
            &bin.join("cargo"),
            b"#!/bin/sh\necho 'cargo 1.99.0 (huacnlee/cargo 1.99.0)'\n",
        );
        fs::create_dir_all(&project).unwrap();

        let env = fake_env(
            &[&bin],
            &[
                ("CARGO_INCREMENTAL", "0"),
                ("RUSTC_WRAPPER", "sccache"),
                ("CARGO_TARGET_DIR", "/tmp/target"),
                ("HOME", root.join("home").to_str().unwrap()),
            ],
        );
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "rustc is rustc 1.99.0 (abcdef 2026-01-01)");
        assert_line(&lines, "rustup is on PATH");
        assert_line(
            &lines,
            "CARGO_INCREMENTAL=0 disables the fork's shared incremental caches",
        );
        assert_line(&lines, "RUSTC_WRAPPER=sccache; sccache and kache store");
        assert_line(
            &lines,
            "CARGO_TARGET_DIR=/tmp/target; the fork always builds in ./target",
        );
        assert_line(&lines, "RUST_SRC_PATH is not set and rustc has no rust-src");
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn compares_the_toolchain_file_with_rustc() {
        let root = scratch("rust-toolchain-file");
        let bin = root.join("bin");
        let project = root.join("p");
        write_executable(
            &bin.join("cargo"),
            b"#!/bin/sh\necho 'cargo 1.99.0 (huacnlee/cargo 1.99.0)'\n",
        );
        write_executable(
            &bin.join("rustc"),
            b"#!/bin/sh\nif [ \"$1\" = -V ]; then echo 'rustc 1.99.0 (abcdef 2026-01-01)'; exit 0; fi\n\
              if [ \"$1\" = --print ]; then echo /nonexistent/sysroot; exit 0; fi\nexit 1\n",
        );
        fs::create_dir_all(&project).unwrap();
        let env = fake_env(&[&bin], &[("HOME", root.join("home").to_str().unwrap())]);

        fs::write(
            project.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.99\"\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "pins 1.99, which matches rustc 1.99.0");
        assert_eq!(errors, 0);

        fs::write(
            project.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.98\"\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "pins 1.98, but rustc is 1.99.0");
        assert_eq!(errors, 1);

        // An alias is reported but not compared against a numbered rustc.
        fs::remove_file(project.join("rust-toolchain.toml")).unwrap();
        fs::write(project.join("rust-toolchain"), "stable\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "pins channel stable, which is not a numbered channel",
        );
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sizes_the_forks_build_cache() {
        let root = scratch("rust-targets");
        let home = root.join("home");
        let cache = home.join(".cargo/targets");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("blob"), vec![0; 2048]).unwrap();
        let env = fake_env(&[], &[("HOME", home.to_str().unwrap())]);
        let (lines, errors) = run_check(|checks| check_targets_dir(checks, &env));
        assert_line(&lines, "holds 2.0 KiB");
        assert_eq!(errors, 0);

        let env = fake_env(&[], &[]);
        let (lines, _) = run_check(|checks| check_targets_dir(checks, &env));
        assert_line(&lines, "HOME is not set; cannot size ~/.cargo/targets");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compares_the_active_sysroot_sources() {
        assert!(!has_std_sources(Path::new("/nonexistent")));
        let root = scratch("rust-src");
        let a = root.join("a");
        let b = root.join("b");
        fs::create_dir_all(a.join("core/src")).unwrap();
        fs::create_dir_all(b.join("core/src")).unwrap();
        fs::write(a.join("core/src/lib.rs"), "").unwrap();
        fs::write(b.join("core/src/lib.rs"), "").unwrap();
        assert!(has_std_sources(&a));
        assert!(!same_path(&a, &b));
        fs::remove_dir_all(&root).unwrap();
    }
}
