//! Stability of what `crucible pin` prints.
//!
//! The Nix side reads these snippets by hand (AGENTS.md's `shasumsUrl`,
//! `releaseUrl` and `downloadUrl` pairs), so a changed label or URL shape is a
//! user-visible break even though the printed bytes are never parsed. Every
//! entry point that fetches is gated on the network, so the output templates
//! are checked against the source and the pure URL pieces are checked through
//! their public functions; the only entry points called here are the ones that
//! reject their input *before* constructing an `Http`. This keeps the test
//! offline while still exercising the real public API.

const PIN: &str = include_str!("../src/pin.rs");
const NODE: &str = include_str!("../src/update/node.rs");
const PYTHON: &str = include_str!("../src/update/python.rs");
const GO: &str = include_str!("../src/update/go.rs");
const SOURCE: &str = include_str!("../src/source.rs");

#[track_caller]
fn has(source: &str, file: &str, snippet: &str) {
    assert!(
        source.contains(snippet),
        "crucible: {file} lost the printed snippet {snippet:?}; edit {file}"
    );
}

#[test]
fn rust_manifest_input_and_labels() {
    // `pin rust` prints the same URL `manifestUrl` builds in Nix.
    assert_eq!(
        init::channel::manifest_url("stable").unwrap(),
        "https://static.rust-lang.org/dist/channel-rust-stable.toml"
    );
    assert_eq!(
        init::channel::manifest_url("nightly-2026-11-01").unwrap(),
        "https://static.rust-lang.org/dist/2026-11-01/channel-rust-nightly.toml"
    );

    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"rustc: {rustc}\");\n    println!(\"manifest: {url}\");\n    println!(\"sha256: {hash}\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"rust-manifest = {{\");\n    println!(\"  url = \\\"{url}\\\";\");\n    println!(\"  flake = false;\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"toolchain {{\");\n    println!(\"  channel = \\\"{channel}\\\";\");\n    println!(\"  sha256 = \\\"{hash}\\\";\");",
    );
}

#[test]
fn node_shasums_url_shape_and_labels() {
    has(
        NODE,
        "crates/pins/src/update/node.rs",
        "pub const DIST: &str = \"https://nodejs.org/dist\";",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "let url = format!(\"{}/v{version}/SHASUMS256.txt\", crate::update::node::DIST);",
    );
    // The two pieces above are what `shasumsUrl` in lib/node/toolchain.nix
    // spells as `${dist}/v${version}/SHASUMS256.txt`.
    let url = format!(
        "{}/v{}/SHASUMS256.txt",
        "https://nodejs.org/dist", "22.11.0"
    );
    assert_eq!(url, "https://nodejs.org/dist/v22.11.0/SHASUMS256.txt");

    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"node: {version}\");\n    println!(\"shasums: {url}\");\n    println!(\"sha256: {hash}\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"node-shasums = {{\");\n    println!(\"  url = \\\"{url}\\\";\");\n    println!(\"  flake = false;\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"  version = \\\"{version}\\\";\");\n    println!(\"  sha256 = \\\"{hash}\\\";\");",
    );
}

#[test]
fn python_release_url_shape_and_labels() {
    has(
        PYTHON,
        "crates/pins/src/update/python.rs",
        "pub(crate) const PBS: &str = \"astral-sh/python-build-standalone\";",
    );
    has(
        SOURCE,
        "crates/pins/src/source.rs",
        "https://github.com/{repo}/releases/download/{tag}/{}",
    );
    has(
        SOURCE,
        "crates/pins/src/source.rs",
        "file.replace('+', \"%2B\")",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "let url = release_url(PBS, &build, \"SHA256SUMS\");",
    );
    // GitHub needs the `+` in python-build-standalone names percent-encoded;
    // `releaseUrl` in lib/python/toolchain.nix does the same replacement.
    let repo = "astral-sh/python-build-standalone";
    let tag = "20261110";
    let file = "cpython-3.14.10+cpython-3.14.10.tar.gz".replace('+', "%2B");
    assert_eq!(
        format!("https://github.com/{repo}/releases/download/{tag}/{file}"),
        "https://github.com/astral-sh/python-build-standalone/releases/download/20261110/cpython-3.14.10%2Bcpython-3.14.10.tar.gz"
    );

    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"python: {version}\");\n    println!(\"shasums: {url}\");\n    println!(\"sha256: {hash}\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"pbs-shasums = {{\");\n    println!(\"  url = \\\"{url}\\\";\");\n    println!(\"  flake = false;\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"  version = \\\"{version}\\\";\");\n    println!(\"  sha256sums = pbs-shasums;\");",
    );
}

#[test]
fn go_and_elm_hash_blocks() {
    has(
        GO,
        "crates/pins/src/update/go.rs",
        "pub const DL: &str = \"https://dl.google.com/go\";",
    );
    has(
        GO,
        "crates/pins/src/update/go.rs",
        "format!(\"{DL}/{filename}\")",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"go: {version}\");\n    println!();\n    println!(\"toolchain {{\");\n    println!(\"  version = \\\"{version}\\\";\");\n    println!(\"  hashes = {{\");",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"    {system} = \\\"{}\\\";\", source.hash);",
    );
    has(
        PIN,
        "crates/pins/src/pin.rs",
        "    println!(\"elm: {compiler}\");\n    println!();\n    for (system, source) in &platforms {\n        println!(\"{system}: {} {}\", source.url, source.hash);",
    );
}

#[test]
fn public_pin_entry_points_reject_lines_without_network() {
    // Each of these fails in its version parser, before `Http::new()`, so
    // the test never reaches out to a registry or a release server.
    let err = pins::pin::run("nutty").unwrap_err().to_string();
    assert!(
        err.contains("\"nutty\" is not a channel rustup understands"),
        "{err}"
    );

    let err = pins::pin::node("lts").unwrap_err().to_string();
    assert!(
        err.contains("\"lts\" is not an exact Node.js release"),
        "{err}"
    );
    let err = pins::pin::node("22.11.0-rc.1").unwrap_err().to_string();
    assert!(err.contains("is not an exact Node.js release"), "{err}");

    let err = pins::pin::python("3.14.0rc1").unwrap_err().to_string();
    assert!(
        err.contains("\"3.14.0rc1\" is not an exact CPython release"),
        "{err}"
    );

    let err = pins::pin::go("1.27").unwrap_err().to_string();
    assert!(err.contains("\"1.27\" is not an exact Go release"), "{err}");
}

#[test]
fn nixpkgs_only_languages_get_a_policy_error_not_a_bare_clap_message() {
    // `pin` and `update` call the same helper, so these checks cover both.
    for (name, reason) in [
        ("swift", "Apple Developer-ID .pkg"),
        ("elixir", "mix.exs cannot name OTP"),
        ("hashi", "damon has no release assets"),
    ] {
        let err = pins::pin::unsupported(name).unwrap_err().to_string();
        assert!(err.starts_with("crucible: "), "{err}");
        assert!(
            err.contains(&format!("{name} is not pinned by crucible")),
            "{err}"
        );
        assert!(err.contains("nixpkgs-only toolchain"), "{err}");
        assert!(err.contains(reason), "{err}");
        assert!(err.contains("docs/template-policy.md"), "{err}");
    }
}
