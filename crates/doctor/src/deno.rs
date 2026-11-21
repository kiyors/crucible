//! Deno. deno.json has no field for the Deno release a project needs, and
//! lib/deno reads none, so the version is reported but not compared; what is
//! checked is that a project with a deno.json has a deno, and only one.

use std::path::Path;

use super::{find_nearest_file, shadowed, stdout_of, Checks, Env};

pub(super) fn check(checks: &mut Checks, cwd: &Path, env: &Env) {
    let denos = env.executables("deno");
    let config = find_nearest_file(cwd, &["deno.json", "deno.jsonc"]);
    let Some(first) = denos.first() else {
        if let Some(config) = config {
            checks.warn(format!(
                "{} is here, but no deno is on PATH; add `lib.<system>.deno.toolchain` to the \
                 dev shell",
                config.display()
            ));
        }
        return;
    };

    // `deno 2.9.8 (stable, release, aarch64-apple-darwin)`, then v8 and
    // typescript lines.
    if let Some(output) = stdout_of(checks, first, &["--version"], &[]) {
        let line = output.lines().next().unwrap_or_default();
        let version = line.split_whitespace().nth(1).unwrap_or(line);
        checks.ok(format!("{} is deno {version}", first.display()));
    }
    for message in shadowed("deno", &denos) {
        checks.warn(message);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::test_support::*;
    use super::*;

    #[test]
    #[cfg(unix)]
    fn checks_deno_and_its_copies() {
        let root = scratch("deno");
        let (first, second, project) = (root.join("first"), root.join("second"), root.join("p"));
        write_executable(
            &first.join("deno"),
            b"#!/bin/sh\necho 'deno 2.9.8 (stable, release, aarch64-apple-darwin)'\necho 'v8 14.1'\n",
        );
        write_executable(
            &second.join("deno"),
            b"#!/bin/sh\necho 'deno 2.1.0 (stable)'\n",
        );
        fs::create_dir_all(&project).unwrap();

        let env = fake_env(&[&first, &second], &[]);
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "is deno 2.9.8");
        assert_line(&lines, "another deno is on PATH: ");
        assert_eq!(errors, 0);

        let env = fake_env(&[&root.join("none")], &[]);
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");

        fs::write(project.join("deno.jsonc"), "// tasks\n{}\n").unwrap();
        let (lines, _) = run_check(|checks| check(checks, &project, &env));
        assert_line(
            &lines,
            "deno.jsonc is here, but no deno is on PATH; add `lib.<system>.deno.toolchain`",
        );

        fs::remove_dir_all(&root).unwrap();
    }
}
