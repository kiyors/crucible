//! The pieces behind the `crucible` binary, split out so the trimming and
//! parsing rules can be tested without a network or a process.

pub mod doctor;
#[cfg(feature = "net")]
pub mod http;
#[cfg(feature = "net")]
pub mod manifest;
#[cfg(feature = "net")]
pub mod pin;
#[cfg(feature = "net")]
pub mod sri;
#[cfg(feature = "net")]
pub mod update;
