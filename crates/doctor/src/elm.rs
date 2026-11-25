//! Elm. elm.json's `elm-version` names the compiler the project was written
//! against, the way lib/elm reads it. An older compiler cannot build the
//! project; a newer one may have changed the language, so doctor only warns.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::{exact_version, find_nearest_file, shadowed, stdout_of, Checks, Env};

const FIX: &str = "use `lib.<system>.elm.fromElmJson` in the dev shell";

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let elms = env.executables("elm");
    let config = find_nearest_file(cwd, &["elm.json"]);
    if elms.is_empty() && config.is_none() {
        return;
    }

    let elm = match elms.first() {
        Some(first) => {
            // `elm --version` prints a plain release such as `0.19.1`.
            // npm's tarball version carries a `-0` suffix that a wrapper can
            // pass through; it names the same compiler release.
            let version = stdout_of(checks, first, &["--version"], &[]);
            if let Some(version) = &version {
                checks.ok(format!("{} is elm {version}", first.display()));
            }
            for message in shadowed("elm", &elms) {
                checks.warn(message);
            }
            version
        }
        None => {
            if let Some(config) = &config {
                checks.warn(format!(
                    "{} is here, but no elm is on PATH; add `lib.<system>.elm.fromElmJson` to the \
                     dev shell",
                    config.display()
                ));
            }
            None
        }
    };

    if let (Some(path), Some(elm)) = (config, elm) {
        check_elm_json(checks, &path, &elm);
    }
}

/// elm.json's `elm-version` is an exact release, not a range: fromElmJson
/// resolves that one version from the pins.
fn check_elm_json(checks: &mut Checks, path: &Path, actual: &str) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let json: Value = match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(err) => {
            checks.error(format!("cannot parse {}: {err}", path.display()));
            return;
        }
    };
    let Some(wanted) = json.get("elm-version").and_then(Value::as_str) else {
        checks.warn(format!(
            "{} has no elm-version, so doctor cannot compare elm {actual} with it",
            path.display()
        ));
        return;
    };

    let actual = actual.strip_suffix("-0").unwrap_or(actual);
    let pin = format!("{} pins elm-version {wanted}", path.display());
    match (exact_version(wanted), exact_version(actual)) {
        (Some(wanted), Some(release)) if release < wanted => checks.error(format!(
            "{pin}, but elm is {actual}, which is older and cannot build it; {FIX}"
        )),
        (Some(wanted), Some(release)) if release > wanted => checks.warn(format!(
            "{pin}, but elm is {actual}, which is newer and may have changed the language; {FIX}"
        )),
        (Some(_), Some(_)) => checks.ok(format!("{pin}, and elm {actual} matches")),
        _ => checks.info(format!(
            "{pin}, which doctor cannot compare with elm {actual}; check it by hand"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    #[cfg(unix)]
    fn compares_elm_with_elm_json() {
        let root = scratch("elm");
        let (first, second, project) = (root.join("first"), root.join("second"), root.join("p"));
        write_executable(&first.join("elm"), b"#!/bin/sh\necho 0.19.1\n");
        write_executable(&second.join("elm"), b"#!/bin/sh\necho 0.19.0\n");
        fs::create_dir_all(project.join("src")).unwrap();
        let elm_json = |version: &str| {
            let text = format!(
                "{{\"elm-version\": \"{version}\", \"source-directories\": [\"src\"], \"dependencies\": {{}}}}"
            );
            fs::write(project.join("elm.json"), text).unwrap();
        };

        elm_json("0.19.1");
        let env = fake_env(&[&first, &second], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project.join("src"), &env));
        assert_line(&lines, "is elm 0.19.1");
        assert_line(&lines, "another elm is on PATH: ");
        assert_line(&lines, "pins elm-version 0.19.1, and elm 0.19.1 matches");
        assert_eq!(errors, 0);

        // Older is an error, newer is only a warning.
        elm_json("0.19.3");
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "but elm is 0.19.1, which is older");
        assert_eq!(errors, 1);

        elm_json("0.19.0");
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "but elm is 0.19.1, which is newer");
        assert_eq!(errors, 0);

        // npm's `-0` tarball suffix names the same release.
        write_executable(&first.join("elm"), b"#!/bin/sh\necho 0.19.1-0\n");
        elm_json("0.19.1");
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "and elm 0.19.1 matches");
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn warns_when_elm_is_missing_but_elm_json_is_here() {
        let root = scratch("elm-missing");
        let project = root.join("p");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("elm.json"), "{\"elm-version\":\"0.19.1\"}\n").unwrap();
        let env = fake_env(&[&root.join("none")], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "elm.json is here, but no elm is on PATH");
        assert_line(&lines, "lib.<system>.elm.fromElmJson");
        assert_eq!(errors, 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn ignores_projects_without_elm_json() {
        let root = scratch("elm-quiet");
        let env = fake_env(&[&root.join("bin")], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &root, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn says_when_elm_json_has_no_elm_version() {
        let root = scratch("elm-no-version");
        let (bin, project) = (root.join("bin"), root.join("p"));
        write_executable(&bin.join("elm"), b"#!/bin/sh\necho 0.19.1\n");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("elm.json"), "{}\n").unwrap();
        let env = fake_env(&[&bin], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "has no elm-version");
        assert_eq!(errors, 0);
        fs::remove_dir_all(&root).unwrap();
    }
}
