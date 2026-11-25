//! `crucible init <directory>` and `crucible add`: create a new project, or
//! add a template to one that already exists. Both shell out to the same
//! `nix flake new` and `nix flake init --template` the user's shell functions
//! used, so github:kiyors/templates stays the single source of truth for what
//! a template installs. Per-tool setup comes later.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The repo every supported template name is in.
const NIX_TEMPLATE_URL: &str = "github:kiyors/templates";
/// The template both commands fall back to.
const DEFAULT_TEMPLATE: &str = "empty";

/// `crucible init <directory>`: create a new project from a template.
pub fn init(directory: &Path, template: Option<&str>) -> Result<()> {
    if directory.exists() {
        bail!(
            "directory {} already exists; `crucible init` creates new projects",
            directory.display()
        );
    }
    let template = template.unwrap_or(DEFAULT_TEMPLATE);
    let ref_arg = template_ref(template);
    let dir_arg = directory.to_string_lossy().into_owned();
    run_nix(
        None,
        &[
            "flake",
            "new",
            dir_arg.as_str(),
            "--template",
            ref_arg.as_str(),
        ],
    )?;
    println!("created {}", directory.display());
    Ok(())
}

/// `crucible add`: add a template to the project in the current directory.
pub fn add(template: Option<&str>) -> Result<()> {
    add_in(&std::env::current_dir()?, template)
}

fn add_in(dir: &Path, template: Option<&str>) -> Result<()> {
    let template = template.unwrap_or(DEFAULT_TEMPLATE);
    if dir_is_empty(dir) {
        bail!("the directory is empty; use `crucible init <directory>` to start a new project");
    }
    let ref_arg = template_ref(template);
    run_nix(
        Some(dir),
        &["flake", "init", "--template", ref_arg.as_str()],
    )?;
    Ok(())
}

/// The `--template` ref for a name: `github:kiyors/templates#<name>`.
fn template_ref(template: &str) -> String {
    format!("{NIX_TEMPLATE_URL}#{template}")
}

fn run_nix(dir: Option<&Path>, args: &[&str]) -> Result<()> {
    let status = Command::new("nix")
        .args(args)
        .current_dir(dir.unwrap_or(Path::new(".")))
        .status()
        .with_context(|| format!("failed to run nix {}", args.join(" ")))?;
    if !status.success() {
        bail!("nix {} exited with {}", args.join(" "), status);
    }
    Ok(())
}

fn dir_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_rejects_an_existing_directory() {
        let dir = std::env::temp_dir().join("crucible-init-exists");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(init(&dir, None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_in_rejects_an_empty_directory() {
        let dir = std::env::temp_dir().join("crucible-add-empty");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(add_in(&dir, None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_ref_names_the_repo_and_template() {
        assert_eq!(template_ref("rust"), "github:kiyors/templates#rust");
        assert_eq!(DEFAULT_TEMPLATE, "empty");
    }
}
