//! Zig. The pin is build.zig.zon's `.minimum_zig_version`, the one
//! fromBuildZigZon in lib/zig/toolchain.nix reads.

use std::fs;
use std::path::Path;

use super::{at_least, exact_version, find_nearest_file, shadowed, stdout_of, Checks, Env};

const FIX: &str =
    "use `lib.<system>.zig.fromBuildZigZon`, or a `programs.crucible.zig.package` on that line";

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let zigs = env.executables("zig");
    let zon = find_nearest_file(cwd, &["build.zig.zon"]);
    if zigs.is_empty() && zon.is_none() {
        return;
    }

    let zig = match zigs.first() {
        Some(first) => {
            let version = stdout_of(checks, first, &["version"], &[]);
            if let Some(version) = &version {
                checks.ok(format!("{} is zig {version}", first.display()));
            }
            for message in shadowed("zig", &zigs) {
                checks.warn(message);
            }
            version
        }
        None => {
            checks.warn(format!("no zig is on PATH; {FIX}"));
            None
        }
    };

    if let (Some(path), Some(zig)) = (zon, zig) {
        check_zon(checks, &path, &zig);
    }
}

/// Zig refuses to build with a release older than the minimum. A newer one
/// passes that test, but 0.x lines change the language and the build API, so
/// a newer line is likely to break where fromBuildZigZon gives the line the
/// file names.
fn check_zon(checks: &mut Checks, path: &Path, actual: &str) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            checks.error(format!("cannot read {}: {err}", path.display()));
            return;
        }
    };
    let Some(minimum) = minimum_zig_version(&text) else {
        checks.info(format!(
            "{} sets no .minimum_zig_version; not compared with zig (fromBuildZigZon needs it)",
            path.display()
        ));
        return;
    };
    let pin = format!("{} sets .minimum_zig_version {minimum}", path.display());
    match at_least(&minimum, actual) {
        Some(false) => checks.error(format!(
            "{pin}, but zig is {actual}, which refuses it; {FIX}"
        )),
        Some(true) if newer_zero_line(&minimum, actual) => checks.warn(format!(
            "{pin}, but zig is {actual}, from a newer line; Zig 0.x changes the language and \
             build API between lines, so the project may not build: {FIX}"
        )),
        Some(true) => checks.ok(format!("{pin}, and zig {actual} is as new")),
        None => checks.info(format!(
            "{pin}, which doctor cannot compare with zig {actual} (a dev build?); check it by hand"
        )),
    }
}

/// Whether `actual` is on a later 0.x line than `minimum`. at_least has
/// already read both, so they are releases.
fn newer_zero_line(minimum: &str, actual: &str) -> bool {
    let minimum = exact_version(minimum).or_else(|| exact_version(&format!("{minimum}.0")));
    match (minimum, exact_version(actual)) {
        (Some([0, minor, _]), Some([major, actual_minor, _])) => major > 0 || actual_minor > minor,
        _ => false,
    }
}

/// `.minimum_zig_version`, as fromBuildZigZon reads it: the string form Zig
/// writes since 0.13 (`"0.15.2"`, perhaps with a leading v), or the tuple form
/// before it (`.{ .major = 0, .minor = 15, .patch = 2 }`). Comment lines are
/// skipped so a commented-out pin does not count.
fn minimum_zig_version(text: &str) -> Option<String> {
    let text: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let key = "minimum_zig_version";
    let rest = &text[text.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    if let Some(rest) = rest.strip_prefix('"') {
        let version = &rest[..rest.find('"')?];
        let version = version.trim().trim_start_matches('v');
        return (!version.is_empty()).then(|| version.to_string());
    }
    let tuple = rest.strip_prefix(".{")?;
    let tuple = &tuple[..tuple.find('}')?];
    let field = |name: &str| -> Option<u64> {
        let after = &tuple[tuple.find(&format!(".{name}"))? + name.len() + 1..];
        let after = after.trim_start().strip_prefix('=')?.trim_start();
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        after[..end].parse().ok()
    };
    Some(format!(
        "{}.{}.{}",
        field("major")?,
        field("minor")?,
        field("patch")?
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn reads_the_minimum_zig_version() {
        let string = ".{\n    .name = .app,\n    .version = \"0.1.0\",\n    .minimum_zig_version = \"0.15.2\",\n}\n";
        assert_eq!(minimum_zig_version(string).as_deref(), Some("0.15.2"));
        assert_eq!(
            minimum_zig_version(".{ .minimum_zig_version = \"v0.14\" }").as_deref(),
            Some("0.14")
        );
        let tuple = ".{\n    .minimum_zig_version = .{ .major = 0, .minor = 12, .patch = 1 },\n}\n";
        assert_eq!(minimum_zig_version(tuple).as_deref(), Some("0.12.1"));
        let commented = ".{\n    // .minimum_zig_version = \"0.11.0\",\n    .name = .app,\n}\n";
        assert_eq!(minimum_zig_version(commented), None);
        assert!(newer_zero_line("0.15.2", "0.16.0"));
        assert!(newer_zero_line("0.15", "1.0.0"));
        assert!(!newer_zero_line("0.15.1", "0.15.2"));
        assert!(!newer_zero_line("1.0.0", "1.1.0"));
    }

    #[test]
    #[cfg(unix)]
    fn compares_zig_with_build_zig_zon() {
        let root = scratch("zig");
        let (first, second, project) = (root.join("first"), root.join("second"), root.join("p"));
        write_executable(&first.join("zig"), b"#!/bin/sh\necho 0.14.1\n");
        write_executable(&second.join("zig"), b"#!/bin/sh\necho 0.15.2\n");
        fs::create_dir_all(project.join("src")).unwrap();
        let zon = |version: &str| {
            fs::write(
                project.join("build.zig.zon"),
                format!(".{{ .name = .app, .minimum_zig_version = \"{version}\" }}\n"),
            )
            .unwrap();
        };

        zon("0.15.2");
        let env = fake_env(&[&first, &second], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project.join("src"), &env));
        assert_line(&lines, "is zig 0.14.1");
        assert_line(&lines, "another zig is on PATH: ");
        assert_line(
            &lines,
            "sets .minimum_zig_version 0.15.2, but zig is 0.14.1, which refuses it",
        );
        assert_eq!(errors, 1);

        zon("0.13.0");
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "but zig is 0.14.1, from a newer line");
        assert_eq!(errors, 0);

        zon("0.14.0");
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "and zig 0.14.1 is as new");

        zon("0.16.0-dev.1+abc");
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "which doctor cannot compare with zig 0.14.1");

        let env = fake_env(&[&root.join("none")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "warn  no zig is on PATH; use `lib.<system>.zig.fromBuildZigZon`",
        );

        // Neither zig nor a build.zig.zon: nothing to say.
        let (lines, _) = run_check(|checks| check(checks, &root, &env));
        assert!(lines.is_empty(), "{lines:?}");

        fs::remove_dir_all(&root).unwrap();
    }
}
