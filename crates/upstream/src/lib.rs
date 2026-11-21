//! Downloading, verifying and parsing upstream release metadata, split out
//! so the trimming and parsing rules can be tested without a network or a
//! process.

pub mod gpg;
pub mod http;
pub mod manifest;
pub mod pnpm_lock;
pub mod sri;
