//! `crucible init`: write a starter flake.nix whose dev shell takes its
//! toolchains from the files a project already has, the way the README and
//! templates/ use crucible.
//!
//! It never touches the network, so it is part of the doctor-only build that a
//! bare `nix run github:kiyors/crucible` runs: starting a flake is a first-run
//! command too, and a cold cache should not have to compile rustls for it.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::Path;

use anyhow::{Context, Result};

pub mod channel;

/// The systems the flake builds for, in order. Must match
/// lib/common/systems.nix; the `init-flakes` check compares them.
pub const SYSTEMS: [&str; 3] = ["x86_64-linux", "aarch64-linux", "aarch64-darwin"];

/// What `init` found in a directory, and so what the flake will use.
#[derive(Debug, PartialEq, Eq)]
pub struct Project {
    pub rust: Option<Rust>,
    pub node: Option<Node>,
    /// Things worth saying that do not stop init, such as a .nvmrc without a
    /// package.json.
    pub notes: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Rust {
    /// `rust-toolchain.toml` or `rust-toolchain`.
    pub file: &'static str,
    pub channel: String,
    /// The `rust-manifest` input's URL, for a channel crucible does not pin.
    pub manifest_url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Node {
    /// Where fromPackageJson will take the Node.js version from.
    pub version_from: String,
    /// .node-version or .nvmrc, when there is one: it belongs in git too.
    pub version_file: Option<&'static str>,
}

/// Looks only in `dir` itself: the flake refers to the files as `./name`, and
/// a flake cannot read a path outside its own directory tree.
pub fn detect(dir: &Path) -> Result<Project> {
    let mut notes = Vec::new();

    let rust = match channel::TOOLCHAIN_FILES
        .into_iter()
        .find(|name| dir.join(name).is_file())
    {
        None => None,
        Some(file) => {
            let path = dir.join(file);
            let text = fs::read_to_string(&path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            let channel = channel::toolchain_channel(Path::new(file), &text)?;
            let manifest_url = if channel::is_pinned(&channel) {
                None
            } else {
                Some(
                    channel::manifest_url(&channel)
                        .with_context(|| format!("{file} names a channel crucible cannot pin"))?,
                )
            };
            Some(Rust {
                file,
                channel,
                manifest_url,
            })
        }
    };

    let version_file = [".node-version", ".nvmrc"]
        .into_iter()
        .find(|name| dir.join(name).is_file());
    let package_json = dir.join("package.json");
    let node = if package_json.is_file() {
        let text = fs::read_to_string(&package_json)
            .with_context(|| format!("cannot read {}", package_json.display()))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse {}", package_json.display()))?;
        Some(Node {
            version_from: node_version_from(&json, version_file),
            version_file,
        })
    } else {
        if let Some(name) = version_file {
            notes.push(format!(
                "{name} found without a package.json; fromPackageJson needs one, so Node.js is left out (add a package.json and run init again)"
            ));
        }
        None
    };

    Ok(Project { rust, node, notes })
}

/// The same order as fromPackageJson in lib/node/toolchain.nix:
/// devEngines.runtime, then the version file, then engines.node, then LTS.
fn node_version_from(json: &serde_json::Value, version_file: Option<&str>) -> String {
    let runtime = json.pointer("/devEngines/runtime").is_some_and(|runtime| {
        let entries = match runtime {
            serde_json::Value::Array(entries) => entries.iter().collect(),
            entry => vec![entry],
        };
        entries.iter().any(|entry| {
            entry.get("name").and_then(|name| name.as_str()) == Some("node")
                && entry.get("version").is_some()
        })
    });
    if runtime {
        "devEngines.runtime".to_string()
    } else if let Some(name) = version_file {
        name.to_string()
    } else if json.pointer("/engines/node").is_some() {
        "engines.node".to_string()
    } else {
        "the pinned LTS".to_string()
    }
}

/// The flake, formatted as nixfmt would leave it, so `nix fmt` on a fresh
/// project changes nothing. The pieces follow templates/*/flake.nix.
pub fn render(project: &Project) -> String {
    let rust = project.rust.as_ref();
    let node = project.node.as_ref();
    let manifest = rust.and_then(|rust| rust.manifest_url.as_deref());

    let description = match (rust, node) {
        (Some(_), Some(_)) => "A Rust and Node.js project with crucible",
        (None, Some(_)) => "A Node.js project with crucible",
        _ => "A Rust project with crucible",
    };

    let mut out = String::new();
    let _ = write!(
        out,
        r#"{{
  description = "{description}";

  inputs = {{
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crucible = {{
      url = "github:kiyors/crucible";
      inputs.nixpkgs.follows = "nixpkgs";
    }};
"#
    );
    if let (Some(rust), Some(url)) = (rust, manifest) {
        let _ = write!(
            out,
            r#"    # Must match `channel` in {file}.
    rust-manifest = {{
      url = "{url}";
      flake = false;
    }};
"#,
            file = rust.file
        );
    }
    out.push_str("  };\n\n  outputs =\n");
    if manifest.is_some() {
        out.push_str(
            "    {\n      nixpkgs,\n      crucible,\n      rust-manifest,\n      ...\n    }:\n",
        );
    } else {
        out.push_str("    { nixpkgs, crucible, ... }:\n");
    }
    out.push_str("    let\n      forAllSystems = nixpkgs.lib.genAttrs [\n");
    for system in SYSTEMS {
        let _ = writeln!(out, "        \"{system}\"");
    }
    out.push_str("      ];\n");

    if let Some(rust) = rust {
        let file = rust.file;
        let _ = writeln!(
            out,
            "\n      # {file} is the one place the Rust version lives, for rustup"
        );
        out.push_str("      # and CI as much as for Nix.");
        if manifest.is_none() {
            out.push_str(
                r#" To pin a numbered channel ("1.99"), add its
      # manifest as an input and pass it here; `crucible pin rust 1.99` prints both."#,
            );
        }
        out.push('\n');
        if manifest.is_some() {
            let _ = write!(
                out,
                r#"      rustFor =
        system:
        crucible.lib.${{system}}.rust.fromToolchainFile {{
          file = ./{file};
          manifest = rust-manifest;
        }};
"#
            );
        } else {
            let _ = writeln!(
                out,
                "      rustFor = system: crucible.lib.${{system}}.rust.fromToolchainFile {{ file = ./{file}; }};"
            );
        }
    }
    if node.is_some() {
        out.push_str(
            r#"
      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };
"#,
        );
    }

    out.push_str("    in\n    {\n");
    if rust.is_some() {
        out.push_str(
            r#"      # The toolchain's cargo is huacnlee/cargo: every project and worktree
      # shares one build directory under ~/.cargo/targets. Its setup hook sets
      # RUST_SRC_PATH, and rust-analyzer is part of it.
"#,
        );
    }
    if node.is_some() {
        if rust.is_some() {
            out.push_str("      #\n");
        }
        out.push_str(
            r#"      # The Node.js toolchain carries node, npm, vp and the package manager,
      # all running on its own Node.js. Its setup hook keeps vp from
      # downloading a Node.js of its own.
"#,
        );
    }
    out.push_str(
        "      devShells = forAllSystems (system: {\n        default = nixpkgs.legacyPackages.${system}.mkShell {\n",
    );
    match (rust, node) {
        (Some(_), Some(_)) => out.push_str(
            "          packages = [\n            (rustFor system)\n            (nodeFor system)\n          ];\n",
        ),
        (None, Some(_)) => out.push_str("          packages = [ (nodeFor system) ];\n"),
        _ => out.push_str("          packages = [ (rustFor system) ];\n"),
    }
    out.push_str("        };\n      });\n    };\n}\n");
    out
}

/// Writes `dir/flake.nix` and prints what to do next.
pub fn run(dir: &Path, force: bool) -> Result<()> {
    let project = detect(dir)?;
    for note in &project.notes {
        println!("note: {note}");
    }
    if project.rust.is_none() && project.node.is_none() {
        anyhow::bail!(
            "found no rust-toolchain.toml, rust-toolchain or package.json in {}; add one and run init again, or start from a template: nix flake init -t github:kiyors/crucible",
            dir.display()
        );
    }

    let path = dir.join("flake.nix");
    let flake = render(&project);
    // create_new so that a flake.nix appearing between a check and the write
    // is never clobbered either.
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .create_new(!force)
        .open(&path);
    let mut file = match file {
        Err(err) if err.kind() == ErrorKind::AlreadyExists => anyhow::bail!(
            "{} already exists; pass --force to replace it",
            path.display()
        ),
        result => result.with_context(|| format!("cannot write {}", path.display()))?,
    };
    file.write_all(flake.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;

    println!("wrote {}, with a dev shell for", path.display());
    print!("{}", next_steps(dir, &project));
    Ok(())
}

/// What the new flake uses, and the commands that take it from a file to a
/// working shell.
fn next_steps(dir: &Path, project: &Project) -> String {
    let mut out = String::new();
    let mut tracked = vec!["flake.nix"];
    if let Some(rust) = &project.rust {
        tracked.push(rust.file);
        match &rust.manifest_url {
            Some(_) => {
                let _ = writeln!(
                    out,
                    "  rust: {} (channel {}, through the rust-manifest input)",
                    rust.file, rust.channel
                );
            }
            None => {
                let _ = writeln!(out, "  rust: {} (channel {})", rust.file, rust.channel);
            }
        }
    }
    if let Some(node) = &project.node {
        tracked.push("package.json");
        tracked.extend(node.version_file);
        let _ = writeln!(
            out,
            "  node: package.json (version from {})",
            node.version_from
        );
    }

    out.push_str("\nnext steps:\n");
    // A flake in a git repository sees only the files git knows about, so an
    // untracked flake.nix or toolchain file reads as a missing one.
    if dir.ancestors().any(|dir| dir.join(".git").exists()) {
        let _ = writeln!(out, "  git add {}", tracked.join(" "));
    }
    out.push_str("  nix develop\n");
    let envrc = fs::read_to_string(dir.join(".envrc")).unwrap_or_default();
    if on_path("direnv") && !envrc.lines().any(|line| line.trim() == "use flake") {
        out.push_str("  echo 'use flake' >> .envrc && direnv allow\n");
    }
    out
}

fn on_path(name: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|path| env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/init");

    /// The golden flakes are also evaluated by the `init-flakes` check, so
    /// matching them means the generated flake evaluates.
    #[test]
    fn renders_the_golden_flakes() {
        let mut cases = 0;
        for entry in fs::read_dir(FIXTURES).unwrap() {
            let dir = entry.unwrap().path();
            let project = detect(&dir).unwrap();
            let golden = fs::read_to_string(dir.join("flake.nix")).unwrap();
            assert_eq!(render(&project), golden, "{}", dir.display());
            cases += 1;
        }
        assert_eq!(cases, 4);
    }

    #[test]
    fn detects_numbered_channels_and_version_sources() {
        let project = detect(&PathBuf::from(FIXTURES).join("rust-numbered")).unwrap();
        assert_eq!(
            project.rust,
            Some(Rust {
                file: "rust-toolchain",
                channel: "1.100".to_string(),
                manifest_url: Some(
                    "https://static.rust-lang.org/dist/channel-rust-1.100.toml".to_string()
                ),
            })
        );
        assert_eq!(project.node, None);

        let project = detect(&PathBuf::from(FIXTURES).join("node")).unwrap();
        assert_eq!(
            project.node,
            Some(Node {
                version_from: ".node-version".to_string(),
                version_file: Some(".node-version"),
            })
        );
    }

    #[test]
    fn reads_the_version_source_from_package_json_would() {
        let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        let dev = json(
            r#"{"devEngines":{"runtime":[{"name":"node","version":"24"}]},"engines":{"node":"22"}}"#,
        );
        assert_eq!(
            node_version_from(&dev, Some(".nvmrc")),
            "devEngines.runtime"
        );
        let engines = json(r#"{"engines":{"node":">=22"}}"#);
        assert_eq!(node_version_from(&engines, Some(".nvmrc")), ".nvmrc");
        assert_eq!(node_version_from(&engines, None), "engines.node");
        assert_eq!(node_version_from(&json("{}"), None), "the pinned LTS");
    }

    /// init's text is a trimmed copy of the templates; if one changes, the
    /// other should follow.
    #[test]
    fn shares_its_pieces_with_the_templates() {
        // crates/crucible/package.nix keeps these in the sandboxed source.
        let rust_template = include_str!("../../../templates/default/flake.nix");
        let node_template = include_str!("../../../templates/node/flake.nix");
        let rust = render(&detect(&PathBuf::from(FIXTURES).join("rust")).unwrap());
        let node = render(&detect(&PathBuf::from(FIXTURES).join("node")).unwrap());

        let shared = |flake: &str, template: &str| {
            let inputs = template.split("  outputs =").next().unwrap();
            assert!(flake.contains(inputs.split_once("inputs = {").unwrap().1));
            let systems = &template[template.find("      forAllSystems").unwrap()..];
            let systems = &systems[..systems.find("];").unwrap()];
            assert!(flake.contains(systems), "{systems}");
            for line in template
                .lines()
                .filter(|line| line.contains("For = system:"))
            {
                assert!(flake.contains(line), "{line}");
            }
        };
        shared(&rust, rust_template);
        shared(&node, node_template);
    }

    #[test]
    fn refuses_to_overwrite_without_force() {
        let dir = std::env::temp_dir().join(format!("crucible-init-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"stable\"\n",
        )
        .unwrap();
        fs::write(dir.join("flake.nix"), "{ }\n").unwrap();

        let err = run(&dir, false).unwrap_err().to_string();
        assert!(err.contains("pass --force"), "{err}");
        assert_eq!(fs::read_to_string(dir.join("flake.nix")).unwrap(), "{ }\n");

        run(&dir, true).unwrap();
        assert!(fs::read_to_string(dir.join("flake.nix"))
            .unwrap()
            .contains("fromToolchainFile { file = ./rust-toolchain.toml; }"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn needs_a_project_file() {
        let dir = std::env::temp_dir().join(format!("crucible-init-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".nvmrc"), "24\n").unwrap();
        let project = detect(&dir).unwrap();
        assert_eq!(project.node, None);
        assert_eq!(project.notes.len(), 1);
        assert!(run(&dir, false).is_err());
        assert!(!dir.join("flake.nix").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
