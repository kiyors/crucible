//! `crucible update`: rewrite the pinned JSON in `data/`, and
//! `crucible update --check`: re-verify the provenance of what is already there.
//!
//! Every pin is fetched and checked before the first file is touched, so a
//! failed update leaves the previous pins intact instead of a mixed set. All
//! files are then staged before any is renamed, and a failure anywhere rolls
//! the whole set back: a half-written pin is worse than a stale one because the
//! flake would fail to evaluate. `--check` shares the trust decisions but never
//! plans a new version, writes a file or takes the update lock.

// The CLI uses `run` and `Scope` only. The language modules stay inside the
// crate: their helpers are implementation detail, not pins' public API.
pub(crate) mod cargo;
pub(crate) mod deno;
pub(crate) mod elm;
pub(crate) mod go;
pub(crate) mod node;
pub(crate) mod package_managers;
pub(crate) mod python;
pub(crate) mod rust;
pub(crate) mod tsrs;
pub(crate) mod vite_plus;
pub(crate) mod zig;

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

// `--check` needs to re-fetch upstream metadata without touching the network
// in tests. The one operation it needs is already abstracted for attestation,
// so reuse that trait instead of defining a second one here.
pub(crate) use crate::attestation::Fetch;

/// What `update` refreshes; everything by default, as the scheduled job runs it.
#[derive(Clone, Copy, Debug)]
pub enum Scope {
    All,
    Cargo,
    Rust,
    /// Node.js and Vite+
    Node,
    /// Deno
    Deno,
    /// The Elm compiler, from its npm platform packages
    Elm,
    /// CPython and uv
    Python,
    /// The supported Go lines
    Go,
    /// Zig releases
    Zig,
    /// tsrs, the TypeScript type checker
    Tsrs,
    /// A nixpkgs-only language. Accepted so the CLI can explain the choice
    /// instead of reporting an invalid value; both `run` and `check` fail.
    Unsupported(&'static str),
}

/// The accepted `--what` values, in help order. The CLI uses these for clap's
/// value parser so its help and invalid-value errors stay the same now that
/// `pins` no longer depends on clap. The last three are nixpkgs-only: they are
/// listed so `parse_scope` can explain them, never to update pins.
pub const SCOPES: [&str; 13] = [
    "all", "cargo", "rust", "node", "deno", "elm", "python", "go", "zig", "tsrs", "swift",
    "elixir", "hashi",
];

/// Turn a `--what` value into a scope. Clap rejects values outside `SCOPES`
/// before this runs; the error is here for other callers.
pub fn parse_scope(value: &str) -> Result<Scope> {
    match value {
        "all" => Ok(Scope::All),
        "cargo" => Ok(Scope::Cargo),
        "rust" => Ok(Scope::Rust),
        "node" => Ok(Scope::Node),
        "deno" => Ok(Scope::Deno),
        "elm" => Ok(Scope::Elm),
        "python" => Ok(Scope::Python),
        "go" => Ok(Scope::Go),
        "zig" => Ok(Scope::Zig),
        "tsrs" => Ok(Scope::Tsrs),
        "swift" => Ok(Scope::Unsupported("swift")),
        "elixir" => Ok(Scope::Unsupported("elixir")),
        "hashi" => Ok(Scope::Unsupported("hashi")),
        other => bail!(
            "crucible: unknown update scope {other:?}; expected one of {}",
            SCOPES.join(", ")
        ),
    }
}

/// The files a run will write, already serialized, in write order.
type Plan = Vec<(PathBuf, Vec<u8>)>;

/// The lock lives in the repo root the run modifies, next to the pins.
const LOCK: &str = ".crucible-update.lock";

pub fn run(scope: Scope, version: Option<String>, root: &Path) -> Result<()> {
    let _lock = Lock::acquire(root)?;
    let http = crate::http::Http::new();

    // Plan everything first: the cargo pin, then stable, beta and nightly, then
    // Node, Vite+, the package managers and Deno, then Python and uv, then
    // Go, Zig and tsrs, then Elm, so a fetch failure anywhere aborts before any
    // pin changes.
    let plan = match scope {
        // These scopes exist only to produce a useful error; there is no pin
        // pipeline to plan and no lock to take for a language in nixpkgs.
        Scope::Unsupported(language) => return crate::pin::unsupported(language),
        Scope::All => {
            let mut plan = cargo::plan(&http, version, root)?;
            plan.extend(rust::plan(&http, root)?);
            plan.extend(node_plan(&http, root)?);
            plan.extend(deno::plan(&http, root)?);
            plan.extend(python::plan(&http, root)?);
            plan.extend(go::plan(&http, root)?);
            plan.extend(zig::plan(&http, root)?);
            plan.extend(tsrs::plan(&http, root)?);
            plan.extend(elm::plan(&http, root)?);
            plan
        }
        Scope::Cargo => cargo::plan(&http, version, root)?,
        Scope::Rust => rust::plan(&http, root)?,
        Scope::Node => node_plan(&http, root)?,
        Scope::Deno => deno::plan(&http, root)?,
        Scope::Elm => elm::plan(&http, root)?,
        Scope::Python => python::plan(&http, root)?,
        Scope::Go => go::plan(&http, root)?,
        Scope::Zig => zig::plan(&http, root)?,
        Scope::Tsrs => tsrs::plan(&http, root)?,
    };

    write_plan(&plan)
}

fn node_plan(http: &crate::http::Http, root: &Path) -> Result<Plan> {
    let mut plan = node::plan(http, root)?;
    plan.extend(vite_plus::plan(http, root)?);
    plan.extend(package_managers::plan(http, root)?);
    Ok(plan)
}

/// Re-fetch and re-verify the provenance of the pins already in `root`.
///
/// The scheduled workflow runs this on the data an update just produced, so a
/// compromised or buggy updater cannot push hashes that no upstream trust
/// anchor vouches for. It deliberately does **not** recompute "latest": every
/// check reads the pinned version first and asks upstream about that exact
/// release, so a new upstream release does not invalidate an older pin.
///
/// Every family is attempted even when an earlier one failed; the failures are
/// reported together so CI shows the whole picture in one run. Each family
/// takes the injected `Fetch`, so every trust decision — including GPG and the
/// npm/attestation signatures — is exercised offline in tests with canned
/// responses; none needs a live network to be checked.
pub fn check(scope: Scope, root: &Path) -> Result<()> {
    let http = crate::http::Http::new();
    check_with(scope, root, &http)
}

fn check_with<F: Fetch>(scope: Scope, root: &Path, http: &F) -> Result<()> {
    let mut failures = Vec::new();
    match scope {
        Scope::Unsupported(language) => return crate::pin::unsupported(language),
        Scope::All => {
            record("cargo", cargo::verify(http, root), &mut failures);
            record("rust manifests", rust::verify(http, root), &mut failures);
            record("node", node::verify(http, root), &mut failures);
            record("vite-plus", vite_plus::verify(http, root), &mut failures);
            record(
                "package managers",
                package_managers::verify(http, root),
                &mut failures,
            );
            record("deno", deno::verify(http, root), &mut failures);
            record("python and uv", python::verify(http, root), &mut failures);
            record("go", go::verify(http, root), &mut failures);
            record("zig", zig::verify(http, root), &mut failures);
            record("tsrs", tsrs::verify(http, root), &mut failures);
            record("elm", elm::verify(http, root), &mut failures);
        }
        Scope::Cargo => record("cargo", cargo::verify(http, root), &mut failures),
        Scope::Rust => record("rust manifests", rust::verify(http, root), &mut failures),
        Scope::Node => {
            record("node", node::verify(http, root), &mut failures);
            record("vite-plus", vite_plus::verify(http, root), &mut failures);
            record(
                "package managers",
                package_managers::verify(http, root),
                &mut failures,
            );
        }
        Scope::Deno => record("deno", deno::verify(http, root), &mut failures),
        Scope::Elm => record("elm", elm::verify(http, root), &mut failures),
        Scope::Python => record("python and uv", python::verify(http, root), &mut failures),
        Scope::Go => record("go", go::verify(http, root), &mut failures),
        Scope::Zig => record("zig", zig::verify(http, root), &mut failures),
        Scope::Tsrs => record("tsrs", tsrs::verify(http, root), &mut failures),
    }

    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "crucible: {} provenance check(s) failed:\n  - {}",
            failures.len(),
            failures.join("\n  - ")
        )
    }
}

/// Keep going after a failed family and collect its error for the final report.
fn record(name: &str, result: Result<()>, failures: &mut Vec<String>) {
    match result {
        Ok(()) => println!("crucible: {name}: provenance verified"),
        Err(err) => {
            let message = format!("{name}: {err:#}");
            eprintln!("crucible: {message}");
            failures.push(message);
        }
    }
}

/// Parse a `SHA256SUMS`-style file (the fork's `SHA256SUMS`, Node's
/// `SHASUMS256.txt`, Vite+'s `vp-checksums.txt`): `<64 hex>  <file>` or
/// `<64 hex> *<file>`.
///
/// Lenient for the callers whose checksum file has no independent trust
/// anchor (`pin`, Node and bun) and which only ever need the rows they ask
/// for. A file whose hashes *are* anchored to a GitHub asset digest must use
/// [`parse_sha256sums_checked`] instead: a dropped row there would let a
/// malformed line hide in a file we just proved authentic.
pub(crate) fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    let mut sums = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match sum_row(line) {
            Some(row) => sums.push(row),
            // A dropped row is easy to miss when the caller only asks for a
            // few files, so say which line was ignored and why. The strict
            // parser below still rejects the same row for verified files.
            None => eprintln!(
                "crucible: warning: ignoring malformed checksum row on line {}: {line:?}; \
                 expected 64 lowercase hex digits, two spaces or `*`, then a filename",
                index + 1
            ),
        }
    }
    sums
}

/// One valid `<64 lowercase hex>  <file>` (or `*<file>`) row. Shared by both
/// parsers so the lenient one warns about exactly what the strict one rejects.
fn sum_row(line: &str) -> Option<(String, String)> {
    // coreutils writes `<hex>  <file>` for text and `<hex> *<file>` for
    // binary; anything else is a malformed row, not a name with spaces.
    let (hex, rest) = line.split_once(' ')?;
    let file = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'))?;
    if file.is_empty() || file.chars().any(char::is_whitespace) {
        return None;
    }
    let is_hex = hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !is_hex {
        return None;
    }
    Some((hex.to_string(), file.to_string()))
}

/// The strict counterpart of [`parse_sha256sums`]: every row must be a
/// lowercase 64-digit sha256 and exactly one filename, separated by either
/// two spaces or a `*`. Blank lines are the only rows skipped. A bad row
/// fails with its line number, so a verified checksum file cannot hide it.
pub(crate) fn parse_sha256sums_checked(text: &str) -> Result<Vec<(String, String)>> {
    let mut sums = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let number = index + 1;
        let (hex, file) = sum_row(line).with_context(|| {
            format!(
                "malformed checksum row on line {number}: {line:?}; expected 64 lowercase hex \
                 digits, two spaces or `*`, then a filename"
            )
        })?;
        sums.push((hex, file));
    }
    Ok(sums)
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

/// The temporary name a pin is written to before it replaces the real one.
/// Deterministic per process so a failed run can clean up after itself.
fn temp_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("manifest"),
        std::process::id()
    ))
}

/// Write `bytes` to a temporary file next to `path`; the rename is deliberately
/// a separate step so every file can be staged before any is replaced.
fn stage_file(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(dir)?;

    let tmp = temp_path(path);
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("failed to create {}", tmp.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
    }
    Ok(tmp)
}

/// Stage every file, then replace every destination, then roll the whole set
/// back if any step failed. The plan is fixed before this runs, so a failure
/// while staging file 7 of 13 leaves files 1–6 untouched rather than written.
fn write_plan(plan: &Plan) -> Result<()> {
    write_plan_with(plan, stage_file, |tmp: &Path, dest: &Path| {
        fs::rename(tmp, dest)
    })
}

/// The seams (`stage`, `rename`) exist so a unit test can fail a specific file
/// and check that the rollback leaves the original bytes behind. Production
/// only ever passes the real filesystem operations above.
fn write_plan_with<S, R>(plan: &Plan, stage: S, rename: R) -> Result<()>
where
    S: Fn(&Path, &[u8]) -> Result<PathBuf>,
    R: Fn(&Path, &Path) -> std::io::Result<()>,
{
    // (temporary file, destination, bytes to put back on failure)
    let mut staged: Vec<(PathBuf, PathBuf, Option<Vec<u8>>)> = Vec::new();
    let outcome = (|| -> Result<()> {
        for (path, bytes) in plan {
            let previous = match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(err) if err.kind() == ErrorKind::NotFound => None,
                Err(err) => {
                    return Err(err).with_context(|| format!("failed to read {}", path.display()));
                }
            };
            let tmp = stage(path, bytes)?;
            staged.push((tmp, path.clone(), previous));
        }
        for (tmp, path, _) in &staged {
            rename(tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
        }
        Ok(())
    })();

    if outcome.is_err() {
        // Best effort: remove every temp, and put the old bytes back. A file
        // that did not exist before is removed instead. Renamed temps no
        // longer exist, so removing them is a harmless no-op.
        for (tmp, path, previous) in staged {
            let _ = fs::remove_file(&tmp);
            let restored = match previous {
                Some(bytes) => fs::write(&path, bytes),
                None => fs::remove_file(&path),
            };
            if let Err(err) = restored {
                eprintln!(
                    "crucible: warning: could not restore {}: {err}",
                    path.display()
                );
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::http::HttpError;

    fn temp_dir(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "crucible-update-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const HEX: &str = "4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd";
    const ROWS: [(&str, &str); 2] = [
        (HEX, "cargo-0.1.0-x86_64-unknown-linux-gnu.tar.gz"),
        (HEX, "cargo-0.1.0-aarch64-apple-darwin.tar.gz"),
    ];
    const VALID: &str = "\
4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd  cargo-0.1.0-x86_64-unknown-linux-gnu.tar.gz
4c9634493bfb3d6fc926a5cee14271afa2e33008a7793932148e31c4b4d543dd *cargo-0.1.0-aarch64-apple-darwin.tar.gz
";

    #[test]
    fn parses_both_sha256sums_separators() {
        let expected = ROWS.map(|(hex, file)| (hex.to_string(), file.to_string()));
        assert_eq!(parse_sha256sums(VALID), expected);
        assert_eq!(parse_sha256sums_checked(VALID).unwrap(), expected);
    }

    #[test]
    fn strict_parser_names_the_bad_line() {
        let text = format!(
            "{VALID}not a sums line\n4C9634493BFB3D6FC926A5CEE14271AFA2E33008A7793932148E31C4B4D543DD  uppercase.tar.gz\n"
        );
        let err = parse_sha256sums_checked(&text).unwrap_err().to_string();
        assert!(err.contains("line 3"), "{err}");
        assert!(err.contains("not a sums line"), "{err}");
        // The unverified caller still drops the bad rows and keeps the good
        // ones; that is the whole reason the two parsers are separate.
        assert_eq!(parse_sha256sums(&text).len(), 2);

        let uppercase = format!("{VALID}{}  uppercase.tar.gz\n", HEX.to_uppercase());
        let err = parse_sha256sums_checked(&uppercase)
            .unwrap_err()
            .to_string();
        assert!(err.contains("line 3"), "{err}");

        let extra = format!("{HEX}  a.tar.gz  extra\n");
        assert!(parse_sha256sums_checked(&extra).is_err());
        let one_space = format!("{HEX} a.tar.gz\n");
        assert!(parse_sha256sums_checked(&one_space).is_err());
        // Blank lines are not rows.
        assert_eq!(
            parse_sha256sums_checked(&format!("\n{VALID}\n"))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn every_advertised_scope_parses() {
        for value in SCOPES {
            assert!(
                parse_scope(value).is_ok(),
                "crucible: {value} is advertised"
            );
        }
        assert!(parse_scope("nope").is_err());
    }

    /// The nixpkgs-only names parse, but neither path may turn into a no-op:
    /// both `update` and `update --check` must explain the choice and leave
    /// the root untouched.
    #[test]
    fn unsupported_scopes_fail_without_writing_pins() {
        let root = temp_dir("unsupported");
        for name in ["swift", "elixir", "hashi"] {
            let scope = parse_scope(name).unwrap();
            for err in [
                run(scope, None, &root).unwrap_err(),
                check(scope, &root).unwrap_err(),
            ] {
                let message = format!("{err:#}");
                assert!(
                    message.contains(&format!("{name} is not pinned by crucible")),
                    "{message}"
                );
                assert!(message.contains("docs/template-policy.md"), "{message}");
            }
            assert!(!root.join(LOCK).exists());
        }
        assert!(snapshot(&root).is_empty());
        fs::remove_dir_all(&root).unwrap();
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

    /// A failure while writing the 2nd of 3 files must put all three back.
    #[test]
    fn a_failed_rename_rolls_back_every_file() {
        let root = temp_dir("rollback");
        let names = ["a.json", "b.json", "c.json"];
        for name in names {
            fs::write(root.join(name), format!("old-{name}")).unwrap();
        }
        let plan: Plan = names
            .iter()
            .map(|name| (root.join(name), format!("new-{name}").into_bytes()))
            .collect();

        let calls = Cell::new(0);
        let err = write_plan_with(&plan, stage_file, |tmp, dest| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "simulated rename failure",
                ));
            }
            fs::rename(tmp, dest)
        })
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("simulated rename failure"),
            "{err:#}"
        );
        for name in names {
            assert_eq!(
                fs::read_to_string(root.join(name)).unwrap(),
                format!("old-{name}"),
                "{name} was not rolled back"
            );
            assert!(
                !temp_path(&root.join(name)).exists(),
                "a temp file was left behind"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// A failure while staging must not leave a temp or touch any pin either.
    #[test]
    fn a_failed_stage_rolls_back_every_file() {
        let root = temp_dir("rollback-stage");
        let names = ["a.json", "b.json", "c.json"];
        for name in names {
            fs::write(root.join(name), format!("old-{name}")).unwrap();
        }
        let plan: Plan = names
            .iter()
            .map(|name| (root.join(name), format!("new-{name}").into_bytes()))
            .collect();

        let err = write_plan_with(
            &plan,
            |path, bytes| {
                if path.ends_with("b.json") {
                    bail!("simulated stage failure");
                }
                stage_file(path, bytes)
            },
            |tmp: &Path, dest: &Path| fs::rename(tmp, dest),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("simulated stage failure"),
            "{err:#}"
        );
        for name in names {
            assert_eq!(
                fs::read_to_string(root.join(name)).unwrap(),
                format!("old-{name}")
            );
            assert!(!temp_path(&root.join(name)).exists());
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// A fake `Fetch`, so the check path can be exercised without a network.
    type Get = Box<dyn Fn(&str) -> std::result::Result<Vec<u8>, HttpError>>;

    struct Fake {
        get: Get,
    }

    impl Fetch for Fake {
        fn get(&self, url: &str) -> std::result::Result<Vec<u8>, HttpError> {
            (self.get)(url)
        }
    }

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                let relative = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                if entry.file_type().unwrap().is_dir() {
                    walk(&entry.path(), &relative, out);
                } else {
                    out.push((relative, fs::read(entry.path()).unwrap()));
                }
            }
            out.sort_by(|left, right| left.0.cmp(&right.0));
        }
        let mut out = Vec::new();
        walk(root, "", &mut out);
        out
    }

    /// `--check` re-verifies pinned data without writing it or taking the
    /// lock. The fake answers a cargo release and its anchored checksum file.
    #[test]
    fn a_check_run_leaves_the_root_byte_identical() {
        let root = temp_dir("check-no-write");
        let dir = root.join("data/rust");
        fs::create_dir_all(&dir).unwrap();
        let version = "0.103.0-1";
        let triples = [
            ("x86_64-linux", "x86_64-unknown-linux-gnu"),
            ("aarch64-linux", "aarch64-unknown-linux-gnu"),
            ("aarch64-darwin", "aarch64-apple-darwin"),
        ];
        let hash = crate::sri::from_hex(HEX).unwrap();
        let mut body = String::new();
        let mut platforms = String::new();
        for (system, triple) in triples {
            let file = format!("cargo-{version}-{triple}.tar.gz");
            body.push_str(&format!("{HEX}  {file}\n"));
            if !platforms.is_empty() {
                platforms.push(',');
            }
            platforms.push_str(&format!(
                "\"{system}\":{{\"url\":\"https://example.test/{file}\",\"hash\":\"{hash}\"}}"
            ));
        }
        let pin = format!(r#"{{"version":"{version}","platforms":{{{platforms}}}}}"#);
        fs::write(dir.join("cargo.json"), pin).unwrap();

        let fake = Fake {
            get: Box::new(move |url| {
                match url {
                    "https://api.github.com/repos/huacnlee/cargo/releases/tags/v0.103.0-1" => Ok(
                        serde_json::json!({
                            "tag_name": format!("v{version}"),
                            "assets": [{ "name": "SHA256SUMS",
                                         "digest": "sha256:c5a07f7801b1269d1bbf8b004141c8f6736d5874a3e82a6053904f6b7b8d210f" }]
                        })
                        .to_string()
                        .into_bytes(),
                    ),
                    "https://github.com/huacnlee/cargo/releases/download/v0.103.0-1/SHA256SUMS" => {
                        Ok(body.clone().into_bytes())
                    }
                    other => Err(HttpError::NotFound {
                        url: other.to_string(),
                    }),
                }
            }),
        };

        let before = snapshot(&root);
        check_with(Scope::Cargo, &root, &fake).unwrap();
        assert_eq!(snapshot(&root), before, "a check run changed the root");
        assert!(!root.join(LOCK).exists(), "a check run created the lock");
        fs::remove_dir_all(&root).unwrap();
    }

    /// Failures from different families are collected, not returned early.
    #[test]
    fn a_check_reports_every_failed_family() {
        let root = temp_dir("check-failures");
        let fake = Fake {
            get: Box::new(|url| {
                Err(HttpError::NotFound {
                    url: url.to_string(),
                })
            }),
        };
        let err = check_with(Scope::Node, &root, &fake)
            .unwrap_err()
            .to_string();
        assert!(err.contains("3 provenance check(s) failed"), "{err}");
        assert!(err.contains("node:"), "{err}");
        assert!(err.contains("vite-plus:"), "{err}");
        assert!(err.contains("package managers:"), "{err}");
        fs::remove_dir_all(&root).unwrap();
    }
}
