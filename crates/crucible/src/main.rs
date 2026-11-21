//! `crucible`: the repo's updater, environment doctor and channel pin helper.

#[cfg(feature = "net")]
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[cfg(feature = "net")]
use pins::{pin, update, update::Scope};

#[derive(Parser)]
#[command(name = "crucible", version)]
#[cfg_attr(
    feature = "net",
    command(about = "Refresh crucible's pins and diagnose a toolchain environment")
)]
#[cfg_attr(
    not(feature = "net"),
    command(about = "Diagnose a toolchain environment (doctor-only build)")
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
    #[cfg(feature = "net")]
    Update {
        /// What to refresh
        #[arg(value_enum, default_value = "all")]
        what: Scope,
        /// Use this huacnlee/cargo version instead of the latest release
        /// (`update cargo` only)
        #[arg(long)]
        version: Option<String>,
        /// Repository root holding data/
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Check that the fork is what cargo resolves to
    Doctor,
    /// Print what a project needs to pin a toolchain version
    #[cfg(feature = "net")]
    #[command(subcommand)]
    Pin(Pin),
    // The doctor-only build is what a bare `nix run github:kiyors/crucible`
    // runs, so `-- pin rust 1.99` lands here; say where the command went
    // instead of clap's bare "unrecognized subcommand".
    #[cfg(not(feature = "net"))]
    #[command(external_subcommand)]
    Missing(Vec<String>),
}

#[cfg(feature = "net")]
#[derive(Subcommand)]
enum Pin {
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
        #[cfg(not(feature = "net"))]
        Some(Command::Missing(args)) => anyhow::bail!(
            "`{}` is not in this doctor-only build; run `nix run github:kiyors/crucible#crucible -- {}`",
            args[0],
            args.join(" ")
        ),
        #[cfg(feature = "net")]
        Some(Command::Update {
            what,
            version,
            root,
        }) => {
            update::run(what, version, &root)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Some(Command::Pin(Pin::Rust { channel })) => {
            pin::run(&channel)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Some(Command::Pin(Pin::Node { version })) => {
            pin::node(&version)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
