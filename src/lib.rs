//! Extensions for git2
//!
//! Goals:
//! - Provide "good enough" implementations of essential or higher-level git2 logic, like cherry-pick, squash, hooks, authentication (not implemented yet), etc
//! - The above serves as examples for people needing to write their own implementations

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod hooks;
pub mod ops;
pub mod tree;
pub mod utils;

pub(crate) mod bytes;

#[cfg(test)]
mod testing;
