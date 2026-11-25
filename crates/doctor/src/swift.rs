//! Swift is a nixpkgs-only toolchain. Swift's release index has no toolchain
//! checksum, macOS ships an Apple Developer-ID `.pkg` that CI cannot verify or
//! unpack on Linux, and the official artifacts need Xcode/Apple SDK setup.
//! There is nothing for crucible to pin, so doctor only says so: an info line,
//! never an error.

use std::path::Path;

use super::{find_nearest_file, Checks, Env};

const FIX: &str = "Swift is nixpkgs-only in crucible (no release checksum, an Apple \
                   Developer-ID .pkg on macOS and Xcode/Apple SDK setup); use the `swift` \
                   template and see docs/template-policy.md";

pub(super) fn check(checks: &mut Checks, cwd: &Path, _env: &Env) {
    let Some(file) = find_nearest_file(cwd, &["Package.swift", ".swift-version"]) else {
        return;
    };
    checks.info(format!("{} is here; {FIX}", file.display()));
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::test_support::*;
    use super::*;

    #[test]
    fn explains_swift_only_when_a_project_file_is_here() {
        let root = scratch("swift");
        let project = root.join("p");
        fs::create_dir_all(&project).unwrap();
        let env = fake_env(&[&root.join("bin")], &[]);

        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(errors, 0);

        fs::write(
            project.join("Package.swift"),
            "// swift-tools-version: 5.10\n",
        )
        .unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, "Package.swift is here; Swift is nixpkgs-only");
        assert_line(&lines, "docs/template-policy.md");
        assert_eq!(errors, 0);

        fs::remove_file(project.join("Package.swift")).unwrap();
        fs::write(project.join(".swift-version"), "6.0.2\n").unwrap();
        let (lines, errors) = run_check(|checks| check(checks, &project, &env));
        assert_line(&lines, ".swift-version is here; Swift is nixpkgs-only");
        assert_eq!(errors, 0);

        fs::remove_dir_all(&root).unwrap();
    }
}
