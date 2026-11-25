//! Enforce the "two copies must stay in sync" rules in AGENTS.md.
//!
//! Nix evaluation stays offline and the main flake has one input, so the two
//! sides cannot share generated constants. These tests read the sources as
//! text instead. Keep every parser narrow — one named declaration or one set
//! of quoted literals — and name the file to edit in every failure message:
//! text checks are fragile (Part D, RISK 5), so a drift must be easy to fix.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The CLI package is built from a restricted source set that contains the
/// crates but not the Nix/doc files these checks mirror. This is a maintainer
/// check run from the repo root (AGENTS.md), so skip it rather than fail a
/// package build that cannot see its inputs.
fn repo_available() -> bool {
    repo().join("flake.nix").is_file()
}

fn text(rel: &str) -> String {
    let path = repo().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "crucible: cannot read {} ({e}); restore or edit {rel}",
            path.display()
        )
    })
}

fn data(rel: &str) -> Value {
    let raw = text(rel);
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!("crucible: cannot parse {rel}: {e}; `crucible update` writes {rel}")
    })
}

/// The part of `source` from `from` through the first `to` after it. All the
/// declarations parsed here are one contiguous block, so a first-close search
/// cannot wander into an unrelated declaration.
fn fragment<'a>(source: &'a str, file: &str, from: &str, to: &str) -> &'a str {
    let start = source
        .find(from)
        .unwrap_or_else(|| panic!("crucible: `{from}` is missing from {file}; edit {file}"));
    let tail = &source[start..];
    let end = tail.find(to).unwrap_or_else(|| {
        panic!("crucible: `{to}` is missing after `{from}` in {file}; edit {file}")
    });
    &tail[..end + to.len()]
}

/// Quoted literals in a hand-picked fragment. These fragments hold only URLs,
/// system names and archive names — no escapes or interpolated quotes.
fn strings(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else {
            break;
        };
        out.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
    out
}

fn const_string(source: &str, file: &str, marker: &str) -> String {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("crucible: `{marker}` is missing from {file}; edit {file}"));
    let value = strings(&source[start..])
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("crucible: `{marker}` in {file} has no string; edit {file}"));
    value
}

/// A Nix attrset of `key = "value";` lines, in source order.
fn nix_map(source: &str, file: &str, marker: &str) -> Vec<(String, String)> {
    let block = fragment(source, file, marker, "};");
    block
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(" = ")?;
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return None;
            }
            Some((key.to_string(), strings(value).first()?.clone()))
        })
        .collect()
}

/// A Rust const array of `("a", "b")` pairs, in source order.
fn rust_pairs(source: &str, file: &str, marker: &str) -> Vec<(String, String)> {
    let block = fragment(source, file, marker, "];");
    strings(block)
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

fn assert_systems(actual: &[String], systems: &[String], file: &str) {
    assert_eq!(
        actual, systems,
        "crucible: the system list in {file} must match lib/common/systems.nix (same systems, \
         same order); edit {file} or lib/common/systems.nix"
    );
}

/// The Rust host triple of a Nix system. A new system needs a new case here
/// and a new row in every table checked below.
fn host(system: &str) -> &'static str {
    match system {
        "x86_64-linux" => "x86_64-unknown-linux-gnu",
        "aarch64-linux" => "aarch64-unknown-linux-gnu",
        "aarch64-darwin" => "aarch64-apple-darwin",
        other => panic!(
            "crucible: no Rust host triple for {other}; edit \\
                        lib/common/systems.nix and crates/upstream/src/manifest.rs"
        ),
    }
}

fn field_values(value: &Value, key: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (name, child) in map {
                if name == key {
                    if let Some(value) = child.as_str() {
                        out.push(value.to_string());
                    }
                }
                field_values(child, key, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                field_values(child, key, out);
            }
        }
        _ => {}
    }
}

fn versions(value: &Value) -> BTreeSet<String> {
    let mut found = Vec::new();
    field_values(value, "version", &mut found);
    found.into_iter().collect()
}

/// The text between `label` and `end` on the first line that has both.
fn between(source: &str, file: &str, label: &str, end: &str) -> String {
    let start = source
        .find(label)
        .unwrap_or_else(|| panic!("crucible: `{label}` is missing from {file}; edit {file}"));
    let tail = &source[start + label.len()..];
    let stop = tail.find(end).unwrap_or_else(|| {
        panic!("crucible: `{end}` is missing after `{label}` in {file}; edit {file}")
    });
    tail[..stop].to_string()
}

fn version_after(source: &str, file: &str, label: &str) -> String {
    let start = source
        .find(label)
        .unwrap_or_else(|| panic!("crucible: `{label}` is missing from {file}; edit {file}"));
    let tail = &source[start + label.len()..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(tail.len());
    let version = tail[..end].trim_end_matches('.');
    assert!(
        !version.is_empty(),
        "crucible: no version after `{label}` in {file}; edit {file}"
    );
    version.to_string()
}

#[test]
fn systems_nix_matches_hosts_systems_and_dev_flake() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let systems = strings(&text("lib/common/systems.nix"));
    assert!(
        !systems.is_empty(),
        "crucible: lib/common/systems.nix lists no systems; edit lib/common/systems.nix"
    );

    let init = text("crates/init/src/lib.rs");
    let init_systems = strings(fragment(
        &init,
        "crates/init/src/lib.rs",
        "pub const SYSTEMS",
        "];",
    ));
    assert_systems(&init_systems, &systems, "crates/init/src/lib.rs");

    let manifest = text("crates/upstream/src/manifest.rs");
    let hosts = strings(fragment(
        &manifest,
        "crates/upstream/src/manifest.rs",
        "pub const HOSTS",
        "];",
    ));
    let expected: Vec<String> = systems.iter().map(|s| host(s).to_string()).collect();
    assert_eq!(
        hosts, expected,
        "crucible: HOSTS must mirror the systems of lib/common/systems.nix in order; edit \\
         crates/upstream/src/manifest.rs or lib/common/systems.nix"
    );

    let dev = text("dev/flake.nix");
    let dev_systems = strings(fragment(&dev, "dev/flake.nix", "systems = [", "];"));
    assert_systems(&dev_systems, &systems, "dev/flake.nix");
}

#[test]
fn rust_manifest_url_matches_nix_manifest_url() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let channel = text("crates/init/src/channel.rs");
    let dist = const_string(
        &channel,
        "crates/init/src/channel.rs",
        "const DIST: &str = ",
    );
    let nix = text("lib/rust/toolchain.nix");
    assert!(
        nix.contains(&dist),
        "crucible: lib/rust/toolchain.nix's manifestUrl must use the same base as \\
         manifest_url's DIST; edit lib/rust/toolchain.nix or crates/init/src/channel.rs"
    );
    for piece in ["channel-rust-", ".toml", "${date}", "/${date}"] {
        assert!(
            nix.contains(piece),
            "crucible: manifestUrl in lib/rust/toolchain.nix lost `{piece}`; edit \\
             lib/rust/toolchain.nix"
        );
    }
    for piece in ["channel-rust-", ".toml", "{date}", "/{date}"] {
        assert!(
            channel.contains(piece),
            "crucible: manifest_url in crates/init/src/channel.rs lost `{piece}`; edit \\
             crates/init/src/channel.rs"
        );
    }
}

#[test]
fn node_platforms_and_shasums_url_match_nix() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let systems = strings(&text("lib/common/systems.nix"));
    let nix = text("lib/node/toolchain.nix");
    let rust = text("crates/pins/src/update/node.rs");

    let nix_platforms = nix_map(&nix, "lib/node/toolchain.nix", "platforms = {");
    let rust_platforms = rust_pairs(
        &rust,
        "crates/pins/src/update/node.rs",
        "PLATFORMS: [(&str, &str); 3] = [",
    );
    let systems_in_rust: Vec<String> = rust_platforms.iter().map(|p| p.0.clone()).collect();
    assert_systems(&systems_in_rust, &systems, "crates/pins/src/update/node.rs");
    assert_eq!(
        nix_platforms, rust_platforms,
        "crucible: Node.js platforms disagree; edit lib/node/toolchain.nix or \\
         crates/pins/src/update/node.rs"
    );

    let dist = const_string(
        &rust,
        "crates/pins/src/update/node.rs",
        "pub const DIST: &str = ",
    );
    let url = const_string(&nix, "lib/node/toolchain.nix", "shasumsUrl = version: ");
    assert_eq!(
        url,
        format!("{dist}/v${{version}}/SHASUMS256.txt"),
        "crucible: shasumsUrl and the URL `pin node` prints disagree; edit \\
         lib/node/toolchain.nix or crates/pins/src/update/node.rs"
    );
    let pin = text("crates/pins/src/pin.rs");
    assert!(
        pin.contains("v{version}/SHASUMS256.txt") && pin.contains("crate::update::node::DIST"),
        "crucible: `pin node` must print shasumsUrl from lib/node/toolchain.nix; edit \\
         crates/pins/src/pin.rs"
    );
}

#[test]
fn python_platforms_release_url_and_freethreaded_names_match_nix() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let systems = strings(&text("lib/common/systems.nix"));
    let nix = text("lib/python/toolchain.nix");
    let rust = text("crates/pins/src/update/python.rs");

    let nix_platforms = nix_map(&nix, "lib/python/toolchain.nix", "platforms = {");
    let rust_platforms = rust_pairs(
        &rust,
        "crates/pins/src/update/python.rs",
        "PLATFORMS: [(&str, &str); 3] = [",
    );
    let systems_in_rust: Vec<String> = rust_platforms.iter().map(|p| p.0.clone()).collect();
    assert_systems(
        &systems_in_rust,
        &systems,
        "crates/pins/src/update/python.rs",
    );
    assert_eq!(
        nix_platforms, rust_platforms,
        "crucible: python-build-standalone platforms disagree; edit \\
         lib/python/toolchain.nix or crates/pins/src/update/python.rs"
    );

    assert!(
        nix.contains(
            "https://github.com/astral-sh/python-build-standalone/releases/download/${build}/"
        ) && nix.contains("lib.replaceStrings [ \"+\" ] [ \"%2B\" ]"),
        "crucible: releaseUrl must match release_url's base and %2B encoding; edit \\
         lib/python/toolchain.nix"
    );
    assert!(
        rust.contains("https://github.com/{repo}/releases/download/{tag}/{}")
            && rust.contains("file.replace('+', \"%2B\")"),
        "crucible: release_url must match releaseUrl in lib/python/toolchain.nix; edit \\
         crates/pins/src/update/python.rs"
    );

    // The Nix side names the interpreter python3.14t; the pins name archives
    // with a "-freethreaded-" infix and lines with a trailing "t".
    let interpreter = text("lib/python/python.nix");
    assert!(
        interpreter.contains("python3.14t")
            && interpreter.contains("lib.optionalString (release.freethreaded or false) \"t\""),
        "crucible: the freethreaded interpreter name changed; edit lib/python/python.nix"
    );
    assert!(
        nix.contains("\"-freethreaded\"")
            && rust.contains("\"freethreaded-\"")
            && rust.contains("ends_with('t')"),
        "crucible: the freethreaded archive infix and line suffix disagree; edit \\
         lib/python/toolchain.nix, lib/python/python.nix or \\
         crates/pins/src/update/python.rs"
    );
    let pinned = data("data/python/python.json");
    let releases = pinned["releases"].as_object().unwrap_or_else(|| {
        panic!("crucible: data/python/python.json has no releases; run `crucible update python`")
    });
    for (line, release) in releases {
        if !line.ends_with('t') {
            continue;
        }
        assert_eq!(
            release["freethreaded"], true,
            "crucible: data/python/python.json line {line} must be marked freethreaded; run `crucible update python`"
        );
        for platform in release["platforms"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, platform)| platform)
        {
            let url = platform["url"].as_str().unwrap_or_default();
            assert!(
                url.contains("-freethreaded-"),
                "crucible: data/python/python.json line {line} has an archive without the \\
                 -freethreaded- infix; run `crucible update python`"
            );
        }
    }
}

#[test]
fn go_platforms_and_download_url_match_nix() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let systems = strings(&text("lib/common/systems.nix"));
    let nix = text("lib/go/toolchain.nix");
    let rust = text("crates/pins/src/update/go.rs");

    let nix_platforms = nix_map(&nix, "lib/go/toolchain.nix", "platforms = {");
    let rust_platforms = rust_pairs(
        &rust,
        "crates/pins/src/update/go.rs",
        "PLATFORMS: [(&str, &str); 3] = [",
    );
    let systems_in_rust: Vec<String> = rust_platforms.iter().map(|p| p.0.clone()).collect();
    assert_systems(&systems_in_rust, &systems, "crates/pins/src/update/go.rs");
    assert_eq!(
        nix_platforms, rust_platforms,
        "crucible: Go platforms disagree; edit lib/go/toolchain.nix or \\
         crates/pins/src/update/go.rs"
    );

    let dl = const_string(
        &rust,
        "crates/pins/src/update/go.rs",
        "pub const DL: &str = ",
    );
    assert!(
        nix.contains(&format!("{dl}/go${{version}}.${{platform}}.tar.gz"))
            && rust.contains("format!(\"{DL}/{filename}\")")
            && rust.contains("format!(\"{}.{platform}.tar.gz\", release.version)"),
        "crucible: downloadUrl and `DL` must build the same Go archive URLs; edit \\
         lib/go/toolchain.nix or crates/pins/src/update/go.rs"
    );
}

#[test]
fn package_manager_layout_matches_nix() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let nix = text("lib/node/package-manager.nix");
    assert!(
        nix.contains("layout = pin.layout or \"tarball\";"),
        "crucible: package-manager.nix must default a pin without `layout` to tarball; \\
         edit lib/node/package-manager.nix"
    );

    let pins = data("data/node/package-managers.json");
    let from_data: BTreeSet<String> = pins["pnpm"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, line)| line.get("layout").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    assert!(
        !from_data.is_empty(),
        "crucible: data/node/package-managers.json pins no pnpm layout; run `crucible update node`"
    );

    let rust = text("crates/pins/src/update/package_managers.rs");
    let block = fragment(
        &rust,
        "crates/pins/src/update/package_managers.rs",
        "const PNPM: [PnpmLine; 3] = [",
        "];",
    );
    let from_rust: BTreeSet<String> = block
        .lines()
        .filter_map(|line| {
            let value = line.trim().strip_prefix("layout: ")?;
            strings(value).first().cloned()
        })
        .collect();
    assert_eq!(
        from_data, from_rust,
        "crucible: pnpm layouts in data/node/package-managers.json must match the ones \\
         crates/pins/src/update/package_managers.rs verifies; run `crucible update node` or \\
         edit crates/pins/src/update/package_managers.rs"
    );
    // The Nix fallback must itself be a real layout, not a spelling drift.
    assert!(
        from_rust.contains("tarball"),
        "crucible: package-manager.nix's `tarball` fallback is not a layout in \\
         crates/pins/src/update/package_managers.rs; edit both files"
    );
}

#[test]
fn npm_keys_endpoint_matches_pinned_keys() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let npm = text("crates/upstream/src/npm.rs");
    let endpoint = "https://registry.npmjs.org/-/npm/v1/keys";
    assert!(
        npm.contains(endpoint),
        "crucible: crates/upstream/src/npm.rs must document the keys endpoint it pins; \\
         edit crates/upstream/src/npm.rs"
    );
    assert!(
        npm.contains("const KEYS: [Key;")
            && npm.contains("update KEYS in")
            && npm.contains("crates/upstream/src/npm.rs"),
        "crucible: npm key rotation must point at the pinned KEYS array in \\
         crates/upstream/src/npm.rs; edit crates/upstream/src/npm.rs"
    );
    let marker = "const KEYS: [Key; ";
    let start = npm.find(marker).unwrap_or_else(|| {
        panic!("crucible: no pinned KEYS in crates/upstream/src/npm.rs; edit that file")
    }) + marker.len();
    let declared: String = npm[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let block = fragment(
        &npm,
        "crates/upstream/src/npm.rs",
        "const KEYS: [Key;",
        "];",
    );
    let entries = block.matches("\n    Key {").count();
    assert_eq!(
        declared,
        entries.to_string(),
        "crucible: KEYS in crates/upstream/src/npm.rs declares {declared} keys but has {entries}; \\
         edit crates/upstream/src/npm.rs"
    );
}

#[test]
fn pnpm_attestation_matches_the_assets_nix_fetches() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let attestation = text("crates/upstream/src/attestation.rs");
    let file = "crates/upstream/src/attestation.rs";
    let api = const_string(&attestation, file, "const API: &str = ");
    let repository = const_string(&attestation, file, "const REPOSITORY: &str = ");
    let release = const_string(&attestation, file, "const RELEASE: &str = ");
    assert_eq!(
        api,
        format!("https://api.github.com/repos/{repository}/attestations/sha256:"),
        "crucible: the attestation API must query REPOSITORY; edit {file}"
    );
    assert_eq!(
        release, "https://in-toto.io/attestation/release/v0.2",
        "crucible: the attestation predicate type changed; edit {file} and its test fixture"
    );
    assert!(
        attestation.contains(&format!("\"predicateType\": \"{release}\"")),
        "crucible: the attestation test fixture must use the pinned predicate type; edit {file}"
    );

    let pins = data("data/node/package-managers.json");
    let rust = text("crates/pins/src/update/package_managers.rs");
    let block = fragment(
        &rust,
        "crates/pins/src/update/package_managers.rs",
        "const PNPM: [PnpmLine; 3] = [",
        "];",
    );
    let mut assets = Vec::new();
    let mut rest = block;
    while let Some(index) = rest.find("platforms: [") {
        let tail = &rest[index..];
        let inner = fragment(
            tail,
            "crates/pins/src/update/package_managers.rs",
            "platforms: [",
            "]",
        );
        assets.extend(strings(inner).into_iter().skip(1).step_by(2));
        rest = &tail[inner.len()..];
    }
    assert!(
        rust.contains("attestation::verify(http, hex, asset, tag)?"),
        "crucible: every pinned pnpm asset must be attestation-checked; edit \\
         crates/pins/src/update/package_managers.rs"
    );

    let prefix = format!("https://github.com/{repository}/releases/download/v");
    for (major, line) in pins["pnpm"].as_object().into_iter().flatten() {
        for platform in line["platforms"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, platform)| platform)
        {
            let url = platform["url"].as_str().unwrap_or_default();
            assert!(
                url.starts_with(&prefix),
                "crucible: pnpm {major} must fetch assets from {repository}, the repo the \\
                 attestation checks; run `crucible update node` or edit {file}"
            );
            let asset = url.rsplit('/').next().unwrap_or_default();
            assert!(
                assets.iter().any(|a| a == asset),
                "crucible: pnpm {major} asset {asset} is fetched by Nix but not attestation-checked; \\
                 edit data/node/package-managers.json or \\
                 crates/pins/src/update/package_managers.rs"
            );
        }
    }
}

#[test]
fn all_platform_tables_match_systems_nix() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let systems = strings(&text("lib/common/systems.nix"));
    // These copies are not in AGENTS.md's original eight pairs, but a new
    // system has to be added to all of them for pins to be complete.
    let tables = [
        (
            "crates/pins/src/update/cargo.rs",
            "PLATFORMS: [(&str, &str); 3] = [",
        ),
        (
            "crates/pins/src/update/vite_plus.rs",
            "PLATFORMS: [(&str, &str); 3] = [",
        ),
        (
            "crates/pins/src/update/deno.rs",
            "DENO_ASSETS: [(&str, &str); 3] = [",
        ),
        (
            "crates/pins/src/update/zig.rs",
            "PLATFORMS: [(&str, &str); 3] = [",
        ),
        (
            "crates/pins/src/update/tsrs.rs",
            "PLATFORMS: [(&str, &str); 3] = [",
        ),
        (
            "crates/pins/src/update/elm.rs",
            "PLATFORMS: [(&str, &str); 3] = [",
        ),
    ];
    for (file, marker) in tables {
        let source = text(file);
        let pairs = rust_pairs(&source, file, marker);
        let listed: Vec<String> = pairs.iter().map(|p| p.0.clone()).collect();
        assert_systems(&listed, &systems, file);
    }

    // pnpm's assets are struct literals, not tuples: pick the `system` field.
    let file = "crates/upstream/src/pnpm_lock.rs";
    let source = text(file);
    let block = fragment(
        &source,
        file,
        "pub const PLATFORMS: [Platform; 3] = [",
        "];",
    );
    let mut listed = Vec::new();
    let mut rest = block;
    while let Some(index) = rest.find("system: \"") {
        rest = &rest[index + "system: ".len()..];
        let value = strings(rest)
            .first()
            .unwrap_or_else(|| {
                panic!("crucible: no system name after `system:` in {file}; edit {file}")
            })
            .clone();
        listed.push(value.clone());
        rest = &rest[value.len() + 2..];
    }
    assert_systems(&listed, &systems, file);

    // The same for pnpm's per-line asset tables in the updater.
    let file = "crates/pins/src/update/package_managers.rs";
    let source = text(file);
    let block = fragment(&source, file, "const PNPM: [PnpmLine; 3] = [", "];");
    let mut listed = Vec::new();
    let mut rest = block;
    while let Some(index) = rest.find("platforms: [") {
        let tail = &rest[index..];
        let inner = fragment(tail, file, "platforms: [", "]");
        listed.extend(strings(inner).into_iter().step_by(2));
        rest = &tail[inner.len()..];
    }
    // Three pnpm lines, each with one row per system: check every line's
    // table against systems.nix instead of concatenating them.
    assert_eq!(
        listed.len() % systems.len(),
        0,
        "crucible: pnpm's asset tables in {file} do not have one row per system; edit {file}"
    );
    for table in listed.chunks(systems.len()) {
        assert_systems(table, &systems, file);
    }
}

#[test]
fn elm_pin_shape_matches_the_data() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let file = "crates/pins/src/update/elm.rs";
    let source = text(file);
    let registry = const_string(&source, file, "const REGISTRY: &str = ");
    let suffix = const_string(&source, file, "const NPM_SUFFIX: &str = ");
    assert_eq!(
        registry, "https://registry.npmjs.org",
        "crucible: the npm registry base changed; edit {file} or lib/elm/toolchain.nix"
    );
    assert_eq!(
        suffix, "-0",
        "crucible: npm's Elm package suffix changed; edit {file}, lib/elm/toolchain.nix and data/elm/elm.json"
    );

    let systems = strings(&text("lib/common/systems.nix"));
    let pairs = rust_pairs(&source, file, "PLATFORMS: [(&str, &str); 3] = [");
    assert_systems(
        &pairs.iter().map(|p| p.0.clone()).collect::<Vec<_>>(),
        &systems,
        file,
    );

    // Elm's compiler version is the data key; npm keeps a -0 suffix in the
    // tarball URL. Nix must mirror both, so pin them here rather than in prose.
    let pins = data("data/elm/elm.json");
    for (version, release) in pins.as_object().into_iter().flatten() {
        assert_eq!(
            release["version"].as_str(),
            Some(version.as_str()),
            "crucible: data/elm/elm.json's inner version must be the compiler version; run `crucible update elm`"
        );
        for (system, platform) in release["platforms"].as_object().into_iter().flatten() {
            assert!(
                systems.contains(system),
                "crucible: Elm pin {version} uses system {system}, which lib/common/systems.nix does not; run `crucible update elm`"
            );
            let package = pairs
                .iter()
                .find(|p| p.0 == *system)
                .map(|p| p.1.as_str())
                .unwrap_or_default();
            let url = platform["url"].as_str().unwrap_or_default();
            let expected = format!("{registry}/{package}/-/");
            let npm_version = format!("{version}{suffix}");
            assert!(
                url.starts_with(&expected) && url.ends_with(&format!("-{npm_version}.tgz")),
                "crucible: Elm pin {version} on {system} has URL {url}, which does not use {package} and the -0 suffix; edit {file} or run `crucible update elm`"
            );
            assert!(
                platform["hash"]
                    .as_str()
                    .is_some_and(|h| h.starts_with("sha512-")),
                "crucible: Elm pin {version} on {system} must carry an SRI hash; run `crucible update elm`"
            );
        }
    }
}

#[test]
fn template_name_lists_match() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let flake = text("flake.nix");
    assert!(
        flake.contains("templates = import ./templates.nix;"),
        "crucible: flake.nix must expose templates.nix; edit flake.nix"
    );

    let templates = text("templates.nix");
    let mut attrs = BTreeSet::new();
    for line in templates.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let Some((name, _)) = rest.split_once(" = {") else {
            continue;
        };
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
            attrs.insert(name.to_string());
        }
    }
    assert!(
        !attrs.is_empty(),
        "crucible: templates.nix declares no templates; edit templates.nix"
    );

    // The Nix check derives the list from templates.nix and the directories
    // themselves, so there is no hardcoded list to drift here either. Read
    // the same two sources and assert the check still wires them up.
    let init = text("tests/init.nix");
    assert!(
        init.contains("declared = import ../templates.nix;")
            && init.contains("builtins.readDir ../templates")
            && init.contains("evaluate attrs"),
        "crucible: tests/init.nix must compare templates.nix attrs with the templates/ \\
         directories and evaluate them; edit tests/init.nix"
    );
    let dir = repo().join("templates");
    let mut expected: BTreeSet<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("crucible: cannot read templates/ ({e})"))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        // `hash` is the flake attr for the `hashi` directory.
        .map(|name| if name == "hashi" { "hash" } else { &name }.to_string())
        .collect();
    // `c` and `cpp` alias `c-cpp` and have no directory of their own.
    expected.insert("c".to_string());
    expected.insert("cpp".to_string());
    assert_eq!(
        attrs, expected,
        "crucible: template attrs and template directories disagree; edit templates.nix, \\
         tests/init.nix and README.md"
    );
    // The README lists canonical template names, not the c/cpp aliases.
    let mut named = attrs.clone();
    named.remove("c");
    named.remove("cpp");

    let readme = text("README.md");
    let mut documented = BTreeSet::new();
    for line in readme.lines() {
        let Some(tail) = line.strip_prefix("nix flake init -t github:kiyors/crucible") else {
            continue;
        };
        if tail.contains("`default` template") {
            documented.insert("default".to_string());
        } else if let Some(name) = tail.trim_start().strip_prefix('#') {
            let name = name.split_whitespace().next().unwrap_or_default();
            documented.insert(name.to_string());
        }
    }
    assert_eq!(
        documented, named,
        "crucible: README.md's template list disagrees with templates.nix; edit README.md or \\
         templates.nix"
    );

    // 4.6 made this flake the single template source. Both the CLI and the
    // README must name it, and the CLI's accepted names must be the flake attrs.
    let cli = text("crates/crucible/src/templates.rs");
    assert!(
        cli.contains("github:kiyors/crucible") && readme.contains("[`templates/`](templates)"),
        "crucible: the CLI and README.md must name this flake's templates; edit \\
         crates/crucible/src/templates.rs or README.md"
    );
    let entries = strings(fragment(
        &cli,
        "crates/crucible/src/templates.rs",
        "const TEMPLATE_DIRS",
        "];",
    ));
    let accepted: BTreeSet<String> = entries.iter().step_by(2).cloned().collect();
    assert_eq!(
        accepted, attrs,
        "crucible: TEMPLATE_DIRS in crates/crucible/src/templates.rs must cover the \\
         templates.nix attrs; edit crates/crucible/src/templates.rs or templates.nix"
    );
}

/// Phase 4.4 was resolved by policy, not silently dropped: the three languages
/// are named in one shared error, and their templates must not pretend to be
/// pinned. This guards both halves so a half-promotion fails a check. It
/// asserts no fake data pins exist for them — there are deliberately none.
#[test]
fn nixpkgs_only_languages_are_explicitly_unsupported() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }

    let pin_file = "crates/pins/src/pin.rs";
    let pin = text(pin_file);
    let table = fragment(&pin, pin_file, "pub(crate) const UNSUPPORTED", "];");
    let update_file = "crates/pins/src/update/mod.rs";
    let update = text(update_file);
    let main_file = "crates/crucible/src/main.rs";
    let main = text(main_file);
    let policy = text("docs/template-policy.md");

    for name in ["swift", "elixir", "hashi"] {
        assert!(
            table.contains(&format!("\"{name}\"")),
            "crucible: pin::UNSUPPORTED lost {name}; edit {pin_file}"
        );
        assert!(
            update.contains(&format!("\"{name}\" => Ok(Scope::Unsupported(\"{name}\"))")),
            "crucible: {name} must be accepted by update so it can be explained; edit {update_file}"
        );
        assert!(
            main.contains(&format!("pin::unsupported(\"{name}\")")),
            "crucible: `pin {name}` must reach the shared helper; edit {main_file}"
        );
        assert!(
            policy.contains(name),
            "crucible: docs/template-policy.md lost {name}; edit docs/template-policy.md"
        );
    }
    for snippet in [
        "is not pinned by crucible",
        "nixpkgs-only toolchain",
        "docs/template-policy.md",
    ] {
        assert!(
            pin.contains(snippet),
            "crucible: the shared unsupported error lost {snippet:?}; edit {pin_file}"
        );
    }
    assert!(
        update.contains("Scope::Unsupported(language) => return crate::pin::unsupported(language)")
            && main.contains("Command::Pin(Pin::Swift { .. }) => pin::unsupported(\"swift\")"),
        "crucible: update and pin must fail on an nixpkgs-only language; edit {update_file} or \\
         {main_file}"
    );

    // (b) A nixpkgs-only template must declare only nixpkgs. An accidental
    // `inputs.crucible` here would be a half-promotion with no pin pipeline.
    for dir in ["swift", "elixir", "hashi"] {
        let file = format!("templates/{dir}/flake.nix");
        let flake = text(&file);
        assert!(
            !flake.contains("inputs.crucible"),
            "crucible: {file} declares a crucible input; nixpkgs-only templates must not, see \\
             docs/template-policy.md"
        );
    }
}

#[test]
fn readme_versions_match_the_data_pins() {
    if !repo_available() {
        eprintln!(
            "crucible: skipping source sync check: repo-root sources are absent from this build"
        );
        return;
    }
    let readme = text("README.md");
    // Read the version out of the prose and look it up in the data. Never the
    // other way around: a scheduled pin update may move the version freely.
    let pnpm = version_after(&readme, "README.md", "`pnpm@");
    let pm = data("data/node/package-managers.json");
    assert!(
        versions(&pm).contains(&pnpm),
        "crucible: README.md documents pnpm {pnpm}, which is not pinned in \\
         data/node/package-managers.json; edit README.md or run `crucible update node`"
    );
    let lines = between(
        &readme,
        "README.md",
        "line upstream still releases (",
        ") is pinned",
    );
    let majors: BTreeSet<&str> = lines.split(", ").flat_map(|s| s.split(" and ")).collect();
    for major in &majors {
        assert!(
            pm["pnpm"].get(*major).is_some(),
            "crucible: README.md lists pnpm line {major}, which data/node/package-managers.json \\
             does not pin; edit README.md or run `crucible update node`"
        );
    }

    let deno_version = version_after(&readme, "README.md", "newest Deno 2, ");
    let deno = data("data/deno/deno.json");
    assert!(
        versions(&deno).contains(&deno_version),
        "crucible: README.md documents Deno {deno_version}, which is not pinned in \\
         data/deno/deno.json; edit README.md or run `crucible update deno`"
    );

    let go = data("data/go/go.json");
    let go_lines = between(&readme, "README.md", "the two newest (", " today");
    let go_releases = go["releases"].as_object().unwrap_or_else(|| {
        panic!("crucible: data/go/go.json has no releases; run `crucible update go`")
    });
    for line in go_lines.split_whitespace().filter(|word| *word != "and") {
        assert!(
            go_releases.contains_key(line),
            "crucible: README.md documents Go line {line}, which data/go/go.json does not pin; \\
             edit README.md or run `crucible update go`"
        );
    }
    let pinned = between(&readme, "README.md", "as new: the pinned ", " release (");
    let label = format!("as new: the pinned {pinned} release (");
    let pinned_version = between(&readme, "README.md", &label, " today");
    assert_eq!(
        go_releases[&pinned]["version"].as_str().unwrap_or_default(),
        pinned_version,
        "crucible: README.md's pinned Go release disagrees with data/go/go.json; edit README.md \\
         or run `crucible update go`"
    );

    let python = data("data/python/python.json");
    let python_lines = between(&readme, "README.md", "still fixes (", " today");
    let free_lines = between(
        &readme,
        "README.md",
        "own next to the regular ones (",
        " today",
    );
    let python_releases = python["releases"].as_object().unwrap_or_else(|| {
        panic!("crucible: data/python/python.json has no releases; run `crucible update python`")
    });
    let endpoints = python_lines
        .split_whitespace()
        .chain(free_lines.split_whitespace())
        .filter(|word| *word != "to");
    for endpoint in endpoints {
        assert!(
            python_releases.contains_key(endpoint),
            "crucible: README.md documents Python line {endpoint}, which \\
             data/python/python.json does not pin; edit README.md or run `crucible update python`"
        );
    }

    let zig_version = version_after(&readme, "README.md", "an exact pinned release (\"");
    let zig = data("data/zig/zig.json");
    assert!(
        versions(&zig).contains(&zig_version),
        "crucible: README.md documents Zig {zig_version}, which is not pinned in \\
         data/zig/zig.json; edit README.md or run `crucible update zig`"
    );

    let elm_version = version_after(&readme, "README.md", "today it is ");
    let elm = data("data/elm/elm.json");
    assert!(
        versions(&elm).contains(&elm_version),
        "crucible: README.md documents Elm {elm_version}, which is not pinned in \\
         data/elm/elm.json; edit README.md or run `crucible update elm`"
    );
}
