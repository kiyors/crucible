//! The one list of systems crucible builds for, and the rules that turn it
//! into per-toolchain names.
//!
//! Nix evaluation is offline and the main flake has one input, so this list
//! cannot be generated from `lib/common/systems.nix`. It is the canonical copy
//! on the Rust side: the other tables keep their human-readable literals (the
//! source sync test reads those as text) but are compile-time checked against
//! this one, so adding a system here cannot be forgotten in a pin updater.

/// The systems of `lib/common/systems.nix`, in the same order.
pub const SYSTEMS: [&str; 3] = ["x86_64-linux", "aarch64-linux", "aarch64-darwin"];

/// Byte equality that works in a `const fn`. Matching on `&str` is not const
/// yet, so the mapping below uses this instead.
const fn str_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The Rust host triple Nix builds for on `system`.
pub const fn host(system: &str) -> &'static str {
    if str_eq(system, "x86_64-linux") {
        "x86_64-unknown-linux-gnu"
    } else if str_eq(system, "aarch64-linux") {
        "aarch64-unknown-linux-gnu"
    } else if str_eq(system, "aarch64-darwin") {
        "aarch64-apple-darwin"
    } else {
        panic!("crucible: no Rust host triple for this system; edit upstream platforms")
    }
}

/// The host triples derived from `SYSTEMS`. Modules that must keep a literal
/// array for the text-based sync check assert against this at compile time.
pub const fn hosts() -> [&'static str; 3] {
    [host(SYSTEMS[0]), host(SYSTEMS[1]), host(SYSTEMS[2])]
}

/// Element-by-element equality of two system/host lists. `const` so a table
/// can be checked when the crate is compiled, not only when tests run.
pub const fn same(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if !str_eq(a[i], b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether a `system -> target` mapping lists exactly `SYSTEMS`, in order.
pub const fn matches<const N: usize>(table: [(&str, &str); N]) -> bool {
    if N != SYSTEMS.len() {
        return false;
    }
    let mut i = 0;
    while i < N {
        if !str_eq(table[i].0, SYSTEMS[i]) {
            return false;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{manifest, pnpm_lock};

    #[test]
    fn the_canonical_table_is_the_one_from_systems_nix() {
        assert_eq!(SYSTEMS, ["x86_64-linux", "aarch64-linux", "aarch64-darwin"]);
        assert_eq!(
            hosts(),
            [
                "x86_64-unknown-linux-gnu",
                "aarch64-unknown-linux-gnu",
                "aarch64-apple-darwin"
            ]
        );
        // Every copy in upstream is checked against the same table.
        assert!(same(&manifest::HOSTS, &hosts()));
        assert!(same(
            &[
                pnpm_lock::PLATFORMS[0].system,
                pnpm_lock::PLATFORMS[1].system,
                pnpm_lock::PLATFORMS[2].system,
            ],
            &SYSTEMS
        ));
    }
}
