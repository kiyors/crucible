//! Everything behind `crucible update` and `crucible pin`: both turn
//! upstream metadata into pins, one into data/, the other for a project.

// Re-exported so `crate::http` and friends read the same as they did before
// the split, and `upstream` stays the one place they are defined.
pub use upstream::{attestation, gpg, http, manifest, npm, pnpm_lock, sri};

pub mod pin;
pub mod update;
