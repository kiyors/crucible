//! `nix-cargo`: the repo's updater, environment doctor and channel pin helper.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use nix_cargo::update::Scope;
use nix_cargo::{doctor, pin, update};

#[derive(Parser)]
#[command(
    name = "nix-cargo",
    version,
    about = "Refresh nix-cargo's pins and diagnose a Rust environment"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rewrite sources.json and data/*.json from upstream
    Update {
        /// What to refresh
        #[arg(value_enum, default_value = "all")]
        what: Scope,
        /// Use this huacnlee/cargo version instead of the latest release
        #[arg(long)]
        version: Option<String>,
        /// Repository root holding sources.json and data/
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Check that the fork is what cargo resolves to
    Doctor,
    /// Print the flake input and sha256 for a rustup channel
    Pin {
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
        Command::Update {
            what,
            version,
            root,
        } => {
            update::run(what, version, &root)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor => doctor::run(),
        Command::Pin { channel } => {
            pin::run(&channel)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
