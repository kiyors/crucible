//! Clap dispatch, without running any command.
//!
//! `main.rs` is a binary whose clap types are private, and its module-level
//! doc comments cannot be pulled into a test with `include!`. The two parsers
//! below mirror its `net` and non-`net` branches so `Parser::try_parse_from`
//! can exercise dispatch offline. The source assertions at the bottom pin the
//! mirror to the real declarations and to the exact help lines that
//! `tests/default.nix` greps, so a change to either side fails here.

use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

const SOURCE: &str = include_str!("../src/main.rs");
const PINS_UPDATE: &str = include_str!("../../pins/src/update/mod.rs");
const SCOPES: [&str; 10] = [
    "all", "cargo", "rust", "node", "deno", "elm", "python", "go", "zig", "tsrs",
];

#[track_caller]
fn has(snippet: &str) {
    assert!(
        SOURCE.contains(snippet),
        "crucible: crates/crucible/src/main.rs lost {snippet:?}; edit main.rs and this test"
    );
}

#[track_caller]
fn has_not(snippet: &str) {
    assert!(
        !SOURCE.contains(snippet),
        "crucible: crates/crucible/src/main.rs unexpectedly contains {snippet:?}"
    );
}

/// The `net` build: `update` and `pin` are commands.
#[derive(Debug, Parser)]
#[command(name = "crucible", version, arg_required_else_help = true)]
#[command(about = "Start a toolchain flake, diagnose its environment and refresh crucible's pins")]
struct Full {
    #[command(subcommand)]
    command: FullCommand,
}

#[derive(Debug, Subcommand)]
enum FullCommand {
    /// Rewrite the pins in data/ from upstream
    Update {
        /// What to refresh
        #[arg(
            default_value = "all",
            value_parser = clap::builder::PossibleValuesParser::new(SCOPES)
        )]
        what: String,
        /// Use this huacnlee/cargo version instead of the latest release
        /// (with `cargo` or the default `all` scope)
        #[arg(long)]
        version: Option<String>,
        /// Re-verify the provenance of the already pinned data without
        /// writing it or taking the update lock
        #[arg(long)]
        check: bool,
        /// Repository root holding data/
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Check that the toolchains on PATH work: cargo is the fork, and Node.js,
    /// Python, Go, Zig and Deno match the project
    Doctor,
    /// Write a flake.nix in this directory from its project files; with a
    /// directory, create a new project there from a template instead
    Init {
        /// Replace an existing flake.nix (only when no directory is given)
        #[arg(long)]
        force: bool,
        /// Create a new project in this directory from a template; the
        /// directory must not already exist
        directory: Option<PathBuf>,
        /// Template name from github:kiyors/crucible (default: empty)
        template: Option<String>,
    },
    /// Add a template to the project in the current directory
    Add {
        /// Template name from github:kiyors/crucible (default: empty)
        template: Option<String>,
    },
    /// Print what a project needs to pin a toolchain version
    #[command(subcommand)]
    Pin(FullPin),
}

#[derive(Debug, Subcommand)]
enum FullPin {
    /// The flake input and sha256 for a rustup channel
    Rust {
        /// A rustup channel: stable, 1.99, nightly-2026-11-01, ...
        channel: String,
    },
    /// The flake input and sha256 for a Node.js release's SHASUMS256.txt
    Node {
        /// An exact release: 22.11.0, v24.1.0, ...
        version: String,
    },
    /// The per-system hashes for an Elm compiler release the pins do not cover
    Elm {
        /// A compiler release: 0.19.3 (npm's 0.19.3-0 form also works)
        version: String,
    },
    /// The per-system hashes for a Go release the pins do not cover
    Go {
        /// An exact release: 1.25.3, go1.27rc1, ...
        version: String,
    },
    /// The `sha256sums` input for a Python release the pins do not cover
    Python {
        /// An exact release: 3.14.0
        version: String,
    },
}

/// The user build: no `update`/`pin`; an unknown word is an external command.
#[derive(Debug, Parser)]
#[command(name = "crucible", version, arg_required_else_help = true)]
#[command(about = "Start a toolchain flake and diagnose its environment")]
struct Slim {
    #[command(subcommand)]
    command: SlimCommand,
}

#[derive(Debug, Subcommand)]
enum SlimCommand {
    /// Check that the toolchains on PATH work: cargo is the fork, and Node.js,
    /// Python, Go, Zig and Deno match the project
    Doctor,
    /// Write a flake.nix in this directory from its project files; with a
    /// directory, create a new project there from a template instead
    Init {
        #[arg(long)]
        force: bool,
        directory: Option<PathBuf>,
        template: Option<String>,
    },
    /// Add a template to the project in the current directory
    Add { template: Option<String> },
    #[command(external_subcommand)]
    Missing(Vec<String>),
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn a_bare_command_is_an_error_that_prints_help() {
    let full = Full::try_parse_from(["crucible"]).unwrap_err();
    assert_eq!(
        full.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    let help = full.to_string();
    for command in ["  init ", "  add ", "  doctor ", "  update ", "  pin "] {
        assert!(
            lines(&help).iter().any(|line| line.starts_with(command)),
            "no {command:?} line in the full help:\n{help}"
        );
    }
    assert!(help.contains("Usage: crucible <COMMAND>"), "{help}");

    let slim = Slim::try_parse_from(["crucible"]).unwrap_err();
    assert_eq!(
        slim.kind(),
        clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    let help = slim.to_string();
    for command in ["  init ", "  add ", "  doctor "] {
        assert!(
            lines(&help).iter().any(|line| line.starts_with(command)),
            "no {command:?} line in the user build's help:\n{help}"
        );
    }
    for command in ["  update ", "  pin "] {
        assert!(
            !lines(&help).iter().any(|line| line.starts_with(command)),
            "the user build must not advertise {command:?}:\n{help}"
        );
    }
}

#[test]
fn the_full_build_exposes_update_and_pin_flags() {
    let cli =
        Full::try_parse_from(["crucible", "update", "--check", "--root", "/tmp/repo"]).unwrap();
    match cli.command {
        FullCommand::Update {
            what,
            version,
            check,
            root,
        } => {
            assert_eq!(what, "all");
            assert_eq!(version, None);
            assert!(check);
            assert_eq!(root, PathBuf::from("/tmp/repo"));
        }
        other => panic!("expected update, got {other:?}"),
    }

    let cli = Full::try_parse_from(["crucible", "update", "node", "--version", "24.11.0"]).unwrap();
    match cli.command {
        FullCommand::Update {
            what,
            version,
            check,
            ..
        } => {
            assert_eq!(what, "node");
            assert_eq!(version.as_deref(), Some("24.11.0"));
            assert!(!check);
        }
        other => panic!("expected update, got {other:?}"),
    }

    for (args, expected) in [
        (vec!["crucible", "pin", "rust", "1.99"], "rust"),
        (vec!["crucible", "pin", "node", "22.11.0"], "node"),
        (vec!["crucible", "pin", "python", "3.14.0"], "python"),
        (vec!["crucible", "pin", "go", "1.25.3"], "go"),
        (vec!["crucible", "pin", "elm", "0.19.3"], "elm"),
    ] {
        let cli = Full::try_parse_from(args).unwrap();
        let FullCommand::Pin(pin) = cli.command else {
            panic!("expected pin")
        };
        let name = match pin {
            FullPin::Rust { .. } => "rust",
            FullPin::Node { .. } => "node",
            FullPin::Python { .. } => "python",
            FullPin::Go { .. } => "go",
            FullPin::Elm { .. } => "elm",
        };
        assert_eq!(name, expected);
    }
}

#[test]
fn an_unknown_external_subcommand_is_only_a_user_build_concept() {
    let args = ["crucible", "pin", "rust", "1.99"];
    let slim = Slim::try_parse_from(args).unwrap();
    match slim.command {
        SlimCommand::Missing(words) => assert_eq!(words, ["pin", "rust", "1.99"]),
        other => panic!("expected an external subcommand, got {other:?}"),
    }
    // The full build has a real `pin` command, not an external one.
    assert!(Full::try_parse_from(args).is_ok(), "the full build has pin");
    assert!(Full::try_parse_from(["crucible", "nope"]).is_err());
    let slim = Slim::try_parse_from(["crucible", "nope"]).unwrap();
    match slim.command {
        SlimCommand::Missing(words) => assert_eq!(words, ["nope"]),
        other => panic!("expected an external subcommand, got {other:?}"),
    }
}

#[test]
fn invalid_update_scopes_fail_with_the_possible_values() {
    let err = Full::try_parse_from(["crucible", "update", "nope"])
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid value 'nope'"), "{err}");
    assert!(err.contains("possible values"), "{err}");
    for scope in SCOPES {
        assert!(err.contains(scope), "missing {scope:?} in {err}");
    }
    for scope in SCOPES {
        assert!(
            Full::try_parse_from(["crucible", "update", scope]).is_ok(),
            "{scope}"
        );
    }
}

#[test]
fn the_mirror_still_matches_the_real_cli() {
    has("#[command(name = \"crucible\", version, arg_required_else_help = true)]");
    has("value_parser = clap::builder::PossibleValuesParser::new(pins::update::SCOPES)");
    assert!(
        PINS_UPDATE.contains(
            "SCOPES: [&str; 10] = [\n    \"all\", \"cargo\", \"rust\", \"node\", \"deno\", \"elm\", \"python\", \"go\", \"zig\", \"tsrs\",\n];"
        ),
        "crucible: the mirror's scope list drifted from pins::update::SCOPES"
    );
    has("#[arg(long)]\n        version: Option<String>,");
    has("#[arg(long)]\n        check: bool,");
    has("#[arg(long, default_value = \".\")]\n        root: PathBuf,");
    has("#[cfg(feature = \"net\")]\n    #[command(subcommand)]\n    Pin(Pin),");
    has("#[cfg(not(feature = \"net\"))]\n    #[command(external_subcommand)]\n    Missing(Vec<String>),");
    // The exact help lines the Nix check greps, and the pointer it greps in
    // the user build's error for `pin` and `update`.
    has("/// Rewrite the pins in data/ from upstream");
    has("/// Print what a project needs to pin a toolchain version");
    has("/// Check that the toolchains on PATH work");
    has("/// Write a flake.nix in this directory from its project files");
    has("/// Add a template to the project in the current directory");
    has("github:kiyors/crucible#crucible-full -- {}");
    has("--check verifies the pinned versions, so --version cannot be used with it");
    has_not("nix-templates repo");
}

#[test]
fn the_help_advertisement_matches_the_nix_check() {
    // `tests/default.nix` greps `^  <cmd> ` in both builds' help. Assert the
    // same lines on the mirror's rendered help so a wording change fails here.
    let full = Full::command().render_help().to_string();
    let slim = Slim::command().render_help().to_string();
    for command in ["  init ", "  add ", "  doctor "] {
        assert!(
            lines(&full).iter().any(|l| l.starts_with(command)),
            "{full}"
        );
        assert!(
            lines(&slim).iter().any(|l| l.starts_with(command)),
            "{slim}"
        );
    }
    for command in ["  update ", "  pin "] {
        assert!(
            lines(&full).iter().any(|l| l.starts_with(command)),
            "{full}"
        );
        assert!(
            !lines(&slim).iter().any(|l| l.starts_with(command)),
            "{slim}"
        );
    }
}
