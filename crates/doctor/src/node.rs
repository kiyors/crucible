//! Node.js and the package manager a project pins. The checks compare the
//! active node with `.node-version`/`.nvmrc`, and the package manager on PATH
//! with `packageManager` or `devEngines.packageManager`, the same fields
//! fromPackageJson reads. They also look for the settings lib/node and the
//! Home Manager module warn about: another node, a vp install of its own,
//! NODE_PATH, and a package manager wrapped around a different Node.js.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    find_nearest_file, first_version_line, shadowed, stdout_of, version_matches, Checks, Env,
};

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let node = check_node(checks, env);
    check_node_env(checks, env);
    check_node_version_file(checks, cwd, node.as_deref());
    check_package_json(checks, cwd, env);
}

/// The first node on PATH is the one projects run. Crucible's toolchain is
/// authoritative in a dev shell; outside one, another node wins and vp may
/// download its own. Nodes further down PATH are only a hazard when the first
/// disappears, so they are a warning, not an error.
fn check_node(checks: &mut Checks, env: &Env) -> Option<String> {
    let nodes = env.executables("node");
    let Some(first) = nodes.first() else {
        checks.info("no node on PATH; Node.js checks skipped".to_string());
        return None;
    };
    let version = stdout_of(checks, first, &["--version"], &[])?;
    checks.ok(format!("{} is node {version}", first.display()));
    for message in shadowed("node", &nodes) {
        checks.warn(message);
    }
    Some(version)
}

/// State that leaks between projects or makes vp ignore the toolchain's
/// Node.js. Crucible's setup hook clears the two vp variables in a dev shell;
/// outside one, a stale value changes what vp does.
fn check_node_env(checks: &mut Checks, env: &Env) {
    match env.var("VP_HOME") {
        Some(value) => checks.warn(format!(
            "VP_HOME={value}; a standalone vp install shares its config with this one, and a \
             managed-Node config stops vp with VP_ENV_FORBIDDEN=1"
        )),
        None => checks.ok("VP_HOME is not set".to_string()),
    }

    // The setup hook exports 1; 0 lets vp fetch a Node.js of its own.
    match env.var("VP_ENV_FORBIDDEN") {
        Some(value) if value == "1" => checks.ok("VP_ENV_FORBIDDEN=1".to_string()),
        Some(value) => checks.warn(format!(
            "VP_ENV_FORBIDDEN={value}; crucible's vp needs 1 to use the Node.js on PATH"
        )),
        None => checks.info(
            "VP_ENV_FORBIDDEN is not set; vp may manage its own Node.js outside a dev shell"
                .to_string(),
        ),
    }

    match env.var("NODE_PATH") {
        Some(value) => checks.warn(format!(
            "NODE_PATH={value}; it adds modules to every project from outside its own node_modules"
        )),
        None => checks.ok("NODE_PATH is not set".to_string()),
    }

    // npm installs globally into its own prefix, which is in the read-only
    // store for a Nix toolchain; NPM_CONFIG_PREFIX is the supported escape.
    let prefix = env
        .var("NPM_CONFIG_PREFIX")
        .or_else(|| env.var("npm_config_prefix"));
    match prefix {
        Some(value) if value.starts_with("/nix/store") => checks.warn(format!(
            "npm's global prefix is {value}, inside the read-only Nix store; \
             npm install --global will fail"
        )),
        Some(value) => checks.ok(format!("npm's global prefix is {value}")),
        None => checks.info("npm's global prefix is its own (read-only) store path".to_string()),
    }
}

/// `.node-version` and `.nvmrc` are the pins version managers read. Compare
/// the nearest one with the active node, the same way a rust-toolchain file is
/// compared with rustc.
fn check_node_version_file(checks: &mut Checks, cwd: &Path, node: Option<&str>) {
    let Some(path) = find_nearest_file(cwd, &[".node-version", ".nvmrc"]) else {
        checks.ok("no .node-version or .nvmrc here or in a parent directory".to_string());
        return;
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let Some(pin) = first_version_line(&text) else {
        checks.error(format!("{} names no version", path.display()));
        return;
    };
    match version_matches(&pin, node.unwrap_or_default()) {
        Some(true) => checks.ok(format!(
            "{} pins {pin}, which matches node {}",
            path.display(),
            node.unwrap_or_default()
        )),
        Some(false) => checks.error(format!(
            "{} pins {pin}, but node is {}",
            path.display(),
            node.unwrap_or_default()
        )),
        // Aliases such as `lts/*` and `node` are resolved by the toolchain,
        // not by a version we can compare here.
        None => checks.info(format!(
            "{} pins {pin}, which is an alias or a range doctor cannot read; not compared with node",
            path.display()
        )),
    }
}

/// packageManager is corepack's pin for the project's package manager. A
/// different one on PATH means the lockfile and the shell disagree.
fn check_package_json(checks: &mut Checks, cwd: &Path, env: &Env) {
    let Some(path) = find_nearest_file(cwd, &["package.json"]) else {
        checks.ok("no package.json here or in a parent directory".to_string());
        return;
    };
    let json: serde_json::Value = match fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str(&text).map_err(anyhow::Error::from))
    {
        Ok(json) => json,
        Err(err) => {
            checks.error(format!("cannot parse {}: {err}", path.display()));
            return;
        }
    };
    let Some((field, spec)) = package_manager_spec(&json) else {
        checks.ok(format!(
            "{} sets no packageManager or devEngines.packageManager",
            path.display()
        ));
        return;
    };
    let Some((name, wanted)) = parse_package_manager(&spec) else {
        checks.error(format!(
            "{} sets {field} {spec}, which is not <name>@<version>",
            path.display()
        ));
        return;
    };
    let pin = format!("{} pins {spec} in {field}", path.display());
    let fix = format!(
        "use crucible's `packageManager = \"{name}@{}\"` in `lib.<system>.node.toolchain` (or \
         `fromPackageJson`), or pass a package built from the release you need",
        major_of(&wanted).unwrap_or("<major>")
    );

    let managers = env.executables(&name);
    let Some(first) = managers.first() else {
        checks.warn(format!("{pin}, but no {name} is on PATH; {fix}"));
        return;
    };
    for message in shadowed(&name, &managers) {
        checks.warn(message);
    }

    match Command::new(first).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let actual = stdout.trim();
            match version_matches(&wanted, actual) {
                Some(true) => checks.ok(format!("{} is {name} {actual}, which matches {spec}", first.display())),
                Some(false) => checks.warn(format!(
                    "{} is {name} {actual}, but {pin} (lockfile drift or a shim); {fix}",
                    first.display()
                )),
                None => checks.info(format!(
                    "{pin}, which doctor cannot compare with {name} {actual} ({}); check it by hand",
                    first.display()
                )),
            }
        }
        Ok(output) => checks.warn(format!(
            "{} --version failed: {}",
            first.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) => checks.warn(format!("cannot run {}: {err}", first.display())),
    }

    // A missing node was already reported by check_node.
    if let Some(node) = env.executables("node").first() {
        check_manager_node(checks, &name, first, node);
    }
}

/// The package manager spec package.json asks for, and the field it came
/// from: `packageManager`, then the first `devEngines.packageManager` entry
/// with a name and a version, the order fromPackageJson in
/// lib/node/toolchain.nix (and corepack) uses.
fn package_manager_spec(json: &serde_json::Value) -> Option<(&'static str, String)> {
    if let Some(spec) = json.get("packageManager").and_then(|v| v.as_str()) {
        return Some(("packageManager", spec.to_string()));
    }
    // devEngines fields are one { name, version } or a list of them.
    let field = json.get("devEngines")?.get("packageManager")?;
    let entries = match field {
        serde_json::Value::Array(entries) => entries.iter().collect(),
        entry => vec![entry],
    };
    entries.into_iter().find_map(|entry| {
        let name = entry.get("name")?.as_str()?;
        let version = entry.get("version")?.as_str()?;
        Some(("devEngines.packageManager", format!("{name}@{version}")))
    })
}

/// The leading major of a spec's version (`^10.2` -> `10`), for the hint
/// that names crucible's `packageManager` argument.
fn major_of(wanted: &str) -> Option<&str> {
    let rest = wanted.trim_start_matches(|c: char| !c.is_ascii_digit());
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// `pnpm@12.21.0+sha512.<digest>` -> ("pnpm", "12.21.0"). The digest is a
/// corepack integrity suffix, not part of the version. A range (`pnpm@^10.2`,
/// devEngines' `>=9 <11`) is kept whole for version_matches.
fn parse_package_manager(spec: &str) -> Option<(String, String)> {
    let (name, version) = spec.rsplit_once('@')?;
    let version = version.split('+').next().unwrap_or(version).trim();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// The Node.js a package manager executable runs on, as far as its own file
/// says so without running it.
#[derive(Debug, PartialEq)]
enum ManagerNode {
    /// `#!/usr/bin/env node`: whatever node is first on PATH.
    FromPath,
    /// A script or wrapper that names this node (a nixpkgs or Homebrew
    /// wrapper, or a patched shebang).
    Fixed(PathBuf),
    /// A native executable (bun, pnpm's standalone build) that brings its own
    /// runtime, if any.
    Native,
    /// Anything else; doctor does not guess.
    Unknown,
}

/// Where a package manager gets its Node.js. Only what the files state
/// counts: a shebang, an absolute `.../bin/node` in a shell wrapper, or
/// makeWrapper's `.<name>-wrapped` target, followed a few levels. Larger
/// scripts are never scanned for paths, since a bundled pnpm.cjs or yarn.js
/// mentions node paths in strings that mean nothing about how it is run, and
/// a wrong warning here would send people chasing a working setup.
fn manager_node(exe: &Path) -> ManagerNode {
    // Wrappers are a few hundred bytes; anything bigger is the program.
    const WRAPPER_LIMIT: u64 = 64 * 1024;
    let mut exe = exe.to_path_buf();
    for _ in 0..4 {
        let Ok(bytes) = fs::read(&exe) else {
            return ManagerNode::Unknown;
        };
        if is_native(&bytes) {
            return ManagerNode::Native;
        }
        let Some(rest) = bytes.strip_prefix(b"#!") else {
            return ManagerNode::Unknown;
        };
        let text = String::from_utf8_lossy(rest);
        let shebang: Vec<&str> = text
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .collect();
        let interpreter = shebang.first().copied().unwrap_or_default();
        if interpreter.ends_with("/env") {
            // `env node` or `env -S node --flags`.
            return if shebang[1..].contains(&"node") {
                ManagerNode::FromPath
            } else {
                ManagerNode::Unknown
            };
        }
        if interpreter.ends_with("/bin/node") {
            return ManagerNode::Fixed(PathBuf::from(interpreter));
        }
        let shell = ["sh", "bash", "dash", "zsh"]
            .iter()
            .any(|shell| interpreter.ends_with(&format!("/{shell}")));
        if !shell || bytes.len() as u64 > WRAPPER_LIMIT {
            return ManagerNode::Unknown;
        }
        let paths: Vec<&str> = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || "/._+-@".contains(c)))
            .filter(|word| word.starts_with('/'))
            .collect();
        let mut nodes: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| path.ends_with("/bin/node"))
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        match nodes[..] {
            [node] => return ManagerNode::Fixed(PathBuf::from(node)),
            [] => {}
            _ => return ManagerNode::Unknown,
        }
        let wrapped = exe
            .file_name()
            .map(|name| format!("/.{}-wrapped", name.to_string_lossy()));
        match paths
            .iter()
            .find(|path| wrapped.as_deref().is_some_and(|w| path.ends_with(w)))
        {
            Some(next) => exe = PathBuf::from(next),
            None => return ManagerNode::Unknown,
        }
    }
    ManagerNode::Unknown
}

/// ELF or Mach-O (thin, fat, either byte order).
fn is_native(bytes: &[u8]) -> bool {
    const MAGIC: [[u8; 4]; 6] = [
        *b"\x7fELF",
        [0xfe, 0xed, 0xfa, 0xce],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
    ];
    bytes.len() >= 4 && MAGIC.iter().any(|magic| bytes[..4] == magic[..])
}

/// A package manager wrapped around another Node.js runs install scripts and
/// `pnpm exec` on that one, so native addons and engines checks see a
/// different release than the shell's node.
fn check_manager_node(checks: &mut Checks, name: &str, manager: &Path, node: &Path) {
    match manager_node(manager) {
        ManagerNode::FromPath => checks.ok(format!(
            "{} runs on the node on PATH ({})",
            manager.display(),
            node.display()
        )),
        ManagerNode::Fixed(its) => {
            // The toolchain's node is a symlink into the store, as is the one
            // a crucible wrapper names, so compare the files they resolve to.
            match (fs::canonicalize(&its), fs::canonicalize(node)) {
                (Ok(a), Ok(b)) if a == b => checks.ok(format!(
                    "{} runs on the node on PATH ({})",
                    manager.display(),
                    node.display()
                )),
                (Ok(_), Ok(_)) => checks.warn(format!(
                    "{} runs on {}, not the node on PATH ({}); use crucible's \
                     `packageManager = \"{name}@<major>\"` in `lib.<system>.node.toolchain`, \
                     which builds {name} on the toolchain's Node.js, instead of a separate {name}",
                    manager.display(),
                    its.display(),
                    node.display()
                )),
                _ => checks.info(format!(
                    "{} names {}, which does not exist; cannot tell which Node.js it runs on",
                    manager.display(),
                    its.display()
                )),
            }
        }
        ManagerNode::Native => checks.info(format!(
            "{} is a native executable with its own runtime; the node on PATH does not apply",
            manager.display()
        )),
        ManagerNode::Unknown => checks.info(format!(
            "cannot tell which Node.js {} runs on; not compared with the node on PATH",
            manager.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn parses_corepack_package_manager_specs() {
        assert_eq!(
            parse_package_manager("pnpm@12.21.0+sha512.abcdef"),
            Some(("pnpm".to_string(), "12.21.0".to_string()))
        );
        assert_eq!(
            parse_package_manager("yarn@4.18.1"),
            Some(("yarn".to_string(), "4.18.1".to_string()))
        );
        // Ranges stay whole, for version_matches.
        assert_eq!(
            parse_package_manager("pnpm@^12.21.0"),
            Some(("pnpm".to_string(), "^12.21.0".to_string()))
        );
        assert_eq!(
            parse_package_manager("pnpm@>=9 <11"),
            Some(("pnpm".to_string(), ">=9 <11".to_string()))
        );
        assert_eq!(parse_package_manager("pnpm"), None);
        assert_eq!(parse_package_manager("pnpm@"), None);
        assert_eq!(
            parse_package_manager("@scope@1.2.3"),
            Some(("@scope".to_string(), "1.2.3".to_string()))
        );
        assert_eq!(major_of("^10.2"), Some("10"));
        assert_eq!(major_of(">=9 <11"), Some("9"));
        assert_eq!(major_of("latest"), None);
    }

    #[test]
    fn falls_back_to_dev_engines_package_manager() {
        let spec = |text: &str| package_manager_spec(&serde_json::from_str(text).unwrap());
        assert_eq!(
            spec(
                r#"{"packageManager": "pnpm@10.4.1", "devEngines": {"packageManager": {"name": "yarn", "version": "4"}}}"#
            ),
            Some(("packageManager", "pnpm@10.4.1".to_string()))
        );
        assert_eq!(
            spec(r#"{"devEngines": {"packageManager": {"name": "pnpm", "version": "^10.2"}}}"#),
            Some(("devEngines.packageManager", "pnpm@^10.2".to_string()))
        );
        // The first entry with both a name and a version, as fromPackageJson.
        assert_eq!(
            spec(
                r#"{"devEngines": {"packageManager": [{"name": "bun"}, {"name": "pnpm", "version": ">=9 <11", "onFail": "warn"}, {"name": "yarn", "version": "4"}]}}"#
            ),
            Some(("devEngines.packageManager", "pnpm@>=9 <11".to_string()))
        );
        assert_eq!(
            spec(r#"{"devEngines": {"runtime": {"name": "node", "version": "24"}}}"#),
            None
        );
        assert_eq!(spec(r#"{"devEngines": {"packageManager": []}}"#), None);
        assert_eq!(spec("{}"), None);
    }

    #[test]
    fn reads_which_node_a_package_manager_runs_on() {
        let root = scratch("manager-node");
        let store_node = "/nix/store/00000000000000000000000000000000-nodejs-22.14.0/bin/node";

        let env_node = root.join("env/pnpm");
        write_executable(&env_node, b"#!/usr/bin/env node\nrequire('./pnpm.cjs')\n");
        assert_eq!(manager_node(&env_node), ManagerNode::FromPath);

        let shebang = root.join("shebang/yarn");
        write_executable(
            &shebang,
            format!("#!{store_node}\nconsole.log('/usr/bin/node')\n").as_bytes(),
        );
        assert_eq!(
            manager_node(&shebang),
            ManagerNode::Fixed(store_node.into())
        );

        // makeWrapper's script around nixpkgs' pnpm.
        let wrapper = root.join("wrapper/pnpm");
        write_executable(
            &wrapper,
            format!(
                "#! /nix/store/x-bash-5.2/bin/bash -e\nexec \"{store_node}\" --add-flags /nix/store/y-pnpm/libexec/pnpm.cjs \"$@\"\n"
            )
            .as_bytes(),
        );
        assert_eq!(
            manager_node(&wrapper),
            ManagerNode::Fixed(store_node.into())
        );

        // wrapProgram around a native binary: PATH gets a node, but the
        // program that runs is the native one.
        let native = root.join("native/bin");
        write_executable(&native.join(".pnpm-wrapped"), b"\x7fELF\x02\x01\x01");
        write_executable(
            &native.join("pnpm"),
            format!(
                "#!/bin/bash -e\nPATH=/nix/store/z-nodejs-22/bin${{PATH:+:}}$PATH\nexec -a \"$0\" \"{}\" \"$@\"\n",
                native.join(".pnpm-wrapped").display()
            )
            .as_bytes(),
        );
        assert_eq!(manager_node(&native.join("pnpm")), ManagerNode::Native);

        let macho = root.join("macho/bun");
        write_executable(&macho, &[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00]);
        assert_eq!(manager_node(&macho), ManagerNode::Native);

        // Two different nodes, or a big bundle, are not guessed at.
        let two = root.join("two/pnpm");
        write_executable(
            &two,
            b"#!/bin/sh\n[ -x /a/bin/node ] && exec /a/bin/node \"$@\"\nexec /b/bin/node \"$@\"\n",
        );
        assert_eq!(manager_node(&two), ManagerNode::Unknown);
        let python = root.join("python/pnpm");
        write_executable(&python, b"#!/usr/bin/python3\nprint('/usr/bin/node')\n");
        assert_eq!(manager_node(&python), ManagerNode::Unknown);
        let big = root.join("big/pnpm");
        let mut text = format!("#!/bin/sh\nexec {store_node} \"$@\"\n").into_bytes();
        text.resize(70 * 1024, b'#');
        write_executable(&big, &text);
        assert_eq!(manager_node(&big), ManagerNode::Unknown);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn checks_the_node_and_its_copies() {
        let root = scratch("node-checks");
        let (first, second, project) = (root.join("first"), root.join("second"), root.join("p"));
        write_executable(&first.join("node"), b"#!/bin/sh\necho v24.11.0\n");
        write_executable(&second.join("node"), b"#!/bin/sh\necho v22.14.0\n");
        fs::create_dir_all(&project).unwrap();

        let env = fake_env(&[&first, &second], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "is node v24.11.0");
        assert_line(&lines, "another node is on PATH: ");
        assert_line(&lines, "VP_HOME is not set");
        assert_eq!(errors, 0);

        // Nothing to say when neither a node nor a version file is here.
        fs::write(project.join(".node-version"), "24.11.0\n").unwrap();
        let env = fake_env(&[&root.join("empty")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "no node on PATH; Node.js checks skipped");
        assert_line(
            &lines,
            ".node-version pins 24.11.0, which is an alias or a range doctor cannot read",
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn checks_the_variables_that_make_vp_and_npm_stray() {
        let root = scratch("node-env");
        let env = fake_env(
            &[],
            &[
                ("VP_HOME", "/home/me/.local/share/vp"),
                ("VP_ENV_FORBIDDEN", "0"),
                ("NODE_PATH", "/opt/modules"),
                ("NPM_CONFIG_PREFIX", "/nix/store/xyz-nodejs/bin"),
            ],
        );
        let (lines, errors) = run_check(|checks| check_node_env(checks, &env));
        assert_line(&lines, "VP_HOME=/home/me/.local/share/vp;");
        assert_line(&lines, "VP_ENV_FORBIDDEN=0; crucible's vp needs 1");
        assert_line(&lines, "NODE_PATH=/opt/modules;");
        assert_line(
            &lines,
            "npm's global prefix is /nix/store/xyz-nodejs/bin, inside the read-only Nix store",
        );
        assert_eq!(errors, 0);

        let env = fake_env(
            &[],
            &[("VP_ENV_FORBIDDEN", "1"), ("npm_config_prefix", "/opt/npm")],
        );
        let (lines, _) = run_check(|checks| check_node_env(checks, &env));
        assert_line(&lines, "VP_ENV_FORBIDDEN=1");
        assert_line(&lines, "npm's global prefix is /opt/npm");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn compares_a_node_version_file_with_the_active_node() {
        let root = scratch("node-version-file");
        let (bin, project) = (root.join("bin"), root.join("p"));
        write_executable(&bin.join("node"), b"#!/bin/sh\necho v22.14.0\n");
        fs::create_dir_all(&project).unwrap();
        let env = fake_env(&[&bin], &[]);

        fs::write(project.join(".nvmrc"), "# the version\nv22.14.0\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, ".nvmrc pins v22.14.0, which matches node v22.14.0");
        assert_eq!(errors, 0);

        fs::write(project.join(".nvmrc"), "24\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, ".nvmrc pins 24, but node is v22.14.0");
        assert_eq!(errors, 1);

        fs::write(project.join(".nvmrc"), "lts/*\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            ".nvmrc pins lts/*, which is an alias or a range doctor cannot read",
        );
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn checks_the_package_manager_and_its_node() {
        // The same layout tests/default.nix uses: two pnpms, and the first
        // one wrapped around a node that is not the one on PATH.
        let root = scratch("node-package-manager");
        let (first, second, other, project) = (
            root.join("first"),
            root.join("second"),
            root.join("other/bin"),
            root.join("p"),
        );
        write_executable(&first.join("node"), b"#!/bin/sh\necho v24.11.0\n");
        write_executable(&other.join("node"), b"#!/bin/sh\necho v24.11.0\n");
        write_executable(
            &first.join("pnpm"),
            format!(
                "#!/bin/sh\n[ \"$1\" = --version ] && exec echo 10.4.1\nexec \"{}\" \"$@\"\n",
                other.join("node").display()
            )
            .as_bytes(),
        );
        write_executable(&second.join("pnpm"), b"#!/bin/sh\necho 9.15.0\n");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join("package.json"),
            r#"{"devEngines":{"packageManager":[{"name":"pnpm","version":">=10 <11"}]}}"#,
        )
        .unwrap();

        let env = fake_env(&[&first, &second], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "is pnpm 10.4.1, which matches pnpm@>=10 <11");
        assert_line(&lines, "another pnpm is on PATH: ");
        assert_line(&lines, "runs on ");
        assert_line(&lines, "/other/bin/node, not the node on PATH");
        assert_eq!(errors, 0);

        // A packageManager that is missing from PATH says what to add.
        fs::write(
            project.join("package.json"),
            r#"{"packageManager":"bun@1.2.3"}"#,
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "pins bun@1.2.3 in packageManager, but no bun is on PATH",
        );
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }
}
