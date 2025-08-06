//! Re-exported daemon test helpers.
//!
//! This crate now delegates to `test_support`. Prefer using `test_support`
//! directly in new code.

pub use test_support::{octocrab_for, temp_config};
