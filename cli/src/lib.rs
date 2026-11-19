//! The pieces behind the `crucible` binary, split out so the trimming and
//! parsing rules can be tested without a network or a process.

pub mod doctor;
pub mod http;
pub mod manifest;
pub mod pin;
pub mod sri;
pub mod update;
