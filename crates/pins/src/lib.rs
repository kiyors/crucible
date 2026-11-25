//! Everything behind `crucible update` and `crucible pin`: both turn
//! upstream metadata into pins, one into data/, the other for a project.

// The modules stay in `upstream`; this alias only lets internal code keep
// writing `crate::http`. It is not part of pins' public surface.
pub(crate) use upstream::{attestation, gpg, http, manifest, npm, platforms, pnpm_lock, sri};

pub mod pin;
pub(crate) mod source;
pub mod update;

// `init` cannot depend on `upstream` (it is in the network-free user build),
// so its `SYSTEMS` cannot derive from the canonical table; check it here where
// both crates are available instead.
const _: () = assert!(platforms::same(&init::SYSTEMS, &platforms::SYSTEMS));

#[cfg(test)]
mod tests {
    use crate::platforms::SYSTEMS;
    use crate::update::{
        cargo, deno, elm, go, node, package_managers, python, tsrs, vite_plus, zig,
    };

    /// One system list on the Rust side: the canonical one in `upstream`, the
    /// one in `init` (which cannot depend on `upstream`), and every pin
    /// updater's mapping table. The literal tables are also checked at compile
    /// time; this test keeps the check visible when someone edits one.
    #[test]
    fn every_system_table_agrees() {
        assert!(crate::platforms::same(&init::SYSTEMS, &SYSTEMS));
        assert!(crate::platforms::matches(node::PLATFORMS));
        assert!(crate::platforms::matches(python::PLATFORMS));
        assert!(crate::platforms::matches(go::PLATFORMS));
        assert!(crate::platforms::matches(cargo::PLATFORMS));
        assert!(crate::platforms::matches(vite_plus::PLATFORMS));
        assert!(crate::platforms::matches(deno::DENO_ASSETS));
        assert!(crate::platforms::matches(elm::PLATFORMS));
        assert!(crate::platforms::matches(zig::PLATFORMS));
        assert!(crate::platforms::matches(tsrs::PLATFORMS));
        assert!(crate::platforms::matches(package_managers::BUN_ASSETS));
        for line in &package_managers::PNPM {
            assert!(crate::platforms::matches(line.platforms));
        }
    }
}
