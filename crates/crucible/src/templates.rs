//! `crucible init <directory>` and `crucible add`: create a new project, or
//! add a template to one that already exists. Both shell out to the same
//! `nix flake new` and `nix flake init --template` the user's shell functions
//! used, so this flake's `templates/` is the single source of truth for what
//! a template installs. Per-tool setup comes later.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The repo every supported template name is in. This flake is canonical: a
/// second, external template repo would need another flake input and an
/// offline check of its names, neither of which fits the one-input design.
const NIX_TEMPLATE_URL: &str = "github:kiyors/crucible";
/// The template both commands fall back to.
const DEFAULT_TEMPLATE: &str = "empty";

/// Every attr in `templates.nix`, mapped to the directory it installs. Keep
/// this list in step with that file: `hash` is the flake attr for `hashi`, and
/// `c` and `cpp` alias `c-cpp` without directories of their own. Names not in
/// this list are rejected before `nix flake new` can fail opaquely.
const TEMPLATE_DIRS: [(&str, &str); 27] = [
    ("default", "default"),
    ("node", "node"),
    ("python", "python"),
    ("go", "go"),
    ("zig", "zig"),
    ("deno", "deno"),
    ("bun", "bun"),
    ("c-cpp", "c-cpp"),
    ("elixir", "elixir"),
    ("elm", "elm"),
    ("empty", "empty"),
    ("hash", "hashi"),
    ("java", "java"),
    ("jupyter", "jupyter"),
    ("kotlin", "kotlin"),
    ("laravel", "laravel"),
    ("nim", "nim"),
    ("nix", "nix"),
    ("odin", "odin"),
    ("php", "php"),
    ("rust", "rust"),
    ("scala", "scala"),
    ("shell", "shell"),
    ("swift", "swift"),
    ("uv", "uv"),
    ("c", "c-cpp"),
    ("cpp", "c-cpp"),
];

/// `crucible init <directory>`: create a new project from a template.
pub fn init(directory: &Path, template: Option<&str>) -> Result<()> {
    if directory.exists() {
        bail!(
            "directory {} already exists; `crucible init` creates new projects",
            directory.display()
        );
    }
    let template = template.unwrap_or(DEFAULT_TEMPLATE);
    let ref_arg = template_ref(template)?;
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
    let ref_arg = template_ref(template)?;
    run_nix(
        Some(dir),
        &["flake", "init", "--template", ref_arg.as_str()],
    )?;
    Ok(())
}

/// Resolve a template attr to its directory, so a name that is only a
/// directory (`hashi`, `c-cpp`) or is unknown is rejected with a useful hint.
fn template_dir(name: &str) -> Result<&'static str> {
    if let Some((_, dir)) = TEMPLATE_DIRS.iter().find(|(attr, _)| *attr == name) {
        return Ok(dir);
    }
    if let Some((attr, _)) = TEMPLATE_DIRS.iter().find(|(_, dir)| *dir == name) {
        bail!("crucible: `{name}` is a template directory; use the flake attr `{attr}`");
    }
    let names = TEMPLATE_DIRS
        .iter()
        .map(|(attr, _)| *attr)
        .collect::<Vec<_>>()
        .join(", ");
    bail!("crucible: no template named `{name}` in {NIX_TEMPLATE_URL}; choose one of: {names}")
}

/// The `--template` ref for an attr: `github:kiyors/crucible#<name>`. The ref
/// names the flake attr, not the directory it points at.
fn template_ref(template: &str) -> Result<String> {
    template_dir(template)?;
    Ok(format!("{NIX_TEMPLATE_URL}#{template}"))
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
        assert_eq!(template_ref("rust").unwrap(), "github:kiyors/crucible#rust");
        assert_eq!(DEFAULT_TEMPLATE, "empty");
    }

    #[test]
    fn template_names_are_the_flakes_attrs() {
        for (attr, dir) in TEMPLATE_DIRS {
            assert_eq!(template_dir(attr).unwrap(), dir);
            assert!(template_ref(attr)
                .unwrap()
                .starts_with("github:kiyors/crucible#"));
        }
        // The aliases name flake attrs, not directories.
        assert_eq!(template_dir("hash").unwrap(), "hashi");
        assert_eq!(template_dir("c").unwrap(), "c-cpp");
        assert_eq!(template_dir("cpp").unwrap(), "c-cpp");
        assert!(template_dir("hashi").is_err());
        assert!(template_dir("not-a-template").is_err());
    }
}
