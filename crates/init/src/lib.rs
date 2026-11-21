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
    pub deno: Option<Deno>,
    pub python: Option<Python>,
    pub go: Option<Go>,
    pub zig: Option<Zig>,
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

#[derive(Debug, PartialEq, Eq)]
pub struct Deno {
    /// `deno.json` or `deno.jsonc`. Neither has a field for the Deno version
    /// (`deno.json` pins dependencies, not the runtime), so the flake takes
    /// the newest pinned release and never reads the file.
    pub file: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Python {
    /// `pyproject.toml` (fromPyprojectToml) or `.python-version`
    /// (fromPythonVersion): the file the flake hands to crucible.
    pub file: &'static str,
    /// Where the Python version comes from, as fromPyprojectToml reads it.
    pub version_from: String,
    /// A .python-version next to pyproject.toml: fromPyprojectToml reads it,
    /// so it belongs in git too.
    pub version_file: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Go {
    /// The go.mod directive fromGoMod will take the release from, such as
    /// `go 1.27` or `toolchain go1.27.4`.
    pub directive: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Zig {
    /// `.minimum_zig_version` from build.zig.zon. Without one,
    /// fromBuildZigZon has nothing to resolve, so the flake takes the newest
    /// pinned release instead.
    pub minimum: Option<String>,
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

    let deno = ["deno.json", "deno.jsonc"]
        .into_iter()
        .find(|name| dir.join(name).is_file())
        .map(|file| Deno { file });

    let python_version = dir.join(".python-version").is_file();
    let pyproject = dir.join("pyproject.toml");
    let python = if pyproject.is_file() {
        let text = fs::read_to_string(&pyproject)
            .with_context(|| format!("cannot read {}", pyproject.display()))?;
        let toml: toml::Value = toml::from_str(&text)
            .with_context(|| format!("cannot parse {}", pyproject.display()))?;
        // The same order as fromPyprojectToml in lib/python/toolchain.nix.
        let version_from = if python_version {
            ".python-version"
        } else if toml
            .get("project")
            .and_then(|project| project.get("requires-python"))
            .is_some()
        {
            "requires-python"
        } else {
            "the newest pinned line"
        };
        Some(Python {
            file: "pyproject.toml",
            version_from: version_from.to_string(),
            version_file: python_version.then_some(".python-version"),
        })
    } else if python_version {
        Some(Python {
            file: ".python-version",
            version_from: ".python-version".to_string(),
            version_file: None,
        })
    } else {
        None
    };

    let go_mod = dir.join("go.mod");
    let go = if go_mod.is_file() {
        let text = fs::read_to_string(&go_mod)
            .with_context(|| format!("cannot read {}", go_mod.display()))?;
        Some(Go {
            directive: go_directive(&text).with_context(|| {
                format!(
                    "{} has no `go` directive, which fromGoMod needs; add one with `go mod edit -go=1.27` and run init again",
                    go_mod.display()
                )
            })?,
        })
    } else {
        None
    };

    let zon = dir.join("build.zig.zon");
    let zig = if zon.is_file() {
        let text =
            fs::read_to_string(&zon).with_context(|| format!("cannot read {}", zon.display()))?;
        let minimum = minimum_zig_version(&text);
        if minimum.is_none() {
            notes.push(
                "build.zig.zon has no .minimum_zig_version, so Zig is the newest pinned release (add one and run init again to pin it)".to_string(),
            );
        }
        Some(Zig { minimum })
    } else {
        None
    };

    Ok(Project {
        rust,
        node,
        deno,
        python,
        go,
        zig,
        notes,
    })
}

/// The directive fromGoMod in lib/go/toolchain.nix picks: `toolchain`, unless
/// it is `default`, then `go`. `//` comments do not count.
fn go_directive(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default().trim())
        .collect();
    let directive = |name: &str| {
        lines.iter().find_map(|line| {
            let (key, value) = line.split_once([' ', '\t'])?;
            let value = value.trim();
            (key == name && !value.is_empty()).then(|| format!("{name} {value}"))
        })
    };
    directive("toolchain")
        .filter(|toolchain| toolchain != "toolchain default")
        .or_else(|| directive("go"))
}

/// `.minimum_zig_version` in either form fromBuildZigZon reads: a string
/// ("0.15.2", Zig 0.13 and later) or the older tuple
/// (`.{ .major = 0, .minor = 15, .patch = 2 }`).
fn minimum_zig_version(text: &str) -> Option<String> {
    let text: String = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let (_, rest) = text.split_once("minimum_zig_version")?;
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    if let Some(string) = rest.strip_prefix('"') {
        let version = string.split('"').next()?.trim().trim_start_matches('v');
        return (!version.is_empty()).then(|| version.to_string());
    }
    let tuple = rest.strip_prefix(".{")?.split('}').next()?;
    let part = |name: &str| {
        let (_, after) = tuple.split_once(name)?;
        let digits: String = after
            .trim_start()
            .strip_prefix('=')?
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        (!digits.is_empty()).then_some(digits)
    };
    Some(format!(
        "{}.{}.{}",
        part(".major")?,
        part(".minor")?,
        part(".patch")?
    ))
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
    let manifest = rust.and_then(|rust| rust.manifest_url.as_deref());
    let shells = shells(project);

    let names: Vec<&str> = shells.iter().map(|shell| shell.name).collect();
    let description = match names.split_last() {
        None => "A project with crucible".to_string(),
        Some((last, [])) => format!("A {last} project with crucible"),
        Some((last, rest)) => format!("A {} and {last} project with crucible", rest.join(", ")),
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
    for shell in &shells {
        out.push('\n');
        out.push_str(&shell.binding);
    }

    out.push_str("    in\n    {\n");
    for (i, shell) in shells.iter().enumerate() {
        if i > 0 {
            out.push_str("      #\n");
        }
        out.push_str(shell.comment);
    }
    out.push_str(
        "      devShells = forAllSystems (system: {\n        default = nixpkgs.legacyPackages.${system}.mkShell {\n",
    );
    match shells.as_slice() {
        [] => out.push_str("          packages = [ ];\n"),
        [shell] => {
            let _ = writeln!(out, "          packages = [ ({}For system) ];", shell.var);
        }
        shells => {
            out.push_str("          packages = [\n");
            for shell in shells {
                let _ = writeln!(out, "            ({}For system)", shell.var);
            }
            out.push_str("          ];\n");
        }
    }
    out.push_str("        };\n      });\n    };\n}\n");
    out
}

/// One language in the dev shell: its `<var>For` binding in the `let`, with
/// the comment above it, and what the shell gets from it.
struct Shell {
    /// For the description.
    name: &'static str,
    var: &'static str,
    binding: String,
    comment: &'static str,
}

/// The project's languages in the README's order, each written the way
/// templates/*/flake.nix writes it.
fn shells(project: &Project) -> Vec<Shell> {
    let mut shells = Vec::new();

    if let Some(rust) = &project.rust {
        let file = rust.file;
        let mut binding = format!(
            "      # {file} is the one place the Rust version lives, for rustup\n      # and CI as much as for Nix."
        );
        if rust.manifest_url.is_some() {
            let _ = write!(
                binding,
                r#"
      rustFor =
        system:
        crucible.lib.${{system}}.rust.fromToolchainFile {{
          file = ./{file};
          manifest = rust-manifest;
        }};
"#
            );
        } else {
            let _ = write!(
                binding,
                r#" To pin a numbered channel ("1.99"), add its
      # manifest as an input and pass it here; `crucible pin rust 1.99` prints both.
      rustFor = system: crucible.lib.${{system}}.rust.fromToolchainFile {{ file = ./{file}; }};
"#
            );
        }
        shells.push(Shell {
            name: "Rust",
            var: "rust",
            binding,
            comment: r#"      # The toolchain's cargo is huacnlee/cargo: every project and worktree
      # shares one build directory under ~/.cargo/targets. Its setup hook sets
      # RUST_SRC_PATH, and rust-analyzer is part of it.
"#,
        });
    }

    if project.node.is_some() {
        shells.push(Shell {
            name: "Node.js",
            var: "node",
            binding:
                r#"      # package.json, .node-version and packageManager stay the single source
      # of truth for nvm/corepack, CI and Nix. Nothing here touches the network
      # at evaluation time: the versions are resolved against crucible's pins.
      nodeFor = system: crucible.lib.${system}.node.fromPackageJson { file = ./package.json; };
"#
                .to_string(),
            comment: r#"      # The Node.js toolchain carries node, npm, vp and the package manager,
      # all running on its own Node.js. Its setup hook keeps vp from
      # downloading a Node.js of its own.
"#,
        });
    }

    if let Some(deno) = &project.deno {
        shells.push(Shell {
            name: "Deno",
            var: "deno",
            binding: format!(
                r#"      # {file} has no field for the Deno version, so this is the newest
      # pinned release; `version = "2";` keeps the shell on one major line.
      denoFor = system: crucible.lib.${{system}}.deno.toolchain {{ }};
"#,
                file = deno.file
            ),
            comment: r#"      # The Deno toolchain is the official deno executable, from the release
      # zip Deno itself publishes.
"#,
        });
    }

    if let Some(python) = &project.python {
        let binding = if python.file == "pyproject.toml" {
            r#"      # pyproject.toml's requires-python and .python-version stay the single
      # source of truth for uv, CI and Nix, resolved against crucible's pins
      # without the network.
      pythonFor = system: crucible.lib.${system}.python.fromPyprojectToml { file = ./pyproject.toml; };
"#
        } else {
            r#"      # .python-version is the one place the Python version lives, for uv
      # and pyenv as much as for Nix.
      pythonFor = system: crucible.lib.${system}.python.fromPythonVersion { file = ./.python-version; };
"#
        };
        shells.push(Shell {
            name: "Python",
            var: "python",
            binding: binding.to_string(),
            comment: r#"      # The Python toolchain carries python3, pip and uv. Its setup hook
      # keeps uv on this Python and stops it downloading one of its own.
"#,
        });
    }

    if project.go.is_some() {
        shells.push(Shell {
            name: "Go",
            var: "go",
            binding:
                r#"      # go.mod's toolchain or go directive names the oldest Go that will do,
      # as the go command reads it, and the pinned release of that line
      # satisfies it as the pins move to newer patch releases.
      goFor = system: crucible.lib.${system}.go.fromGoMod { file = ./go.mod; };
"#
                .to_string(),
            comment: r#"      # The Go toolchain is the official go.dev release. Its setup hook sets
      # GOTOOLCHAIN=local, so go never downloads a toolchain of its own.
"#,
        });
    }

    if let Some(zig) = &project.zig {
        let binding = if zig.minimum.is_some() {
            r#"      # .minimum_zig_version in build.zig.zon is the one place the Zig
      # version lives, for zig itself as much as for Nix.
      zigFor = system: crucible.lib.${system}.zig.fromBuildZigZon { file = ./build.zig.zon; };
"#
        } else {
            r#"      # build.zig.zon has no .minimum_zig_version, so this is the newest
      # pinned release. Once it has one, fromBuildZigZon resolves it.
      zigFor = system: crucible.lib.${system}.zig.toolchain { };
"#
        };
        shells.push(Shell {
            name: "Zig",
            var: "zig",
            binding: binding.to_string(),
            comment:
                "      # The Zig toolchain is the official ziglang.org release, as it ships.\n",
        });
    }

    shells
}

/// Writes `dir/flake.nix` and prints what to do next.
pub fn run(dir: &Path, force: bool) -> Result<()> {
    let project = detect(dir)?;
    for note in &project.notes {
        println!("note: {note}");
    }
    if shells(&project).is_empty() {
        anyhow::bail!(
            "found no rust-toolchain.toml, rust-toolchain, package.json, deno.json, deno.jsonc, pyproject.toml, .python-version, go.mod or build.zig.zon in {}; add one and run init again, or start from a template: nix flake init -t github:kiyors/crucible",
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
    if let Some(deno) = &project.deno {
        let _ = writeln!(
            out,
            "  deno: the newest pinned release ({} names no Deno version)",
            deno.file
        );
    }
    if let Some(python) = &project.python {
        tracked.push(python.file);
        tracked.extend(python.version_file);
        if python.file == python.version_from {
            let _ = writeln!(out, "  python: {}", python.file);
        } else {
            let _ = writeln!(
                out,
                "  python: {} (version from {})",
                python.file, python.version_from
            );
        }
    }
    if let Some(go) = &project.go {
        tracked.push("go.mod");
        let _ = writeln!(out, "  go: go.mod ({})", go.directive);
    }
    if let Some(zig) = &project.zig {
        match &zig.minimum {
            Some(minimum) => {
                tracked.push("build.zig.zon");
                let _ = writeln!(out, "  zig: build.zig.zon (minimum_zig_version {minimum})");
            }
            None => out.push_str("  zig: the newest pinned release\n"),
        }
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
        assert_eq!(cases, 10);
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
    fn detects_the_other_languages() {
        let fixture = |name: &str| detect(&PathBuf::from(FIXTURES).join(name)).unwrap();

        let python = fixture("python");
        assert_eq!(
            python.python,
            Some(Python {
                file: "pyproject.toml",
                version_from: ".python-version".to_string(),
                version_file: Some(".python-version"),
            })
        );
        assert_eq!(
            fixture("python-version").python,
            Some(Python {
                file: ".python-version",
                version_from: ".python-version".to_string(),
                version_file: None,
            })
        );
        assert_eq!(
            fixture("go").go,
            Some(Go {
                directive: "toolchain go1.27.1".to_string()
            })
        );
        assert_eq!(
            fixture("zig").zig,
            Some(Zig {
                minimum: Some("0.17.1".to_string())
            })
        );
        assert_eq!(fixture("deno").deno, Some(Deno { file: "deno.json" }));

        let mixed = fixture("mixed");
        assert!(mixed.rust.is_some() && mixed.node.is_some());
        assert_eq!(mixed.deno, Some(Deno { file: "deno.jsonc" }));
        assert_eq!(
            mixed.python.map(|python| python.version_from),
            Some("requires-python".to_string())
        );
        assert_eq!(
            mixed.go,
            Some(Go {
                directive: "go 1.27.0".to_string()
            })
        );
        assert_eq!(mixed.zig, Some(Zig { minimum: None }));
        assert_eq!(mixed.notes.len(), 1);
    }

    #[test]
    fn reads_go_mod_as_from_go_mod_would() {
        assert_eq!(
            go_directive("module m\n\ngo 1.27 // comment\ntoolchain default\n").as_deref(),
            Some("go 1.27")
        );
        assert_eq!(
            go_directive("module m\ngo 1.26.0\ntoolchain\tgo1.27.4\n").as_deref(),
            Some("toolchain go1.27.4")
        );
        // A comment that only mentions the directive is not one.
        assert_eq!(go_directive("module m\n// go 1.27\n"), None);
    }

    #[test]
    fn reads_build_zig_zon_as_from_build_zig_zon_would() {
        assert_eq!(
            minimum_zig_version(".{ .minimum_zig_version = \"v0.15.2\" }").as_deref(),
            Some("0.15.2")
        );
        assert_eq!(
            minimum_zig_version(
                ".{\n    .minimum_zig_version = .{ .major = 0, .minor = 12, .patch = 1 },\n}"
            )
            .as_deref(),
            Some("0.12.1")
        );
        assert_eq!(
            minimum_zig_version(".{\n    // .minimum_zig_version = \"0.15.2\",\n}"),
            None
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
        let templates = [
            ("rust", include_str!("../../../templates/default/flake.nix")),
            ("node", include_str!("../../../templates/node/flake.nix")),
            (
                "python",
                include_str!("../../../templates/python/flake.nix"),
            ),
            ("go", include_str!("../../../templates/go/flake.nix")),
            ("zig", include_str!("../../../templates/zig/flake.nix")),
            ("deno", include_str!("../../../templates/deno/flake.nix")),
        ];

        let shared = |flake: &str, template: &str| {
            let inputs = template.split("  outputs =").next().unwrap();
            assert!(flake.contains(inputs.split_once("inputs = {").unwrap().1));
            let systems = &template[template.find("      forAllSystems").unwrap()..];
            let systems = &systems[..systems.find("];").unwrap()];
            assert!(flake.contains(systems), "{systems}");
            let bindings: Vec<&str> = template
                .lines()
                .filter(|line| line.contains("For = system:"))
                .collect();
            assert_eq!(bindings.len(), 1);
            for line in bindings {
                assert!(flake.contains(line), "{line}");
            }
        };
        for (fixture, template) in templates {
            let flake = render(&detect(&PathBuf::from(FIXTURES).join(fixture)).unwrap());
            shared(&flake, template);
        }
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
