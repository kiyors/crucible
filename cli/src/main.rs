//! `crucible`: the repo's updater, environment doctor and channel pin helper.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crucible::update::Scope;
use crucible::{doctor, pin, update};

#[derive(Parser)]
#[command(
    name = "crucible",
    version,
    about = "Refresh crucible's pins and diagnose a toolchain environment"
)]
struct Cli {
    // Optional so a bare `nix run github:kiyors/crucible` diagnoses the
    // caller's shell: `doctor` is the one command most users ever need.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Rewrite the pins in data/ from upstream
    Update {
        /// What to refresh
        #[arg(value_enum, default_value = "all")]
        what: Scope,
        /// Use this huacnlee/cargo version instead of the latest release
        #[arg(long)]
        version: Option<String>,
        /// Repository root holding data/
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Check that the fork is what cargo resolves to
    Doctor,
    /// Print what a project needs to pin a toolchain version
    #[command(subcommand)]
    Pin(Pin),
}

#[derive(Subcommand)]
enum Pin {
    /// The flake input and sha256 for a rustup channel
    Rust {
        /// A rustup channel: stable, 1.99, nightly-2026-11-01, ...
        channel: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        None | Some(Command::Doctor) => doctor::run(),
        Some(Command::Update {
            what,
            version,
            root,
        }) => {
            update::run(what, version, &root)?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Command::Pin(Pin::Rust { channel })) => {
            pin::run(&channel)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
