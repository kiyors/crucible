//! The HashiCorp tools are a nixpkgs-only stack. There is no project version
//! file to read, damon has no release assets, tflint and terragrunt need
//! bespoke handlers, and four of the tools are unfree. A `*.tf` file is the
//! only thing doctor can notice; it reports the policy as info, not an error.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use super::{Checks, Env};

const FIX: &str = "HashiCorp tools are nixpkgs-only in crucible (no project version file, \
                   damon has no release assets, tflint and terragrunt need bespoke handlers, \
                   and four tools are unfree); use the `hash` template and see \
                   docs/template-policy.md";

pub(super) fn check(checks: &mut Checks, cwd: &Path, _env: &Env) {
    let has_terraform = fs::read_dir(cwd)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry.path().extension() == Some(OsStr::new("tf"))
        });
    if has_terraform {
        checks.info(format!("{} has a *.tf file; {FIX}", cwd.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn explains_hashicorp_only_when_a_tf_file_is_here() {
        let root = scratch("hashi");
        let project = root.join("p");
        fs::create_dir_all(&project).unwrap();
        let env = fake_env(&[&root.join("bin")], &[]);

        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);

        fs::write(project.join("notes.txt"), "not terraform\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);

        fs::write(
            project.join("main.tf"),
            "resource \"null_resource\" \"x\" {}\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "has a *.tf file; HashiCorp tools are nixpkgs-only");
        assert_line(&lines, "damon has no release assets");
        assert_line(&lines, "docs/template-policy.md");
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }
}
