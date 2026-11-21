//! Python and uv. The pins are the ones lib/python/toolchain.nix reads (the
//! nearest `.python-version`, `requires-python`), and the variables are the
//! ones its setup hook overwrites and the Home Manager module warns about:
//! anything that lets uv pick, or download, another Python than the
//! toolchain's, and a PYTHONHOME that sends python3 to another standard
//! library.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use super::{exact_version, find_nearest_file, first_version_line, Checks, Env, Op, Version};
use super::{shadowed, stdout_of};

const FIX: &str = "use `lib.<system>.python.fromPyprojectToml` (or `fromPythonVersion`) in the \
                   dev shell, or a `programs.crucible.python.package` on the release you need";

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let pythons = env.executables("python3");
    let uvs = env.executables("uv");
    let version_file = find_nearest_file(cwd, &[".python-version"]);
    let pyproject = find_nearest_file(cwd, &["pyproject.toml"]);
    if pythons.is_empty() && uvs.is_empty() && version_file.is_none() && pyproject.is_none() {
        return;
    }

    let python = match pythons.first() {
        Some(first) => {
            let version = stdout_of(checks, first, &["--version"], &[]).map(|out| {
                out.strip_prefix("Python ")
                    .unwrap_or(&out)
                    .trim()
                    .to_string()
            });
            if let Some(version) = &version {
                checks.ok(format!("{} is python3 {version}", first.display()));
            }
            for message in shadowed("python3", &without_system_copies(&pythons)) {
                checks.warn(message);
            }
            version
        }
        None => {
            checks.warn(format!("no python3 is on PATH; {FIX}"));
            None
        }
    };

    if let Some(first) = uvs.first() {
        if let Some(version) = stdout_of(checks, first, &["--version"], &[]) {
            checks.ok(format!("{} is {version}", first.display()));
        }
        for message in shadowed("uv", &uvs) {
            checks.warn(message);
        }
    }

    if let Some(path) = version_file {
        let found = pythons.first().map(PathBuf::as_path).zip(python.as_deref());
        check_version_file(checks, &path, found);
    }
    if let Some(path) = pyproject {
        check_requires_python(checks, &path, python.as_deref());
    }

    match env.var("PYTHONHOME") {
        Some(value) => checks.warn(format!(
            "PYTHONHOME={value}; python3 looks for its standard library there instead of next \
             to itself, and fails to start or runs another release's modules: unset it"
        )),
        None => checks.ok("PYTHONHOME is not set".to_string()),
    }

    if let Some(uv) = uvs.first() {
        check_uv_env(checks, env, uv, pythons.first().map(PathBuf::as_path));
    }
}

/// The OS's own python3 (/usr/bin on macOS and most distributions) further
/// down PATH cannot be removed and runs only when the toolchain's is gone, so
/// a warning about it would be noise on every machine.
fn without_system_copies(found: &[PathBuf]) -> Vec<PathBuf> {
    let system = |path: &PathBuf| {
        path.parent()
            .is_some_and(|dir| dir == Path::new("/usr/bin") || dir == Path::new("/bin"))
    };
    found
        .iter()
        .enumerate()
        .filter(|(i, path)| *i == 0 || !system(path))
        .map(|(_, path)| path.clone())
        .collect()
}

/// `.python-version` is what uv and pyenv read, and fromPythonVersion too.
/// `python` is the python3 on PATH and its version.
fn check_version_file(checks: &mut Checks, path: &Path, python: Option<(&Path, &str)>) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let Some(entry) = first_version_line(&text) else {
        checks.error(format!("{} names no version", path.display()));
        return;
    };
    let Some((request, freethreaded)) = python_request(&entry) else {
        checks.warn(format!(
            "{} asks for {entry}, but crucible ships CPython only, so fromPythonVersion \
             rejects it; use a version such as 3.14, or 3.14t for the freethreaded build",
            path.display()
        ));
        return;
    };
    // A missing python3 was already reported.
    let Some((exe, python)) = python else {
        return;
    };
    // Both kinds of build print the same --version, so ask the interpreter.
    match is_freethreaded(checks, exe) {
        Some(found) if found != freethreaded => {
            let (asked, found) = if freethreaded {
                ("the freethreaded build", "the regular (GIL) one")
            } else {
                ("the regular (GIL) build", "the freethreaded one")
            };
            checks.error(format!(
                "{} pins {entry}, {asked}, but python3 is {found}; {FIX}",
                path.display()
            ));
            return;
        }
        Some(_) => {}
        None => {
            checks.info(format!(
                "doctor cannot tell whether python3 is a freethreaded build; check that it is \
                 the one {} pins by hand",
                path.display()
            ));
            return;
        }
    }
    match pep440_matches(&request, python) {
        Some(true) => checks.ok(format!(
            "{} pins {entry}, which matches python3 {python}",
            path.display()
        )),
        Some(false) => checks.error(format!(
            "{} pins {entry}, but python3 is {python}; {FIX}",
            path.display()
        )),
        None => checks.info(format!(
            "{} pins {entry}, which doctor cannot compare with python3 {python}; check it by hand",
            path.display()
        )),
    }
}

/// uv refuses an interpreter that `requires-python` does not allow, even the
/// one UV_PYTHON names, so a mismatch stops every `uv run`.
fn check_requires_python(checks: &mut Checks, path: &Path, python: Option<&str>) {
    let toml: toml::Value = match fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| toml::from_str(&text).map_err(anyhow::Error::from))
    {
        Ok(toml) => toml,
        Err(err) => {
            checks.error(format!("cannot parse {}: {err}", path.display()));
            return;
        }
    };
    let Some(spec) = toml
        .get("project")
        .and_then(|project| project.get("requires-python"))
        .and_then(|spec| spec.as_str())
    else {
        checks.ok(format!("{} sets no requires-python", path.display()));
        return;
    };
    let Some(python) = python else {
        return;
    };
    match pep440_matches(spec, python) {
        Some(true) => checks.ok(format!(
            "{} requires-python {spec}, which python3 {python} satisfies",
            path.display()
        )),
        Some(false) => checks.error(format!(
            "{} requires-python {spec}, which python3 {python} does not satisfy, so uv refuses \
             it; {FIX}",
            path.display()
        )),
        None => checks.info(format!(
            "{} requires-python {spec}, which doctor cannot compare with python3 {python}; \
             check it by hand",
            path.display()
        )),
    }
}

/// What makes crucible's uv use another Python than the toolchain's. Its
/// wrapper sets UV_PYTHON, UV_NO_MANAGED_PYTHON=1 and
/// UV_PYTHON_DOWNLOADS=never as defaults, so a value from the login shell
/// wins outside a dev shell; inside one the setup hook overwrites them.
fn check_uv_env(checks: &mut Checks, env: &Env, uv: &Path, python: Option<&Path>) {
    let mut fine = true;

    let wrapper_python = wrapper_default(uv, "UV_PYTHON");
    match (env.var("UV_PYTHON"), wrapper_python) {
        (Some(value), _) if !value.contains('/') => {
            fine = false;
            checks.warn(format!(
                "UV_PYTHON={value} asks uv for an interpreter by version, not the python3 on \
                 PATH, and uv may download one; unset it (crucible's uv points it at its own \
                 python3)"
            ));
        }
        (Some(value), _) => {
            if !same_python(checks, &format!("UV_PYTHON={value}"), &value, python) {
                fine = false;
            }
        }
        (None, Some(default)) => {
            let what = format!("{} defaults UV_PYTHON to {default}", uv.display());
            if !same_python(checks, &what, &default, python) {
                fine = false;
            }
        }
        (None, None) => {
            fine = false;
            checks.info(format!(
                "{} sets no UV_PYTHON, so it is not crucible's uv: it picks a Python by itself \
                 and may download one; use the uv in crucible's Python toolchain",
                uv.display()
            ));
        }
    }

    if let Some(value) = env
        .var("UV_PYTHON_DOWNLOADS")
        .filter(|value| value != "never")
    {
        fine = false;
        checks.warn(format!(
            "UV_PYTHON_DOWNLOADS={value} lets uv download a Python instead of using the \
             toolchain's; unset it (crucible's uv sets never)"
        ));
    }
    if let Some(value) = env
        .var("UV_MANAGED_PYTHON")
        .filter(|value| !is_false(value))
    {
        fine = false;
        checks.warn(format!(
            "UV_MANAGED_PYTHON={value} contradicts crucible's UV_NO_MANAGED_PYTHON=1 and points \
             uv at Pythons it installed itself; unset it"
        ));
    }
    if let Some(value) = env.var("UV_PYTHON_PREFERENCE") {
        fine = false;
        checks.warn(format!(
            "UV_PYTHON_PREFERENCE={value} contradicts crucible's UV_NO_MANAGED_PYTHON=1 (the \
             toolchain's setup hook unsets it); unset it"
        ));
    }

    if fine {
        checks.ok("no UV_PYTHON* or UV_MANAGED_PYTHON setting works against uv".to_string());
    }
}

/// Whether `interpreter` (from `what`) is the python3 on PATH, reporting
/// either way. The toolchain's python3 is a symlink into the store, as is the
/// one its uv names, so the files they resolve to are compared.
fn same_python(checks: &mut Checks, what: &str, interpreter: &str, python: Option<&Path>) -> bool {
    let Some(python) = python else {
        return true;
    };
    match (fs::canonicalize(interpreter), fs::canonicalize(python)) {
        (Ok(a), Ok(b)) if a == b => {
            checks.ok(format!(
                "uv uses the python3 on PATH ({})",
                python.display()
            ));
            true
        }
        (Ok(_), Ok(_)) => {
            checks.warn(format!(
                "{what}, not the python3 on PATH ({}), so uv runs projects on another \
                 interpreter; use python3 and uv from one crucible toolchain, and open a new \
                 shell if an older one set UV_PYTHON",
                python.display()
            ));
            false
        }
        _ => {
            checks.warn(format!("{what}, which does not exist; uv cannot use it"));
            false
        }
    }
}

/// uv's own reading of a false boolean variable.
fn is_false(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// The default a makeWrapper script gives `var` with `--set-default`
/// (`export VAR=${VAR-'value'}`), which is how crucible's uv carries its
/// Python. A binary or anything larger than a wrapper is not read.
fn wrapper_default(exe: &Path, var: &str) -> Option<String> {
    const WRAPPER_LIMIT: u64 = 64 * 1024;
    if fs::metadata(exe).ok()?.len() > WRAPPER_LIMIT {
        return None;
    }
    let text = fs::read_to_string(exe).ok()?;
    if !text.starts_with("#!") {
        return None;
    }
    let marker = format!("{var}=${{{var}-");
    let rest = &text[text.find(&marker)? + marker.len()..];
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let rest = &rest[1..];
    Some(rest[..rest.find(quote)?].to_string())
}

/// The request in a `.python-version` entry, as readPythonVersion in
/// lib/python/toolchain.nix reads it: `3.14`, `3.14.2`, `cpython@3.14`,
/// `cpython-3.14` or a specifier, and whether a trailing `t` (`3.14t`) asks
/// for the freethreaded build. None for what crucible does not ship
/// (`pypy@3.10`).
fn python_request(entry: &str) -> Option<(String, bool)> {
    let lower = entry.to_lowercase();
    let version = ["cpython-", "cpython@", "python"]
        .iter()
        .find_map(|prefix| lower.strip_prefix(prefix))
        .unwrap_or(&lower);
    let (version, freethreaded) = match version.strip_suffix('t') {
        Some(version) => (version, true),
        None => (version, false),
    };
    let plain = {
        let parts: Vec<&str> = version.split('.').collect();
        parts.len() <= 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    };
    if plain {
        Some((version.to_string(), freethreaded))
    } else if !freethreaded && entry.starts_with(['<', '>', '=', '!', '~']) {
        Some((entry.to_string(), false))
    } else {
        None
    }
}

/// Whether `python` is a freethreaded build, from the Py_GIL_DISABLED its
/// sysconfig records; None when it cannot say.
fn is_freethreaded(checks: &mut Checks, python: &Path) -> Option<bool> {
    let code = "import sysconfig; print(sysconfig.get_config_var('Py_GIL_DISABLED') or 0)";
    match stdout_of(checks, python, &["-c", code], &[])?.as_str() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// Whether `actual` is a release PEP 440 specifier `spec` allows, with the
/// clauses lib/python/pep440.nix reads: comma-separated `==`, `!=`, `~=`,
/// `>=`, `<=`, `>`, `<`, `===`, the `.*` suffix, and a bare version meaning
/// that prefix ("3.14" is any 3.14.x). None when doctor cannot decide: a
/// pre-release or local segment, more than three parts, or no clause at all.
fn pep440_matches(spec: &str, actual: &str) -> Option<bool> {
    let actual = exact_version(actual)?;
    let mut clauses = 0;
    let mut all = true;
    for clause in spec.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        clauses += 1;
        let split = clause
            .find(|c: char| !"=!~<>".contains(c))
            .unwrap_or(clause.len());
        let (op, rest) = clause.split_at(split);
        let rest = rest.trim().trim_start_matches('v');
        let (rest, wildcard) = match rest.strip_suffix(".*") {
            Some(rest) => (rest, true),
            None => (rest, false),
        };
        let release = release_parts(rest)?;
        let ordering = actual.cmp(&padded(&release));
        let holds = |op: Op| op.holds(ordering);
        all &= match (op, wildcard) {
            ("", false) | ("==", true) => has_prefix(&release, actual),
            ("!=", true) => !has_prefix(&release, actual),
            ("==" | "===", false) => ordering == Ordering::Equal,
            ("!=", false) => ordering != Ordering::Equal,
            ("~=", false) if release.len() >= 2 => {
                holds(Op::Ge) && has_prefix(&release[..release.len() - 1], actual)
            }
            (">=", false) => holds(Op::Ge),
            ("<=", false) => holds(Op::Le),
            (">", false) => holds(Op::Gt),
            ("<", false) => holds(Op::Lt),
            _ => return None,
        };
    }
    (clauses > 0).then_some(all)
}

/// `3.14` -> [3, 14]: one to three numeric parts, nothing else.
fn release_parts(text: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = text
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        })
        .collect::<Option<_>>()?;
    (1..=3).contains(&parts.len()).then_some(parts)
}

/// PEP 440 pads the shorter release with zeros: 3.14 == 3.14.0.
fn padded(release: &[u64]) -> Version {
    let mut version = [0; 3];
    version[..release.len()].copy_from_slice(release);
    version
}

fn has_prefix(release: &[u64], actual: Version) -> bool {
    release.iter().zip(actual).all(|(a, b)| *a == b)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn reads_pep_440_specifiers() {
        assert_eq!(pep440_matches("3.14", "3.14.2"), Some(true));
        assert_eq!(pep440_matches("3", "3.14.2"), Some(true));
        assert_eq!(pep440_matches("3.14", "3.13.9"), Some(false));
        assert_eq!(pep440_matches("3.14.2", "3.14.3"), Some(false));
        assert_eq!(pep440_matches(">=3.12,<3.15", "3.14.2"), Some(true));
        assert_eq!(pep440_matches(">=3.12, <3.14", "3.14.0"), Some(false));
        assert_eq!(pep440_matches(">= 3.12", "3.12.0"), Some(true));
        assert_eq!(pep440_matches("==3.14.*", "3.14.9"), Some(true));
        assert_eq!(pep440_matches("!=3.13.*", "3.13.1"), Some(false));
        assert_eq!(pep440_matches("==3.14", "3.14.0"), Some(true));
        assert_eq!(pep440_matches("==3.14", "3.14.1"), Some(false));
        assert_eq!(pep440_matches("!=3.14.1", "3.14.2"), Some(true));
        assert_eq!(pep440_matches("~=3.13.2", "3.13.9"), Some(true));
        assert_eq!(pep440_matches("~=3.13.2", "3.14.0"), Some(false));
        assert_eq!(pep440_matches("~=3.12", "3.14.0"), Some(true));
        assert_eq!(pep440_matches(">3.13", "3.13.1"), Some(true));
        assert_eq!(pep440_matches("<=3.13", "3.13.1"), Some(false));
        // What doctor cannot decide is None, not a mismatch.
        assert_eq!(pep440_matches(">=3.14.0a1", "3.14.2"), None);
        assert_eq!(pep440_matches(">=3.12", "3.15.0a2"), None);
        assert_eq!(pep440_matches("~=3", "3.14.0"), None);
        assert_eq!(pep440_matches(">=3.*", "3.14.0"), None);
        assert_eq!(pep440_matches("", "3.14.0"), None);
    }

    #[test]
    fn reads_python_version_entries() {
        let gil = |version: &str| Some((version.to_string(), false));
        assert_eq!(python_request("3.14"), gil("3.14"));
        assert_eq!(python_request("cpython@3.14.2"), gil("3.14.2"));
        assert_eq!(python_request("cpython-3.13"), gil("3.13"));
        assert_eq!(python_request("CPython-3.13"), gil("3.13"));
        assert_eq!(python_request("python3.12"), gil("3.12"));
        assert_eq!(python_request(">=3.12,<3.15"), gil(">=3.12,<3.15"));
        assert_eq!(python_request("3.14t"), Some(("3.14".to_string(), true)));
        assert_eq!(
            python_request("cpython@3.14.10t"),
            Some(("3.14.10".to_string(), true))
        );
        assert_eq!(python_request("pypy@3.10"), None);
        assert_eq!(python_request("3.t"), None);
    }

    #[test]
    fn reads_the_python_a_uv_wrapper_sets() {
        let root = scratch("uv-wrapper");
        let wrapper = root.join("uv");
        write_executable(
            &wrapper,
            b"#! /nix/store/x-bash/bin/bash -e\n\
              export UV_PYTHON=${UV_PYTHON-'/nix/store/y-python-3.14.2/bin/python3'}\n\
              export UV_NO_MANAGED_PYTHON=${UV_NO_MANAGED_PYTHON-'1'}\n\
              export UV_PYTHON_DOWNLOADS=${UV_PYTHON_DOWNLOADS-'never'}\n\
              exec -a \"$0\" \"/nix/store/z-uv-unwrapped/bin/uv\"  \"$@\"\n",
        );
        assert_eq!(
            wrapper_default(&wrapper, "UV_PYTHON").as_deref(),
            Some("/nix/store/y-python-3.14.2/bin/python3")
        );
        assert_eq!(
            wrapper_default(&wrapper, "UV_PYTHON_DOWNLOADS").as_deref(),
            Some("never")
        );
        let native = root.join("native-uv");
        write_executable(&native, b"\x7fELF\x02\x01\x01");
        assert_eq!(wrapper_default(&native, "UV_PYTHON"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stays_quiet_without_python() {
        let root = scratch("python-quiet");
        let env = fake_env(&[&root.join("bin")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &root, &env));
        assert!(lines.is_empty(), "{lines:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn compares_python_with_the_project_and_uv() {
        let root = scratch("python");
        let (first, second, project) = (root.join("first"), root.join("second"), root.join("p"));
        // `-c` is doctor asking whether the build is freethreaded.
        write_executable(
            &first.join("python3"),
            b"#!/bin/sh\n[ \"$1\" = -c ] && exec echo 0\necho Python 3.13.1\n",
        );
        write_executable(&second.join("python3"), b"#!/bin/sh\necho Python 3.14.2\n");
        // A uv wrapper from another toolchain than the python3 on PATH.
        write_executable(
            &first.join("uv"),
            format!(
                "#!/bin/sh\nexport UV_PYTHON=${{UV_PYTHON-'{}'}}\n[ \"$1\" = --version ] && echo 'uv 0.9.5'\n",
                second.join("python3").display()
            )
            .as_bytes(),
        );
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join(".python-version"), "# pinned\ncpython@3.14\n").unwrap();
        fs::write(
            project.join("pyproject.toml"),
            "[project]\nname = \"app\"\nrequires-python = \">=3.12\"\n",
        )
        .unwrap();

        let env = fake_env(
            &[&first, &second],
            &[
                ("PYTHONHOME", "/opt/python"),
                ("UV_PYTHON_DOWNLOADS", "automatic"),
                ("UV_MANAGED_PYTHON", "1"),
                ("UV_PYTHON_PREFERENCE", "managed"),
            ],
        );
        let (lines, errors) = run_check(|checks| check(checks, &project.join("src"), &env));
        assert_line(&lines, "python3 is python3 3.13.1");
        assert_line(&lines, "is uv 0.9.5");
        assert_line(&lines, "another python3 is on PATH: ");
        assert_line(
            &lines,
            ".python-version pins cpython@3.14, but python3 is 3.13.1",
        );
        assert_line(
            &lines,
            "requires-python >=3.12, which python3 3.13.1 satisfies",
        );
        assert_line(&lines, "warn  PYTHONHOME=/opt/python");
        assert_line(&lines, "defaults UV_PYTHON to ");
        assert_line(&lines, "UV_PYTHON_DOWNLOADS=automatic lets uv download");
        assert_line(&lines, "UV_MANAGED_PYTHON=1 contradicts");
        assert_line(&lines, "UV_PYTHON_PREFERENCE=managed contradicts");
        assert_eq!(errors, 1);

        // The same python3 through UV_PYTHON, and a version file it matches.
        fs::write(project.join(".python-version"), "3.13\n").unwrap();
        let python = first.join("python3").display().to_string();
        let env = fake_env(
            &[&first],
            &[("UV_PYTHON", &python), ("UV_MANAGED_PYTHON", "0")],
        );
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            ".python-version pins 3.13, which matches python3 3.13.1",
        );
        assert_line(&lines, "uv uses the python3 on PATH");
        assert_line(&lines, "ok    no UV_PYTHON* or UV_MANAGED_PYTHON setting");
        assert!(
            !lines.iter().any(|line| line.starts_with("warn")),
            "{lines:?}"
        );
        assert_eq!(errors, 0);

        // A freethreaded pin needs the freethreaded build, at the same versions.
        fs::write(project.join(".python-version"), "3.13t\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            ".python-version pins 3.13t, the freethreaded build, but python3 is the regular (GIL) one",
        );
        assert_eq!(errors, 1);
        let free = root.join("free");
        write_executable(
            &free.join("python3"),
            b"#!/bin/sh\n[ \"$1\" = -c ] && exec echo 1\necho Python 3.13.1\n",
        );
        let (lines, errors) = run_check(|checks| check(checks, &project, &fake_env(&[&free], &[])));
        assert_line(
            &lines,
            ".python-version pins 3.13t, which matches python3 3.13.1",
        );
        assert_eq!(errors, 0);

        // A project without python3 says what to add.
        let env = fake_env(&[&root.join("none")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "warn  no python3 is on PATH; use `lib.<system>.python.fromPyprojectToml`",
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn leaves_the_system_python3_out_of_shadowing() {
        let found = [
            PathBuf::from("/nix/store/x/bin/python3"),
            PathBuf::from("/opt/homebrew/bin/python3"),
            PathBuf::from("/usr/bin/python3"),
        ];
        assert_eq!(without_system_copies(&found), found[..2]);
        // The system copy is still the one in use when it comes first.
        assert_eq!(without_system_copies(&found[2..]), found[2..]);
    }
}
