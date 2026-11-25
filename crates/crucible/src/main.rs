//! `crucible`: start a toolchain flake and diagnose its environment; the
//! `net` build adds the repo's pin updater and channel pin helper.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod templates;

#[cfg(feature = "net")]
use pins::{pin, update};

// A bare `crucible` prints the commands instead of picking one for the
// user: `doctor` reads the environment and exits non-zero on a broken one,
// which is a surprising answer to "what does this do?".
#[derive(Parser)]
#[command(name = "crucible", version, arg_required_else_help = true)]
#[cfg_attr(
    feature = "net",
    command(
        about = "Start a toolchain flake, diagnose its environment and refresh crucible's pins"
    )
)]
#[cfg_attr(
    not(feature = "net"),
    command(about = "Start a toolchain flake and diagnose its environment")
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rewrite the pins in data/ from upstream
    #[cfg(feature = "net")]
    Update {
        /// What to refresh
        #[arg(
            default_value = "all",
            value_parser = clap::builder::PossibleValuesParser::new(pins::update::SCOPES)
        )]
        what: String,
        /// Use this huacnlee/cargo version instead of the latest release
        /// (with `cargo` or the default `all` scope)
        #[arg(long)]
        version: Option<String>,
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
        /// Template name from github:kiyors/templates (default: empty)
        template: Option<String>,
    },
    /// Add a template to the project in the current directory
    Add {
        /// Template name from github:kiyors/templates (default: empty)
        template: Option<String>,
    },
    /// Print what a project needs to pin a toolchain version
    #[cfg(feature = "net")]
    #[command(subcommand)]
    Pin(Pin),
    // The user build (`packages.crucible`, what `nix run
    // github:kiyors/crucible` runs) leaves out `update` and `pin`, so
    // `-- pin rust 1.99` lands here; say where the command went instead of
    // clap's bare "unrecognized subcommand".
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

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", format_error(&err));
            ExitCode::FAILURE
        }
    }
}

/// Library errors rarely carry the `crucible:` prefix, so add it once at the
/// CLI boundary instead of repeating it in every context string. A handful of
/// messages (gpg.rs) already start with it, so leave those alone.
fn format_error(err: &anyhow::Error) -> String {
    let message = format!("{err:#}");
    if message.starts_with("crucible:") {
        message
    } else {
        format!("crucible: {message}")
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        Command::Doctor => doctor::run(),
        Command::Init {
            force,
            directory,
            template,
        } => {
            if let Some(directory) = directory {
                templates::init(&directory, template.as_deref())?;
            } else {
                if template.is_some() {
                    anyhow::bail!(
                        "a template needs a directory: `crucible init <directory> [template]`"
                    );
                }
                init::run(&std::env::current_dir()?, force)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Add { template } => {
            templates::add(template.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(not(feature = "net"))]
        Command::Missing(args) => anyhow::bail!(
            "`{}` is a maintainer command, left out of this build; run `nix run github:kiyors/crucible#crucible-full -- {}`",
            args[0],
            args.join(" ")
        ),
        #[cfg(feature = "net")]
        Command::Update {
            what,
            version,
            root,
        } => {
            update::run(update::parse_scope(&what)?, version, &root)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Command::Pin(Pin::Rust { channel }) => {
            pin::run(&channel)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Command::Pin(Pin::Node { version }) => {
            pin::node(&version)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Command::Pin(Pin::Go { version }) => {
            pin::go(&version)?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "net")]
        Command::Pin(Pin::Python { version }) => {
            pin::python(&version)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn errors_get_one_prefix_and_keep_their_chain() {
        assert_eq!(
            format_error(&anyhow!("no python3 is on PATH")),
            "crucible: no python3 is on PATH"
        );
        assert_eq!(
            format_error(&anyhow!("no such file").context("failed to read flake.nix")),
            "crucible: failed to read flake.nix: no such file"
        );
    }

    #[test]
    fn an_existing_prefix_is_not_doubled() {
        assert_eq!(
            format_error(&anyhow!("crucible: refresh data/node/release-keys.asc")),
            "crucible: refresh data/node/release-keys.asc"
        );
    }
}
