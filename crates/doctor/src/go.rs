//! Go. The pins are go.mod's (or go.work's) `toolchain` and `go`
//! directives, read as minimums the way lib/go/toolchain.nix and the go
//! command read them, and the settings are the ones the setup hook overwrites
//! and the Home Manager module warns about: a GOTOOLCHAIN that lets go
//! download another toolchain, and a GOROOT that pairs this go with another
//! standard library.
//!
//! Every go command runs with GOTOOLCHAIN=local: under `auto`, even `go
//! version` in a module that asks for a newer Go would download one, and
//! doctor stays offline.

use std::fs;
use std::path::{Path, PathBuf};

use super::{at_least, find_nearest_file, shadowed, stdout_of, Checks, Env};

const LOCAL: [(&str, &str); 1] = [("GOTOOLCHAIN", "local")];

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let gos = env.executables("go");
    let module = module_file(cwd, env);
    if gos.is_empty() && module.is_none() {
        return;
    }

    match env.var("GOROOT") {
        Some(value) => checks.warn(format!(
            "GOROOT={value}; go then uses the standard library and tools under it instead of its \
             own, and finds its GOROOT by itself: unset it"
        )),
        None => checks.ok("GOROOT is not set".to_string()),
    }

    let go = gos.first().and_then(|first| {
        let output = stdout_of(checks, first, &["version"], &LOCAL)?;
        let version = go_release(output.split_whitespace().nth(2).unwrap_or_default());
        checks.ok(format!("{} is go {version}", first.display()));
        Some((first.clone(), version))
    });
    for message in shadowed("go", &gos) {
        checks.warn(message);
    }
    if gos.is_empty() {
        checks.warn(
            "no go is on PATH; use `lib.<system>.go.fromGoMod` in the dev shell, or enable \
             `programs.crucible.go`"
                .to_string(),
        );
    }

    // One line per variable, in the order asked for; an unset one is empty.
    let output = go.as_ref().and_then(|(exe, _)| {
        stdout_of(
            checks,
            exe,
            &["env", "GOENV", "GOROOT", "GOPATH", "GOBIN"],
            &LOCAL,
        )
    });
    let lines: Vec<&str> = output
        .as_deref()
        .map_or_else(Vec::new, |out| out.lines().collect());
    let value = |i: usize| lines.get(i).map(|s| s.trim()).filter(|s| !s.is_empty());
    check_gotoolchain(checks, env, value(0), value(1));
    if go.is_some() {
        check_gobin(checks, env, value(2), value(3));
    }

    if let (Some(path), Some((_, version))) = (module, &go) {
        check_module(checks, &path, version);
    }
}

/// The file whose directives pick the toolchain: the nearest go.work, as the
/// go command finds it (unless GOWORK=off), else the nearest go.mod.
fn module_file(cwd: &Path, env: &Env) -> Option<PathBuf> {
    let work = match env.var("GOWORK").as_deref() {
        Some("off") => None,
        Some(file) => Some(PathBuf::from(file)).filter(|path| path.is_file()),
        None => find_nearest_file(cwd, &["go.work"]),
    };
    work.or_else(|| find_nearest_file(cwd, &["go.mod"]))
}

/// `go1.27.4` -> `1.27.4`, and the `go1.20` of releases before 1.21 ->
/// `1.20.0`, so version comparisons see three parts.
fn go_release(word: &str) -> String {
    let version = word.strip_prefix("go").unwrap_or(word);
    let numeric = version.chars().all(|c| c.is_ascii_digit() || c == '.');
    if numeric && version.matches('.').count() == 1 {
        format!("{version}.0")
    } else {
        version.to_string()
    }
}

/// GOTOOLCHAIN as go resolves it: the environment, then the `go env -w` file,
/// then the GOROOT's go.env (which says `local` in crucible's go). Anything
/// but `local` lets go fetch or switch to the toolchain a module asks for.
fn check_gotoolchain(checks: &mut Checks, env: &Env, goenv: Option<&str>, goroot: Option<&str>) {
    let from_file = |file: PathBuf| {
        let value = env_file_value(&fs::read_to_string(&file).ok()?, "GOTOOLCHAIN")?;
        Some((value, file))
    };
    let setting = env
        .var("GOTOOLCHAIN")
        .map(|value| (value, None))
        .or_else(|| {
            goenv
                .filter(|file| *file != "off")
                .and_then(|file| from_file(PathBuf::from(file)))
                .map(|(value, file)| (value, Some((file, true))))
        })
        .or_else(|| {
            goroot
                .and_then(|root| from_file(Path::new(root).join("go.env")))
                .map(|(value, file)| (value, Some((file, false))))
        });
    match setting {
        Some((value, source)) if value == "local" => checks.ok(format!(
            "GOTOOLCHAIN=local{}",
            source.map_or_else(String::new, |(file, _)| format!(
                " (from {})",
                file.display()
            ))
        )),
        Some((value, None)) => checks.warn(format!(
            "GOTOOLCHAIN={value} lets go download and run the toolchain a module asks for \
             instead of this one; unset it or set it to local, as crucible's go.env does"
        )),
        Some((value, Some((file, true)))) => checks.warn(format!(
            "{} sets GOTOOLCHAIN={value} (`go env -w`), which lets go download and run the \
             toolchain a module asks for instead of this one; run `go env -u GOTOOLCHAIN`",
            file.display()
        )),
        Some((value, Some((file, false)))) => checks.warn(format!(
            "{} sets GOTOOLCHAIN={value}, so this go is not crucible's and downloads the \
             toolchain a module asks for; use `lib.<system>.go.toolchain` or \
             `programs.crucible.go`",
            file.display()
        )),
        // No go to ask and no variable: nothing to judge.
        None => {}
    }
}

/// The last `key=value` line for `key` in a go env file.
fn env_file_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
        .next_back()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `go install` puts commands in GOBIN, or else the first GOPATH's bin; off
/// PATH, they install fine and then cannot be found.
fn check_gobin(checks: &mut Checks, env: &Env, gopath: Option<&str>, gobin: Option<&str>) {
    let dir = match (gobin, gopath) {
        (Some(gobin), _) => PathBuf::from(gobin),
        (None, Some(gopath)) => {
            let first = std::env::split_paths(gopath).next().unwrap_or_default();
            first.join("bin")
        }
        (None, None) => return,
    };
    if env.on_path(&dir) {
        checks.ok(format!(
            "{} (where `go install` puts commands) is on PATH",
            dir.display()
        ));
    } else {
        checks.warn(format!(
            "`go install` puts commands in {}, which is not on PATH; add it \
             (`programs.crucible.go.goPath` does)",
            dir.display()
        ));
    }
}

/// The `go` directive is a hard minimum: with GOTOOLCHAIN=local an older go
/// refuses the module. The `toolchain` directive is only a preference to the
/// go command, but fromGoMod reads it first, so an older go is not what the
/// project's flake would give.
fn check_module(checks: &mut Checks, path: &Path, actual: &str) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let fix = "use `lib.<system>.go.fromGoMod`, or a `programs.crucible.go.package` on a newer \
               release";

    if let Some(value) = directive(&text, "toolchain").filter(|value| value != "default") {
        let wanted = go_release(&value);
        match at_least(&wanted, actual) {
            Some(true) => checks.ok(format!(
                "{} asks for toolchain {value}, and go {actual} is as new",
                path.display()
            )),
            Some(false) => checks.warn(format!(
                "{} asks for toolchain {value}, but go is {actual}; go builds it anyway with \
                 GOTOOLCHAIN=local if the go directive allows, but fromGoMod would give \
                 {wanted}: {fix}",
                path.display()
            )),
            None => checks.info(format!(
                "{} asks for toolchain {value}, which doctor cannot compare with go {actual}",
                path.display()
            )),
        }
    }

    let Some(value) = directive(&text, "go") else {
        checks.warn(format!(
            "{} has no go directive; add one with `go mod edit -go=<version>`, which fromGoMod \
             needs",
            path.display()
        ));
        return;
    };
    match at_least(&go_release(&value), actual) {
        Some(true) => checks.ok(format!(
            "{} needs go {value} or newer, and go is {actual}",
            path.display()
        )),
        Some(false) => checks.error(format!(
            "{} needs go {value} or newer, but go is {actual}, which refuses the module with \
             GOTOOLCHAIN=local; {fix}",
            path.display()
        )),
        None => checks.info(format!(
            "{} needs go {value}, which doctor cannot compare with go {actual}",
            path.display()
        )),
    }
}

/// The value of a one-line directive, as fromGoMod reads it: `//` comments
/// dropped, then `name` and a single word.
fn directive(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split("//").next().unwrap_or_default().trim();
        let rest = line.strip_prefix(name)?;
        if !rest.starts_with([' ', '\t']) {
            return None;
        }
        let mut words = rest.split_whitespace();
        let value = words.next()?;
        words.next().is_none().then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn reads_go_mod_directives() {
        let text = "module example.com/m // the module\n\ngo 1.26.1\ntoolchain go1.27.2 // newer\ngodebug default=go1.21\n";
        assert_eq!(directive(text, "go").as_deref(), Some("1.26.1"));
        assert_eq!(directive(text, "toolchain").as_deref(), Some("go1.27.2"));
        assert_eq!(directive("module m\n", "go"), None);
        assert_eq!(directive("godebug x=1\n", "go"), None);
        assert_eq!(go_release("go1.27.4"), "1.27.4");
        assert_eq!(go_release("go1.20"), "1.20.0");
        assert_eq!(go_release("1.27rc1"), "1.27rc1");
        assert_eq!(
            env_file_value(
                "# comment\nGOTOOLCHAIN=auto\nGOPROXY=off\nGOTOOLCHAIN=local\n",
                "GOTOOLCHAIN"
            )
            .as_deref(),
            Some("local")
        );
        assert_eq!(env_file_value("GOTOOLCHAINX=auto\n", "GOTOOLCHAIN"), None);
    }

    #[test]
    fn stays_quiet_without_go() {
        let root = scratch("go-quiet");
        let env = fake_env(&[&root.join("bin")], &[("GOTOOLCHAIN", "auto")]);
        let (lines, _) = run_check(|checks| check(checks, &root, &env));
        assert!(lines.is_empty(), "{lines:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    /// A go that reports `version` and the four `go env` values doctor asks
    /// for, and fails if GOTOOLCHAIN=local is missing, as a download would.
    fn fake_go(bin: &Path, version: &str, goenv: &Path, goroot: &Path, gopath: &Path) {
        write_executable(
            &bin.join("go"),
            format!(
                "#!/bin/sh\n[ \"$GOTOOLCHAIN\" = local ] || exit 1\ncase \"$1\" in\n\
                 version) echo 'go version go{version} darwin/arm64' ;;\n\
                 env) printf '%s\\n' '{}' '{}' '{}' '' ;;\nesac\n",
                goenv.display(),
                goroot.display(),
                gopath.display()
            )
            .as_bytes(),
        );
    }

    #[test]
    #[cfg(unix)]
    fn compares_go_with_go_mod_and_its_settings() {
        let root = scratch("go");
        let (bin, other, goroot, gopath) = (
            root.join("bin"),
            root.join("other"),
            root.join("goroot"),
            root.join("gopath"),
        );
        fs::create_dir_all(&goroot).unwrap();
        fs::write(goroot.join("go.env"), "GOTOOLCHAIN=local\n").unwrap();
        fs::write(root.join("goenv"), "GOTOOLCHAIN=auto\n").unwrap();
        fake_go(&bin, "1.26.3", &root.join("goenv"), &goroot, &gopath);
        fake_go(&other, "1.27.0", &root.join("goenv"), &goroot, &gopath);
        let project = root.join("project");
        fs::create_dir_all(project.join("cmd")).unwrap();
        fs::write(
            project.join("go.mod"),
            "module example.com/m\n\ngo 1.26.1\n\ntoolchain go1.27.2\n",
        )
        .unwrap();

        let env = fake_env(&[&bin, &other], &[("GOROOT", "/opt/go")]);
        let (lines, errors) = run_check(|checks| check(checks, &project.join("cmd"), &env));
        assert_line(&lines, "warn  GOROOT=/opt/go");
        assert_line(&lines, "is go 1.26.3");
        assert_line(&lines, "another go is on PATH: ");
        assert_line(&lines, "goenv sets GOTOOLCHAIN=auto (`go env -w`)");
        assert_line(&lines, "gopath/bin, which is not on PATH");
        assert_line(&lines, "asks for toolchain go1.27.2, but go is 1.26.3");
        assert_line(&lines, "needs go 1.26.1 or newer, and go is 1.26.3");
        assert_eq!(errors, 0);

        // An old go errors; GOTOOLCHAIN from the environment wins over the
        // file, and the GOROOT's go.env is the last word.
        fs::write(project.join("go.mod"), "module example.com/m\n\ngo 1.27\n").unwrap();
        fs::create_dir_all(gopath.join("bin")).unwrap();
        let env = fake_env(
            &[&bin, &gopath.join("bin")],
            &[("GOTOOLCHAIN", "go1.27.0+auto")],
        );
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "GOTOOLCHAIN=go1.27.0+auto lets go download");
        assert_line(
            &lines,
            "gopath/bin (where `go install` puts commands) is on PATH",
        );
        assert_line(&lines, "needs go 1.27 or newer, but go is 1.26.3");
        assert_eq!(errors, 1);

        fs::remove_file(root.join("goenv")).unwrap();
        let env = fake_env(&[&bin], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "ok    GOTOOLCHAIN=local (from ");

        fs::write(goroot.join("go.env"), "GOTOOLCHAIN=auto\n").unwrap();
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "go.env sets GOTOOLCHAIN=auto, so this go is not crucible's",
        );

        // A module without go says what to add.
        let env = fake_env(&[&root.join("none")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "warn  no go is on PATH; use `lib.<system>.go.fromGoMod`",
        );

        fs::remove_dir_all(&root).unwrap();
    }
}
