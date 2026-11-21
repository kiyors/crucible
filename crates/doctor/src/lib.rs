//! `crucible doctor`: explain why a machine is not using the fork the way
//! this flake intends.
//!
//! Every check mirrors something the README warns about: other `cargo` binaries
//! earlier or later on PATH, rustup's proxies, settings the fork ignores,
//! caches that end up storing everything twice, a shell older than the last
//! Home Manager switch, and a C compiler that cannot build some crates. The
//! other languages (Node.js here, Python, Go, Zig and Deno in their own
//! modules) compare the active toolchain with the project's pins and look for
//! the settings lib/<lang> and the Home Manager module warn about.

mod deno;
mod go;
mod python;
mod zig;

use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;

const FORK: &str = "huacnlee/cargo";

pub fn run() -> Result<ExitCode> {
    let cwd = env::current_dir()?;
    let mut checks = Checks::default();

    let cargo = check_cargo(&mut checks);
    let rustc = check_rustc(&mut checks);
    check_rustup(&mut checks);
    check_env(&mut checks, cargo.as_deref());
    check_rust_src(&mut checks);
    check_hm_session(&mut checks);
    check_cc(&mut checks);
    check_toolchain_file(&mut checks, &cwd, rustc.as_deref());
    check_targets_dir(&mut checks);
    let node = check_node(&mut checks);
    check_node_env(&mut checks);
    check_node_version_file(&mut checks, &cwd, node.as_deref());
    check_package_json(&mut checks, &cwd);
    // Each of these is silent when neither its executable nor its project
    // files are here, so a Rust-only project sees no Python or Go lines.
    let env = Env::process();
    python::check(&mut checks, &cwd, &env);
    go::check(&mut checks, &cwd, &env);
    zig::check(&mut checks, &cwd, &env);
    deno::check(&mut checks, &cwd, &env);

    Ok(if checks.errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[derive(Default)]
struct Checks {
    errors: u32,
    /// Every line printed, for the tests to read back.
    #[cfg(test)]
    lines: Vec<String>,
}

impl Checks {
    fn ok(&mut self, message: String) {
        self.line("ok", &message);
    }

    fn warn(&mut self, message: String) {
        self.line("warn", &message);
    }

    fn info(&mut self, message: String) {
        self.line("info", &message);
    }

    fn error(&mut self, message: String) {
        self.errors += 1;
        self.line("error", &message);
    }

    fn line(&mut self, level: &str, message: &str) {
        let line = format!("{level:<5} {message}");
        println!("{line}");
        #[cfg(test)]
        self.lines.push(line);
    }
}

/// PATH and the variables the language checks read. The process's own in
/// `run`; the tests pass theirs instead of changing the real environment,
/// which other tests share.
struct Env {
    path: OsString,
    /// None reads the process environment.
    vars: Option<HashMap<String, String>>,
}

impl Env {
    fn process() -> Self {
        Env {
            path: env::var_os("PATH").unwrap_or_default(),
            vars: None,
        }
    }

    /// A variable's value; an empty one counts as unset, as in `env_value`.
    fn var(&self, name: &str) -> Option<String> {
        match &self.vars {
            None => env_value(name),
            Some(vars) => vars.get(name).filter(|value| !value.is_empty()).cloned(),
        }
    }

    fn executables(&self, name: &str) -> Vec<PathBuf> {
        executables_in(name, &self.path)
    }

    /// Whether `dir` is on PATH, as itself or through a symlink.
    fn on_path(&self, dir: &Path) -> bool {
        let real = fs::canonicalize(dir).ok();
        env::split_paths(&self.path)
            .any(|entry| entry == dir || real.is_some() && fs::canonicalize(&entry).ok() == real)
    }
}

/// What `exe args` prints on stdout, trimmed, with `envs` added to its
/// environment. A toolchain that cannot say its version will not build
/// anything either, so a failure is an error.
fn stdout_of(
    checks: &mut Checks,
    exe: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Option<String> {
    match Command::new(exe)
        .args(args)
        .envs(envs.iter().copied())
        .output()
    {
        Ok(output) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        Ok(output) => {
            checks.error(format!(
                "{} {} failed: {}",
                exe.display(),
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            None
        }
        Err(err) => {
            checks.error(format!("cannot run {}: {err}", exe.display()));
            None
        }
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

/// A compiler must exist even when cargo does: build scripts, rust-analyzer and
/// the toolchain-file check below all need `rustc`, and a missing one makes
/// every pin look fine while nothing compiles. Returns rustc's `-V` line.
fn check_rustc(checks: &mut Checks) -> Option<String> {
    match Command::new("rustc").arg("-V").output() {
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

    // crucible's wrapper sets CARGO_FORK_AUTO_UPDATE=0 for its own process,
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
fn check_hm_session(checks: &mut Checks) {
    if env_value("__HM_SESS_VARS_SOURCED").is_none() {
        return;
    }
    // Without the file there is nothing to compare against (no Home Manager
    // profile here, CI), and silence beats a guess.
    let Some(text) = hm_session_vars_files()
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
    else {
        return;
    };
    // A dev shell's toolchain sets RUST_SRC_PATH itself.
    let in_nix_shell = env_value("IN_NIX_SHELL").is_some();
    let stale = stale_session_vars(&text, |name| {
        env_value(name).is_some() && !(in_nix_shell && name == "RUST_SRC_PATH")
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
fn hm_session_vars_files() -> Vec<PathBuf> {
    const FILE: &str = "etc/profile.d/hm-session-vars.sh";
    let mut files = Vec::new();
    if let Some(user) = env_value("USER") {
        files.push(Path::new("/etc/profiles/per-user").join(user).join(FILE));
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
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
fn check_cc(checks: &mut Checks) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(cc) = executables("cc").into_iter().next() else {
        checks.info("no cc on PATH; rustc links and build scripts compile with it".to_string());
        return;
    };
    let version = match Command::new(&cc).arg("--version").output() {
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

/// The first node on PATH is the one projects run. Crucible's toolchain is
/// authoritative in a dev shell; outside one, another node wins and vp may
/// download its own. Nodes further down PATH are only a hazard when the first
/// disappears, so they are a warning, not an error.
fn check_node(checks: &mut Checks) -> Option<String> {
    let nodes = executables("node");
    let Some(first) = nodes.first() else {
        checks.info("no node on PATH; Node.js checks skipped".to_string());
        return None;
    };
    let version = stdout_of(checks, first, &["--version"], &[])?;
    checks.ok(format!("{} is node {version}", first.display()));
    for message in shadowed("node", &nodes) {
        checks.warn(message);
    }
    Some(version)
}

/// State that leaks between projects or makes vp ignore the toolchain's
/// Node.js. Crucible's setup hook clears the two vp variables in a dev shell;
/// outside one, a stale value changes what vp does.
fn check_node_env(checks: &mut Checks) {
    match env_value("VP_HOME") {
        Some(value) => checks.warn(format!(
            "VP_HOME={value}; a standalone vp install shares its config with this one, and a \
             managed-Node config stops vp with VP_ENV_FORBIDDEN=1"
        )),
        None => checks.ok("VP_HOME is not set".to_string()),
    }

    // The setup hook exports 1; 0 lets vp fetch a Node.js of its own.
    match env_value("VP_ENV_FORBIDDEN") {
        Some(value) if value == "1" => checks.ok("VP_ENV_FORBIDDEN=1".to_string()),
        Some(value) => checks.warn(format!(
            "VP_ENV_FORBIDDEN={value}; crucible's vp needs 1 to use the Node.js on PATH"
        )),
        None => checks.info(
            "VP_ENV_FORBIDDEN is not set; vp may manage its own Node.js outside a dev shell"
                .to_string(),
        ),
    }

    match env_value("NODE_PATH") {
        Some(value) => checks.warn(format!(
            "NODE_PATH={value}; it adds modules to every project from outside its own node_modules"
        )),
        None => checks.ok("NODE_PATH is not set".to_string()),
    }

    // npm installs globally into its own prefix, which is in the read-only
    // store for a Nix toolchain; NPM_CONFIG_PREFIX is the supported escape.
    let prefix = env_value("NPM_CONFIG_PREFIX").or_else(|| env_value("npm_config_prefix"));
    match prefix {
        Some(value) if value.starts_with("/nix/store") => checks.warn(format!(
            "npm's global prefix is {value}, inside the read-only Nix store; \
             npm install --global will fail"
        )),
        Some(value) => checks.ok(format!("npm's global prefix is {value}")),
        None => checks.info("npm's global prefix is its own (read-only) store path".to_string()),
    }
}

/// `.node-version` and `.nvmrc` are the pins version managers read. Compare
/// the nearest one with the active node, the same way a rust-toolchain file is
/// compared with rustc.
fn check_node_version_file(checks: &mut Checks, cwd: &Path, node: Option<&str>) {
    let Some(path) = find_nearest_file(cwd, &[".node-version", ".nvmrc"]) else {
        checks.ok("no .node-version or .nvmrc here or in a parent directory".to_string());
        return;
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let Some(pin) = first_version_line(&text) else {
        checks.error(format!("{} names no version", path.display()));
        return;
    };
    match version_matches(&pin, node.unwrap_or_default()) {
        Some(true) => checks.ok(format!(
            "{} pins {pin}, which matches node {}",
            path.display(),
            node.unwrap_or_default()
        )),
        Some(false) => checks.error(format!(
            "{} pins {pin}, but node is {}",
            path.display(),
            node.unwrap_or_default()
        )),
        // Aliases such as `lts/*` and `node` are resolved by the toolchain,
        // not by a version we can compare here.
        None => checks.info(format!(
            "{} pins {pin}, which is an alias or a range doctor cannot read; not compared with node",
            path.display()
        )),
    }
}

/// packageManager is corepack's pin for the project's package manager. A
/// different one on PATH means the lockfile and the shell disagree.
fn check_package_json(checks: &mut Checks, cwd: &Path) {
    let Some(path) = find_nearest_file(cwd, &["package.json"]) else {
        checks.ok("no package.json here or in a parent directory".to_string());
        return;
    };
    let json: serde_json::Value = match fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from))
    {
        Ok(json) => json,
        Err(err) => {
            checks.error(format!("cannot parse {}: {err}", path.display()));
            return;
        }
    };
    let Some((field, spec)) = package_manager_spec(&json) else {
        checks.ok(format!(
            "{} sets no packageManager or devEngines.packageManager",
            path.display()
        ));
        return;
    };
    let Some((name, wanted)) = parse_package_manager(&spec) else {
        checks.error(format!(
            "{} sets {field} {spec}, which is not <name>@<version>",
            path.display()
        ));
        return;
    };
    let pin = format!("{} pins {spec} in {field}", path.display());
    let fix = format!(
        "use crucible's `packageManager = \"{name}@{}\"` in `lib.<system>.node.toolchain` (or \
         `fromPackageJson`), or pass a package built from the release you need",
        major_of(&wanted).unwrap_or("<major>")
    );

    let managers = executables(&name);
    let Some(first) = managers.first() else {
        checks.warn(format!("{pin}, but no {name} is on PATH; {fix}"));
        return;
    };
    for message in shadowed(&name, &managers) {
        checks.warn(message);
    }

    match Command::new(first).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let actual = stdout.trim();
            match version_matches(&wanted, actual) {
                Some(true) => checks.ok(format!("{} is {name} {actual}, which matches {spec}", first.display())),
                Some(false) => checks.warn(format!(
                    "{} is {name} {actual}, but {pin} (lockfile drift or a shim); {fix}",
                    first.display()
                )),
                None => checks.info(format!(
                    "{pin}, which doctor cannot compare with {name} {actual} ({}); check it by hand",
                    first.display()
                )),
            }
        }
        Ok(output) => checks.warn(format!(
            "{} --version failed: {}",
            first.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => checks.warn(format!("cannot run {}: {err}", first.display())),
    }

    // A missing node was already reported by check_node.
    if let Some(node) = executables("node").first() {
        check_manager_node(checks, &name, first, node);
    }
}

/// The package manager spec package.json asks for, and the field it came
/// from: `packageManager`, then the first `devEngines.packageManager` entry
/// with a name and a version, the order fromPackageJson in
/// lib/node/toolchain.nix (and corepack) uses.
fn package_manager_spec(json: &serde_json::Value) -> Option<(&'static str, String)> {
    if let Some(spec) = json.get("packageManager").and_then(|v| v.as_str()) {
        return Some(("packageManager", spec.to_string()));
    }
    // devEngines fields are one { name, version } or a list of them.
    let field = json.get("devEngines")?.get("packageManager")?;
    let entries = match field {
        serde_json::Value::Array(entries) => entries.iter().collect(),
        entry => vec![entry],
    };
    entries.into_iter().find_map(|entry| {
        let name = entry.get("name")?.as_str()?;
        let version = entry.get("version")?.as_str()?;
        Some(("devEngines.packageManager", format!("{name}@{version}")))
    })
}

/// The leading major of a spec's version (`^10.2` -> `10`), for the hint
/// that names crucible's `packageManager` argument.
fn major_of(wanted: &str) -> Option<&str> {
    let rest = wanted.trim_start_matches(|c: char| !c.is_ascii_digit());
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Warnings for every copy of `name` after the first in `found` (as
/// `executables` returns them, so with symlinks to one file collapsed): the
/// first one wins, and the others are a trap as soon as PATH changes.
fn shadowed(name: &str, found: &[PathBuf]) -> Vec<String> {
    let Some(first) = found.first() else {
        return Vec::new();
    };
    found[1..]
        .iter()
        .map(|other| {
            format!(
                "another {name} is on PATH: {} (the first one, {}, wins); remove the extra copy, \
                 for example from `extraPackages`, so only the toolchain's {name} is left",
                other.display(),
                first.display()
            )
        })
        .collect()
}

/// The Node.js a package manager executable runs on, as far as its own file
/// says so without running it.
#[derive(Debug, PartialEq)]
enum ManagerNode {
    /// `#!/usr/bin/env node`: whatever node is first on PATH.
    FromPath,
    /// A script or wrapper that names this node (a nixpkgs or Homebrew
    /// wrapper, or a patched shebang).
    Fixed(PathBuf),
    /// A native executable (bun, pnpm's standalone build) that brings its own
    /// runtime, if any.
    Native,
    /// Anything else; doctor does not guess.
    Unknown,
}

/// Where a package manager gets its Node.js. Only what the files state
/// counts: a shebang, an absolute `.../bin/node` in a shell wrapper, or
/// makeWrapper's `.<name>-wrapped` target, followed a few levels. Larger
/// scripts are never scanned for paths, since a bundled pnpm.cjs or yarn.js
/// mentions node paths in strings that mean nothing about how it is run, and
/// a wrong warning here would send people chasing a working setup.
fn manager_node(exe: &Path) -> ManagerNode {
    // Wrappers are a few hundred bytes; anything bigger is the program.
    const WRAPPER_LIMIT: u64 = 64 * 1024;
    let mut exe = exe.to_path_buf();
    for _ in 0..4 {
        let Ok(bytes) = fs::read(&exe) else {
            return ManagerNode::Unknown;
        };
        if is_native(&bytes) {
            return ManagerNode::Native;
        }
        let Some(rest) = bytes.strip_prefix(b"#!") else {
            return ManagerNode::Unknown;
        };
        let text = String::from_utf8_lossy(rest);
        let shebang: Vec<&str> = text
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let interpreter = shebang.first().copied().unwrap_or_default();
        if interpreter.ends_with("/env") {
            // `env node` or `env -S node --flags`.
            return if shebang[1..].contains(&"node") {
                ManagerNode::FromPath
            } else {
                ManagerNode::Unknown
            };
        }
        if interpreter.ends_with("/bin/node") {
            return ManagerNode::Fixed(PathBuf::from(interpreter));
        }
        let shell = ["sh", "bash", "dash", "zsh"]
            .iter()
            .any(|shell| interpreter.ends_with(&format!("/{shell}")));
        if !shell || bytes.len() as u64 > WRAPPER_LIMIT {
            return ManagerNode::Unknown;
        }
        let paths: Vec<&str> = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || "/._+-@".contains(c)))
            .filter(|word| word.starts_with('/'))
            .collect();
        let mut nodes: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| path.ends_with("/bin/node"))
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        match nodes[..] {
            [node] => return ManagerNode::Fixed(PathBuf::from(node)),
            [] => {}
            _ => return ManagerNode::Unknown,
        }
        let wrapped = exe
            .file_name()
            .map(|name| format!("/.{}-wrapped", name.to_string_lossy()));
        match paths
            .iter()
            .find(|path| wrapped.as_deref().is_some_and(|w| path.ends_with(w)))
        {
            Some(next) => exe = PathBuf::from(next),
            None => return ManagerNode::Unknown,
        }
    }
    ManagerNode::Unknown
}

/// ELF or Mach-O (thin, fat, either byte order).
fn is_native(bytes: &[u8]) -> bool {
    const MAGIC: [[u8; 4]; 6] = [
        *b"\x7fELF",
        [0xfe, 0xed, 0xfa, 0xce],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
    ];
    bytes.len() >= 4 && MAGIC.iter().any(|magic| bytes[..4] == magic[..])
}

/// A package manager wrapped around another Node.js runs install scripts and
/// `pnpm exec` on that one, so native addons and engines checks see a
/// different release than the shell's node.
fn check_manager_node(checks: &mut Checks, name: &str, manager: &Path, node: &Path) {
    match manager_node(manager) {
        ManagerNode::FromPath => checks.ok(format!(
            "{} runs on the node on PATH ({})",
            manager.display(),
            node.display()
        )),
        ManagerNode::Fixed(its) => {
            // The toolchain's node is a symlink into the store, as is the one
            // a crucible wrapper names, so compare the files they resolve to.
            match (fs::canonicalize(&its), fs::canonicalize(node)) {
                (Ok(a), Ok(b)) if a == b => checks.ok(format!(
                    "{} runs on the node on PATH ({})",
                    manager.display(),
                    node.display()
                )),
                (Ok(_), Ok(_)) => checks.warn(format!(
                    "{} runs on {}, not the node on PATH ({}); use crucible's \
                     `packageManager = \"{name}@<major>\"` in `lib.<system>.node.toolchain`, \
                     which builds {name} on the toolchain's Node.js, instead of a separate {name}",
                    manager.display(),
                    its.display(),
                    node.display()
                )),
                _ => checks.info(format!(
                    "{} names {}, which does not exist; cannot tell which Node.js it runs on",
                    manager.display(),
                    its.display()
                )),
            }
        }
        ManagerNode::Native => checks.info(format!(
            "{} is a native executable with its own runtime; the node on PATH does not apply",
            manager.display()
        )),
        ManagerNode::Unknown => checks.info(format!(
            "cannot tell which Node.js {} runs on; not compared with the node on PATH",
            manager.display()
        )),
    }
}

/// The nearest file with one of `names`, searched from `dir` upwards.
fn find_nearest_file(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    dir.ancestors().find_map(|dir| {
        names
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    })
}

/// The first meaningful line of a version-manager file: comments and blank
/// lines are common in `.nvmrc`.
fn first_version_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
}

/// `pnpm@12.21.0+sha512.<digest>` -> ("pnpm", "12.21.0"). The digest is a
/// corepack integrity suffix, not part of the version. A range (`pnpm@^10.2`,
/// devEngines' `>=9 <11`) is kept whole for version_matches.
fn parse_package_manager(spec: &str) -> Option<(String, String)> {
    let (name, version) = spec.rsplit_once('@')?;
    let version = version.split('+').next().unwrap_or(version).trim();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Whether `actual` is a version `wanted` allows. `wanted` is an exact
/// release, a partial pin (`22`, `22.14`, matching that prefix) or an npm
/// range (`^10.2`, `~1.22`, `>=9 <11`, `1.2 - 2`, `8 || 9`), the subset
/// lib/node/semver.nix reads. None when doctor cannot decide: an alias such as
/// `lts/*`, a syntax it does not know, or a pre-release on either side.
fn version_matches(wanted: &str, actual: &str) -> Option<bool> {
    let actual = exact_version(actual)?;
    let mut any = false;
    for alternative in wanted.split("||") {
        let comparators = range_comparators(alternative)?;
        any |= comparators
            .iter()
            .all(|(op, bound)| op.holds(actual.cmp(bound)));
    }
    Some(any)
}

/// Whether `actual` is `minimum` or newer: go.mod's directives and
/// build.zig.zon's `.minimum_zig_version` name the oldest release that will
/// do. None when doctor cannot decide, as for version_matches.
fn at_least(minimum: &str, actual: &str) -> Option<bool> {
    if minimum.contains(char::is_whitespace) {
        return None;
    }
    version_matches(&format!(">={minimum}"), actual)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
}

impl Op {
    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            Op::Ge => ordering != Less,
            Op::Gt => ordering == Greater,
            Op::Le => ordering != Greater,
            Op::Lt => ordering == Less,
        }
    }
}

type Version = [u64; 3];

/// `v22.14.0` -> [22, 14, 0]; anything but a plain release is None.
fn exact_version(text: &str) -> Option<Version> {
    match partial_version(text.trim())? {
        [Some(major), Some(minor), Some(patch)] => Some([major, minor, patch]),
        _ => None,
    }
}

/// `1.2.x` -> [1, 2, None]: a missing or wildcard part is None. Pre-releases
/// are rejected rather than half-supported.
fn partial_version(text: &str) -> Option<[Option<u64>; 3]> {
    let text = text.trim_start_matches(['v', '=']);
    let text = text.split('+').next().unwrap_or(text);
    if text.contains('-') {
        return None;
    }
    let mut parts = [None; 3];
    if text.is_empty() {
        return Some(parts);
    }
    let words: Vec<&str> = text.split('.').collect();
    if words.len() > 3 {
        return None;
    }
    for (slot, word) in parts.iter_mut().zip(words) {
        *slot = match word {
            "x" | "X" | "*" => None,
            word if !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()) => {
                Some(word.parse().ok()?)
            }
            _ => return None,
        };
    }
    // `1.x.3` is not a version.
    if parts.windows(2).any(|w| w[0].is_none() && w[1].is_some()) {
        return None;
    }
    Some(parts)
}

/// One `||` alternative as bounds that must all hold, as `bounds` in
/// lib/node/semver.nix builds them.
fn range_comparators(text: &str) -> Option<Vec<(Op, Version)>> {
    // `>= 9` is `>=9`: glue a lone operator to the version after it.
    let mut words: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match words.last_mut() {
            Some(last) if last.chars().all(|c| "<>=~^".contains(c)) => last.push_str(word),
            _ => words.push(word.to_string()),
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words.get(i + 1).is_some_and(|w| w == "-") {
            let low = partial_version(&words[i])?;
            let high = words.get(i + 2)?;
            out.push((Op::Ge, fill(low)));
            out.extend(at_most(partial_version(high)?));
            i += 3;
            continue;
        }
        let word = &words[i];
        let split = word
            .find(|c: char| !"<>=~^".contains(c))
            .unwrap_or(word.len());
        out.extend(comparator(&word[..split], &word[split..])?);
        i += 1;
    }
    Some(out)
}

fn fill(parts: [Option<u64>; 3]) -> Version {
    parts.map(|part| part.unwrap_or(0))
}

/// `<=1.2` means below 1.3.0, `<=1.2.3` up to and including it.
fn at_most(parts: [Option<u64>; 3]) -> Vec<(Op, Version)> {
    match parts {
        [None, ..] => Vec::new(),
        [Some(major), None, _] => vec![(Op::Lt, [major + 1, 0, 0])],
        [Some(major), Some(minor), None] => vec![(Op::Lt, [major, minor + 1, 0])],
        parts => vec![(Op::Le, fill(parts))],
    }
}

fn comparator(op: &str, version: &str) -> Option<Vec<(Op, Version)>> {
    let parts = partial_version(version)?;
    let low = fill(parts);
    let never = vec![(Op::Lt, [0, 0, 0])];
    Some(match (op, parts) {
        ("" | "=", [None, ..]) => Vec::new(),
        ("" | "=", parts) => {
            let mut bounds = vec![(Op::Ge, low)];
            bounds.extend(at_most(parts));
            bounds
        }
        (">=", _) => vec![(Op::Ge, low)],
        (">", [None, ..]) => never,
        (">", [Some(major), None, _]) => vec![(Op::Ge, [major + 1, 0, 0])],
        (">", [Some(major), Some(minor), None]) => vec![(Op::Ge, [major, minor + 1, 0])],
        (">", _) => vec![(Op::Gt, low)],
        ("<", [None, ..]) => never,
        ("<", _) => vec![(Op::Lt, low)],
        ("<=", parts) => at_most(parts),
        ("~", [None, ..]) => Vec::new(),
        ("~", [Some(major), None, _]) => vec![(Op::Ge, low), (Op::Lt, [major + 1, 0, 0])],
        ("~", [Some(major), Some(minor), _]) => {
            vec![(Op::Ge, low), (Op::Lt, [major, minor + 1, 0])]
        }
        // The first non-zero part may not change, and a missing part is a
        // wildcard: ^0.0 is <0.1.0, ^0 is <1.0.0.
        ("^", [None, ..]) => Vec::new(),
        ("^", [Some(0), Some(0), Some(patch)]) => vec![(Op::Ge, low), (Op::Lt, [0, 0, patch + 1])],
        ("^", [Some(0), Some(minor), _]) if minor > 0 || parts[2].is_none() => {
            vec![(Op::Ge, low), (Op::Lt, [0, minor + 1, 0])]
        }
        ("^", [Some(major), ..]) => vec![(Op::Ge, low), (Op::Lt, [major + 1, 0, 0])],
        _ => return None,
    })
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
    env::var_os("PATH").map_or_else(Vec::new, |path| executables_in(name, &path))
}

/// `executables` for a given PATH value.
fn executables_in(name: &str, path: &std::ffi::OsStr) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for dir in env::split_paths(path) {
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

/// Fixtures the tests of every language share.
#[cfg(test)]
mod test_support {
    use super::*;

    /// A fresh directory for one test, so parallel tests do not collide.
    pub fn scratch(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("crucible-doctor-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn write_executable(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// An Env with only `dirs` on PATH and only `vars` set.
    pub fn fake_env(dirs: &[&Path], vars: &[(&str, &str)]) -> Env {
        Env {
            path: env::join_paths(dirs).unwrap(),
            vars: Some(
                vars.iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
            ),
        }
    }

    /// Run one language's check and return what it printed.
    pub fn run_check(check: impl FnOnce(&mut Checks)) -> (Vec<String>, u32) {
        let mut checks = Checks::default();
        check(&mut checks);
        (checks.lines, checks.errors)
    }

    /// Whether one of `lines` contains `text`, with the lines in the panic
    /// message when it does not.
    #[track_caller]
    pub fn assert_line(lines: &[String], text: &str) {
        assert!(
            lines.iter().any(|line| line.contains(text)),
            "no line contains {text:?}:\n{}",
            lines.join("\n")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn finds_the_nearest_toolchain_file_upwards() {
        let root = std::env::temp_dir().join(format!("crucible-doctor-{}", std::process::id()));
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
    fn reads_the_first_real_line_of_a_version_file() {
        assert_eq!(
            first_version_line("# the line this project runs\n\n  v22.14.0  \n22\n"),
            Some("v22.14.0".to_string())
        );
        assert_eq!(first_version_line("# only a comment\n"), None);
    }

    #[test]
    fn parses_corepack_package_manager_specs() {
        assert_eq!(
            parse_package_manager("pnpm@12.21.0+sha512.abcdef"),
            Some(("pnpm".to_string(), "12.21.0".to_string()))
        );
        assert_eq!(
            parse_package_manager("yarn@4.18.1"),
            Some(("yarn".to_string(), "4.18.1".to_string()))
        );
        // Ranges stay whole, for version_matches.
        assert_eq!(
            parse_package_manager("pnpm@^12.21.0"),
            Some(("pnpm".to_string(), "^12.21.0".to_string()))
        );
        assert_eq!(
            parse_package_manager("pnpm@>=9 <11"),
            Some(("pnpm".to_string(), ">=9 <11".to_string()))
        );
        assert_eq!(parse_package_manager("pnpm"), None);
        assert_eq!(parse_package_manager("pnpm@"), None);
        assert_eq!(
            parse_package_manager("@scope@1.2.3"),
            Some(("@scope".to_string(), "1.2.3".to_string()))
        );
        assert_eq!(major_of("^10.2"), Some("10"));
        assert_eq!(major_of(">=9 <11"), Some("9"));
        assert_eq!(major_of("latest"), None);
    }

    #[test]
    fn falls_back_to_dev_engines_package_manager() {
        let spec = |text: &str| package_manager_spec(&serde_json::from_str(text).unwrap());
        assert_eq!(
            spec(
                r#"{"packageManager": "pnpm@10.4.1", "devEngines": {"packageManager": {"name": "yarn", "version": "4"}}}"#
            ),
            Some(("packageManager", "pnpm@10.4.1".to_string()))
        );
        assert_eq!(
            spec(r#"{"devEngines": {"packageManager": {"name": "pnpm", "version": "^10.2"}}}"#),
            Some(("devEngines.packageManager", "pnpm@^10.2".to_string()))
        );
        // The first entry with both a name and a version, as fromPackageJson.
        assert_eq!(
            spec(
                r#"{"devEngines": {"packageManager": [{"name": "bun"}, {"name": "pnpm", "version": ">=9 <11", "onFail": "warn"}, {"name": "yarn", "version": "4"}]}}"#
            ),
            Some(("devEngines.packageManager", "pnpm@>=9 <11".to_string()))
        );
        assert_eq!(
            spec(r#"{"devEngines": {"runtime": {"name": "node", "version": "24"}}}"#),
            None
        );
        assert_eq!(spec(r#"{"devEngines": {"packageManager": []}}"#), None);
        assert_eq!(spec("{}"), None);
    }

    #[test]
    fn compares_partial_and_exact_node_pins() {
        assert_eq!(version_matches("22", "v22.23.5"), Some(true));
        assert_eq!(version_matches("22.14", "v22.14.0"), Some(true));
        assert_eq!(version_matches("22.14", "v22.23.5"), Some(false));
        assert_eq!(version_matches("22.14.0", "v22.14.1"), Some(false));
        assert_eq!(version_matches("22.14.0", "22.14.0"), Some(true));
        assert_eq!(version_matches("v22", "v22.23.5"), Some(true));
        assert_eq!(version_matches("lts/*", "v22.23.5"), None);
        assert_eq!(version_matches("node", "v22.23.5"), None);
        assert_eq!(version_matches("22", ""), None);
    }

    #[test]
    fn compares_ranges() {
        assert_eq!(version_matches("^22", "v22.23.5"), Some(true));
        assert_eq!(version_matches("^10.2", "10.4.1"), Some(true));
        assert_eq!(version_matches("^10.2", "10.1.9"), Some(false));
        assert_eq!(version_matches("^10.2", "11.0.0"), Some(false));
        assert_eq!(version_matches("^0.2.3", "0.2.9"), Some(true));
        assert_eq!(version_matches("^0.2.3", "0.3.0"), Some(false));
        assert_eq!(version_matches("^0.0.3", "0.0.4"), Some(false));
        assert_eq!(version_matches("~1.22", "1.22.22"), Some(true));
        assert_eq!(version_matches("~1.22.1", "1.23.0"), Some(false));
        assert_eq!(version_matches(">=9 <11", "10.4.1"), Some(true));
        assert_eq!(version_matches(">=9 <11", "11.0.0"), Some(false));
        assert_eq!(version_matches(">= 9 < 11", "9.0.0"), Some(true));
        assert_eq!(version_matches(">9", "9.9.9"), Some(false));
        assert_eq!(version_matches(">9.1", "9.2.0"), Some(true));
        assert_eq!(version_matches("<=10.2", "10.2.7"), Some(true));
        assert_eq!(version_matches("<=10.2", "10.3.0"), Some(false));
        assert_eq!(version_matches("1.2 - 2", "2.9.0"), Some(true));
        assert_eq!(version_matches("1.2 - 2", "3.0.0"), Some(false));
        assert_eq!(version_matches("1.2 - 2", "1.1.0"), Some(false));
        assert_eq!(version_matches("8 || 10.x", "10.4.1"), Some(true));
        assert_eq!(version_matches("8 || 9", "10.4.1"), Some(false));
        assert_eq!(version_matches("*", "10.4.1"), Some(true));
        // What doctor cannot decide is None, not a mismatch.
        assert_eq!(version_matches("latest", "10.4.1"), None);
        assert_eq!(version_matches("^10.0.0-rc.1", "10.4.1"), None);
        assert_eq!(version_matches("^10", "10.0.0-rc.1"), None);
        assert_eq!(version_matches("~>10", "10.4.1"), None);
        assert_eq!(version_matches("1.x.3", "1.0.3"), None);
    }

    #[test]
    #[cfg(unix)]
    fn finds_shadowed_package_managers() {
        let root = scratch("shadowed");
        let (a, b, link, empty) = (
            root.join("a"),
            root.join("b"),
            root.join("link"),
            root.join("empty"),
        );
        write_executable(&a.join("pnpm"), b"#!/bin/sh\necho 10.4.1\n");
        write_executable(&b.join("pnpm"), b"#!/bin/sh\necho 9.15.0\n");
        // A profile symlink to the first copy is the same pnpm.
        fs::create_dir_all(&link).unwrap();
        std::os::unix::fs::symlink(a.join("pnpm"), link.join("pnpm")).unwrap();
        // Not executable, so not a pnpm.
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("pnpm"), "").unwrap();

        let path = env::join_paths([&link, &empty, &a, &b]).unwrap();
        let found = executables_in("pnpm", &path);
        assert_eq!(found, [link.join("pnpm"), b.join("pnpm")]);
        let warnings = shadowed("pnpm", &found);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&b.join("pnpm").display().to_string()));
        assert!(warnings[0].contains(&format!(
            "the first one, {}, wins",
            link.join("pnpm").display()
        )));
        assert!(warnings[0].contains("extraPackages"));

        let path = env::join_paths([&link, &a]).unwrap();
        assert!(shadowed("pnpm", &executables_in("pnpm", &path)).is_empty());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_which_node_a_package_manager_runs_on() {
        let root = scratch("manager-node");
        let store_node = "/nix/store/00000000000000000000000000000000-nodejs-22.14.0/bin/node";

        let env_node = root.join("env/pnpm");
        write_executable(&env_node, b"#!/usr/bin/env node\nrequire('./pnpm.cjs')\n");
        assert_eq!(manager_node(&env_node), ManagerNode::FromPath);

        let shebang = root.join("shebang/yarn");
        write_executable(
            &shebang,
            format!("#!{store_node}\nconsole.log('/usr/bin/node')\n").as_bytes(),
        );
        assert_eq!(
            manager_node(&shebang),
            ManagerNode::Fixed(store_node.into())
        );

        // makeWrapper's script around nixpkgs' pnpm.
        let wrapper = root.join("wrapper/pnpm");
        write_executable(
            &wrapper,
            format!(
                "#! /nix/store/x-bash-5.2/bin/bash -e\nexec \"{store_node}\" --add-flags /nix/store/y-pnpm/libexec/pnpm.cjs \"$@\"\n"
            )
            .as_bytes(),
        );
        assert_eq!(
            manager_node(&wrapper),
            ManagerNode::Fixed(store_node.into())
        );

        // wrapProgram around a native binary: PATH gets a node, but the
        // program that runs is the native one.
        let native = root.join("native/bin");
        write_executable(&native.join(".pnpm-wrapped"), b"\x7fELF\x02\x01\x01");
        write_executable(
            &native.join("pnpm"),
            format!(
                "#!/bin/bash -e\nPATH=/nix/store/z-nodejs-22/bin${{PATH:+:}}$PATH\nexec -a \"$0\" \"{}\" \"$@\"\n",
                native.join(".pnpm-wrapped").display()
            )
            .as_bytes(),
        );
        assert_eq!(manager_node(&native.join("pnpm")), ManagerNode::Native);

        let macho = root.join("macho/bun");
        write_executable(&macho, &[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00]);
        assert_eq!(manager_node(&macho), ManagerNode::Native);

        // Two different nodes, or a big bundle, are not guessed at.
        let two = root.join("two/pnpm");
        write_executable(
            &two,
            b"#!/bin/sh\n[ -x /a/bin/node ] && exec /a/bin/node \"$@\"\nexec /b/bin/node \"$@\"\n",
        );
        assert_eq!(manager_node(&two), ManagerNode::Unknown);
        let python = root.join("python/pnpm");
        write_executable(&python, b"#!/usr/bin/python3\nprint('/usr/bin/node')\n");
        assert_eq!(manager_node(&python), ManagerNode::Unknown);
        let big = root.join("big/pnpm");
        let mut text = format!("#!/bin/sh\nexec {store_node} \"$@\"\n").into_bytes();
        text.resize(70 * 1024, b'#');
        write_executable(&big, &text);
        assert_eq!(manager_node(&big), ManagerNode::Unknown);

        fs::remove_dir_all(&root).unwrap();
    }
}
