//! `crucible doctor`: explain why a machine is not using the fork the way
//! this flake intends.
//!
//! Every check mirrors something the README warns about: other `cargo` binaries
//! earlier or later on PATH, rustup's proxies, settings the fork ignores,
//! caches that end up storing everything twice, a shell older than the last
//! Home Manager switch, and a C compiler that cannot build some crates. Every
//! language has its own module, and every check reads the same replaceable
//! `Env`: the process's in `run`, a fake one in the tests.

mod deno;
mod elm;
mod go;
mod node;
mod python;
mod rust;
mod zig;

use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;

pub fn run() -> Result<ExitCode> {
    let cwd = env::current_dir()?;
    let env = Env::process();
    Ok(exit_code(&run_all(&cwd, &env)))
}

/// Every check, against one cwd and environment. `run` supplies the process's
/// own; the tests a scratch directory with fake executables.
fn run_all(cwd: &Path, env: &Env) -> Checks {
    let mut checks = Checks::default();
    rust::check(&mut checks, cwd, env);
    node::check(&mut checks, cwd, env);
    // Each of these is silent when neither its executable nor its project
    // files are here, so a Rust-only project sees no Python or Go lines.
    python::check(&mut checks, cwd, env);
    go::check(&mut checks, cwd, env);
    zig::check(&mut checks, cwd, env);
    deno::check(&mut checks, cwd, env);
    elm::check(&mut checks, cwd, env);
    checks
}

/// Only errors fail the command; warnings and hints do not.
fn exit_code(checks: &Checks) -> ExitCode {
    if checks.errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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

    /// A variable's value; an empty one counts as unset.
    fn var(&self, name: &str) -> Option<String> {
        match &self.vars {
            None => env_value(name),
            Some(vars) => vars.get(name).filter(|value| !value.is_empty()).cloned(),
        }
    }

    /// An OS-level variable, for values such as HOME that name a path.
    fn var_os(&self, name: &str) -> Option<OsString> {
        match &self.vars {
            None => env::var_os(name),
            Some(vars) => vars.get(name).map(OsString::from),
        }
    }

    fn executables(&self, name: &str) -> Vec<PathBuf> {
        executables_in(name, &self.path)
    }

    /// A command resolved through this PATH, so a test's fake executable wins
    /// without changing the real environment.
    fn command(&self, name: &str) -> Command {
        let mut command = Command::new(name);
        command.env("PATH", &self.path);
        command
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
    match partial_version(text)? {
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

    /// Run every check the way `run` does, but against a scratch cwd and a
    /// fake Env: the printed lines, the error count and the exit code `run`
    /// would choose.
    pub fn run_doctor(cwd: &Path, env: &Env) -> (Vec<String>, u32, ExitCode) {
        let checks = run_all(cwd, env);
        let code = exit_code(&checks);
        (checks.lines, checks.errors, code)
    }

    /// Compare exit codes without assuming they implement `PartialEq`.
    #[track_caller]
    pub fn assert_exit(actual: ExitCode, expected: ExitCode) {
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
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
    fn reads_the_first_real_line_of_a_version_file() {
        assert_eq!(
            first_version_line("# the line this project runs\n\n  v22.14.0  \n22\n"),
            Some("v22.14.0".to_string())
        );
        assert_eq!(first_version_line("# only a comment\n"), None);
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

    /// The npm semver subset has two deliberate copies (Part D tradeoff):
    /// this one and lib/node/semver.nix. They share one corpus so a new case
    /// has to be decided the same way on both sides. `expected: null` means
    /// reject, which is None here and a `throw` in Nix.
    #[test]
    fn compares_ranges() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/corpora/semver.json")).unwrap();
        let cases = corpus["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let range = case["range"].as_str().unwrap();
            let version = case["version"].as_str().unwrap();
            let expected = case["expected"].as_bool();
            assert_eq!(
                version_matches(range, version),
                expected,
                "shared semver corpus: {range} / {version}"
            );
        }
    }

    /// Where the two copies deliberately differ, keep the Rust side's answer
    /// here rather than in the shared corpus: Nix reads `~>` and `1.x.3` as
    /// ranges and ignores a range's pre-release tag, while doctor refuses all
    /// three. These are not corpus cases because Nix disagrees on them.
    #[test]
    fn rejects_the_semver_subset_nix_extends() {
        assert_eq!(version_matches("~>10", "10.4.1"), None);
        assert_eq!(version_matches("1.x.3", "1.0.3"), None);
        assert_eq!(version_matches("^10.0.0-rc.1", "10.4.1"), None);
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
    fn fake_env_only_sees_its_own_path_and_variables() {
        let root = scratch("fake-env");
        let env = fake_env(&[&root], &[("HOME", "/fake/home"), ("EMPTY", "")]);
        assert_eq!(env.var("HOME").as_deref(), Some("/fake/home"));
        assert_eq!(env.var("EMPTY"), None);
        assert_eq!(env.var_os("HOME"), Some(OsString::from("/fake/home")));
        assert!(env.executables("nothing-here").is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A fork cargo and a rustc that both report 1.99.0, so the run-level
    /// tests can layer project files over a clean Rust toolchain.
    #[cfg(unix)]
    fn rust_project(root: &Path) -> (PathBuf, PathBuf) {
        let (bin, project) = (root.join("bin"), root.join("project"));
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
        (bin, project)
    }

    #[test]
    #[cfg(unix)]
    fn doctor_is_clean_for_a_matching_project() {
        let root = scratch("doctor-clean");
        let (bin, project) = rust_project(&root);
        fs::write(
            project.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.99\"\n",
        )
        .unwrap();
        let env = fake_env(&[&bin], &[]);

        let (lines, errors, code) = run_doctor(&project, &env);
        assert_line(&lines, "is huacnlee/cargo");
        assert_line(&lines, "rustc is rustc 1.99.0");
        assert_line(&lines, "rustup is not on PATH");
        assert_line(&lines, "pins 1.99, which matches rustc 1.99.0");
        assert!(
            !lines.iter().any(|line| line.starts_with("error")),
            "{lines:?}"
        );
        assert_eq!(errors, 0);
        assert_exit(code, ExitCode::SUCCESS);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn doctor_reports_an_incompatible_rust_toolchain() {
        let root = scratch("doctor-rust-mismatch");
        let (bin, project) = rust_project(&root);
        fs::write(
            project.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.98\"\n",
        )
        .unwrap();
        let env = fake_env(&[&bin], &[]);

        let (lines, errors, code) = run_doctor(&project, &env);
        assert_line(&lines, "pins 1.98, but rustc is 1.99.0");
        assert_eq!(errors, 1);
        assert_exit(code, ExitCode::FAILURE);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn doctor_reports_an_incompatible_node_pin() {
        let root = scratch("doctor-node-mismatch");
        let (bin, project) = rust_project(&root);
        write_executable(&bin.join("node"), b"#!/bin/sh\necho v22.14.0\n");
        fs::write(project.join(".nvmrc"), "24\n").unwrap();
        let env = fake_env(&[&bin], &[]);

        let (lines, errors, code) = run_doctor(&project, &env);
        assert_line(&lines, "is node v22.14.0");
        assert_line(&lines, ".nvmrc pins 24, but node is v22.14.0");
        assert_eq!(errors, 1);
        assert_exit(code, ExitCode::FAILURE);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn doctor_stays_quiet_without_project_files() {
        let root = scratch("doctor-quiet");
        let (bin, project) = rust_project(&root);
        let env = fake_env(&[&bin], &[]);

        let (lines, errors, code) = run_doctor(&project, &env);
        // Rust and Node still describe the tools on PATH; the five
        // project-driven languages have neither executable nor file to read.
        assert_line(&lines, "is huacnlee/cargo");
        for needle in [
            "pins ",
            "packageManager",
            "no python3 is on PATH",
            "requires-python",
            "no go is on PATH",
            "no zig is on PATH",
            "deno.json is here, but no deno is on PATH",
            "elm.json is here, but no elm is on PATH",
        ] {
            assert!(
                !lines.iter().any(|line| line.contains(needle)),
                "unexpected {needle:?} line:\n{}",
                lines.join("\n")
            );
        }
        assert_eq!(errors, 0);
        assert_exit(code, ExitCode::SUCCESS);

        fs::remove_dir_all(&root).unwrap();
    }
}
