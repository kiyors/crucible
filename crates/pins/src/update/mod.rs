//! `crucible update`: rewrite the pinned JSON in `data/`.
//!
//! Every pin is fetched and checked before the first file is touched, so a
//! failed update leaves the previous pins intact instead of a mixed set. Each
//! file is then replaced atomically, in a fixed order: a half-written pin is
//! worse than a stale one because the flake would fail to evaluate.

// The CLI uses `run` and `Scope` only. The language modules stay inside the
// crate: their helpers are implementation detail, not pins' public API.
pub(crate) mod cargo;
pub(crate) mod deno;
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
    /// CPython and uv
    Python,
    /// The supported Go lines
    Go,
    /// Zig releases
    Zig,
    /// tsrs, the TypeScript type checker
    Tsrs,
}

/// The accepted `--what` values, in help order. The CLI uses these for clap's
/// value parser so its help and invalid-value errors stay the same now that
/// `pins` no longer depends on clap.
pub const SCOPES: [&str; 9] = [
    "all", "cargo", "rust", "node", "deno", "python", "go", "zig", "tsrs",
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
        "python" => Ok(Scope::Python),
        "go" => Ok(Scope::Go),
        "zig" => Ok(Scope::Zig),
        "tsrs" => Ok(Scope::Tsrs),
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
    // Go, Zig and tsrs, so a fetch failure anywhere aborts before any pin changes.
    let plan = match scope {
        Scope::All => {
            let mut plan = cargo::plan(&http, version, root)?;
            plan.extend(rust::plan(&http, root)?);
            plan.extend(node_plan(&http, root)?);
            plan.extend(deno::plan(&http, root)?);
            plan.extend(python::plan(&http, root)?);
            plan.extend(go::plan(&http, root)?);
            plan.extend(zig::plan(&http, root)?);
            plan.extend(tsrs::plan(&http, root)?);
            plan
        }
        Scope::Cargo => cargo::plan(&http, version, root)?,
        Scope::Rust => rust::plan(&http, root)?,
        Scope::Node => node_plan(&http, root)?,
        Scope::Deno => deno::plan(&http, root)?,
        Scope::Python => python::plan(&http, root)?,
        Scope::Go => go::plan(&http, root)?,
        Scope::Zig => zig::plan(&http, root)?,
        Scope::Tsrs => tsrs::plan(&http, root)?,
    };

    for (path, bytes) in plan {
        write_atomic(&path, &bytes)?;
    }
    Ok(())
}

fn node_plan(http: &crate::http::Http, root: &Path) -> Result<Plan> {
    let mut plan = node::plan(http, root)?;
    plan.extend(vite_plus::plan(http, root)?);
    plan.extend(package_managers::plan(http, root)?);
    Ok(plan)
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
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let hex = fields.next()?;
            let file = fields.next()?;
            let file = file.strip_prefix('*').unwrap_or(file);
            let is_hex = hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if !is_hex {
                return None;
            }
            Some((hex.to_string(), file.to_string()))
        })
        .collect()
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
        let malformed = || {
            anyhow::anyhow!(
                "malformed checksum row on line {number}: {line:?}; expected 64 lowercase hex \
                 digits, two spaces or `*`, then a filename"
            )
        };
        // coreutils writes `<hex>  <file>` for text and `<hex> *<file>` for
        // binary; anything else is a malformed row, not a name with spaces.
        let Some((hex, rest)) = line.split_once(' ') else {
            return Err(malformed());
        };
        let file = rest
            .strip_prefix(' ')
            .or_else(|| rest.strip_prefix('*'))
            .filter(|file| !file.is_empty() && !file.chars().any(char::is_whitespace))
            .ok_or_else(malformed)?;
        let is_hex = hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !is_hex {
            return Err(malformed());
        }
        sums.push((hex.to_string(), file.to_string()));
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
