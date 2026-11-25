//! `crucible pin`: turn a version the pins do not cover into the two things a
//! Nix user needs to pin it: a `flake = false` input, or that file's SRI hash.
//!
//! For Rust the file is the channel manifest, and the URL rules mirror
//! `manifestUrl` in lib/rust/toolchain.nix, including the dated forms
//! (`nightly-2026-11-01`), so the hashes and the flake agree. For Node it is the
//! release's `SHASUMS256.txt`, as `shasumsUrl` in lib/node/toolchain.nix builds it.

use anyhow::{bail, Context, Result};

use crate::http::Http;
use crate::manifest::Manifest;
use crate::source::{checked_asset, release_url};
use crate::sri;
use crate::update::python::{self, PBS};

// The URL rules live in `channel`, so `init` can use them without `net`.
pub(crate) use init::channel::manifest_url;

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

pub fn node(version: &str) -> Result<()> {
    let version = node_version(version)?;
    let http = Http::new();
    let text = crate::update::node::shasums(&http, version)?;
    // Fails here, rather than at evaluation time, for a release that lacks a
    // tarball for one of crucible's systems.
    crate::update::node::sources(&crate::update::parse_sha256sums(&text), version)?;
    let url = format!("{}/v{version}/SHASUMS256.txt", crate::update::node::DIST);
    let hash = sri::sri(text.as_bytes());

    println!("node: {version}");
    println!("shasums: {url}");
    println!("sha256: {hash}");
    println!();
    println!("node-shasums = {{");
    println!("  url = \"{url}\";");
    println!("  flake = false;");
    println!("}};");
    println!();
    println!("toolchain {{");
    println!("  version = \"{version}\";");
    println!("  sha256 = \"{hash}\";");
    println!("}};");
    Ok(())
}

/// `crucible pin go <version>`: the `hashes` argument for a release that is not
/// pinned. The index is the same one `update go` reads, so the hashes printed
/// here are the ones Nix will use.
pub fn go(version: &str) -> Result<()> {
    use crate::update::go::{exact_version, index, sources};

    let version = exact_version(version)?;
    let releases = index(&Http::new())?;
    let release = releases
        .iter()
        .find(|release| release.version == format!("go{version}"))
        .with_context(|| format!("go.dev lists no Go release {version}"))?;
    let platforms = sources(release)?;

    println!("go: {version}");
    println!();
    println!("toolchain {{");
    println!("  version = \"{version}\";");
    println!("  hashes = {{");
    for (system, source) in &platforms {
        println!("    {system} = \"{}\";", source.hash);
    }
    println!("  }};");
    println!("}};");
    Ok(())
}

/// `crucible pin python <x.y.z>`: the `sha256sums` input for a release that is
/// not pinned, with its hash. The build is the newest python-build-standalone
/// one that carries the release for every system; its `SHA256SUMS` is checked
/// against the digest GitHub records for it, as `update python` checks its
/// own.
pub fn python(version: &str) -> Result<()> {
    let version = python_release_version(version)?;
    let http = Http::new();
    let (release, build) = python::find_release(&http, version)?;
    let sums = checked_asset(&http, PBS, &release, "SHA256SUMS")?;
    // Fails here, rather than at evaluation time, for a release the build's
    // SHA256SUMS lacks an archive of for one of crucible's systems.
    python::sources(
        &crate::update::parse_sha256sums(&sums),
        version,
        &build,
        false,
    )?;
    let url = release_url(PBS, &build, "SHA256SUMS");
    let hash = sri::sri(sums.as_bytes());

    println!("python: {version}");
    println!("shasums: {url}");
    println!("sha256: {hash}");
    println!();
    println!("pbs-shasums = {{");
    println!("  url = \"{url}\";");
    println!("  flake = false;");
    println!("}};");
    println!();
    println!("toolchain {{");
    println!("  version = \"{version}\";");
    println!("  sha256sums = pbs-shasums;");
    println!("}};");
    Ok(())
}

/// An exact CPython release, `x.y.z`: the build's archives name it.
fn python_release_version(version: &str) -> Result<&str> {
    let valid = version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if !valid {
        bail!(
            "\"{version}\" is not an exact CPython release such as 3.14.0; \
             lines (3.14) are pinned in data/python/python.json"
        );
    }
    Ok(version)
}

/// An exact Node.js release, `v` prefix optional: SHASUMS256.txt exists per
/// release only, not per line.
fn node_version(version: &str) -> Result<&str> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let parts: Vec<&str> = version.split('.').collect();
    let exact = parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if !exact {
        anyhow::bail!(
            "\"{version}\" is not an exact Node.js release such as 22.11.0; lines (22, lts) are already pinned"
        );
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_node_releases() {
        assert_eq!(node_version("v22.11.0").unwrap(), "22.11.0");
        assert_eq!(node_version("26.13.1").unwrap(), "26.13.1");
        for version in ["22", "22.11", "lts", "22.11.0-rc.1", "v22..0", ""] {
            assert!(
                node_version(version).is_err(),
                "{version} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_only_exact_python_releases() {
        assert_eq!(python_release_version("3.14.0").unwrap(), "3.14.0");
        assert_eq!(python_release_version("3.15.2").unwrap(), "3.15.2");
        for version in ["3", "3.14", "v3.14.0", "3.14.0rc1", "3.14.0.0", "", "three"] {
            assert!(
                python_release_version(version).is_err(),
                "{version} should be rejected"
            );
        }
    }
}
