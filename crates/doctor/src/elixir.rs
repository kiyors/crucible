//! Elixir is a nixpkgs-only toolchain. The compiler is a platform-independent
//! zip, but OTP ships only as source tarballs: a two-axis OTP x Elixir pin
//! would mean per-system source builds, and `mix.exs` cannot name OTP (only
//! `.tool-versions` does). Doctor says so in an info line, never an error.

use std::fs;
use std::path::Path;

use super::{find_nearest_file, Checks, Env};

const FIX: &str = "Elixir is nixpkgs-only in crucible (OTP ships only as source tarballs, \
                   so a two-axis OTP x Elixir pin needs per-system source builds, and mix.exs \
                   cannot name OTP); use the `elixir` template and see docs/template-policy.md";

pub(super) fn check(checks: &mut Checks, cwd: &Path, _env: &Env) {
    // `mix.exs` is the project file; `.tool-versions` counts only when it
    // actually names Elixir or Erlang, so a Node-only project stays quiet.
    let found = find_nearest_file(cwd, &["mix.exs"])
        .or_else(|| find_nearest_file(cwd, &[".tool-versions"]).filter(|path| names_beam(path)));
    let Some(file) = found else {
        return;
    };
    checks.info(format!("{} is here; {FIX}", file.display()));
}

fn names_beam(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| {
        text.lines().any(|line| {
            let line = line.trim();
            line.starts_with("elixir ") || line.starts_with("erlang ")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn explains_elixir_only_for_beam_projects() {
        let root = scratch("elixir");
        let project = root.join("p");
        fs::create_dir_all(&project).unwrap();
        let env = fake_env(&[&root.join("bin")], &[]);

        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);

        // A `.tool-versions` for another language is not an Elixir project.
        fs::write(project.join(".tool-versions"), "nodejs 22.14.0\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);

        fs::write(
            project.join(".tool-versions"),
            "erlang 27.0\nelixir 1.17.3-otp-27\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, ".tool-versions is here; Elixir is nixpkgs-only");
        assert_line(&lines, "mix.exs cannot name OTP");
        assert_line(&lines, "docs/template-policy.md");
        assert_eq!(errors, 0);

        fs::write(
            project.join("mix.exs"),
            "defmodule Demo.MixProject do\nend\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "mix.exs is here; Elixir is nixpkgs-only");
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }
}
