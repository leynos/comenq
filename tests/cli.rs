//! Behavioural tests for the CLI argument parser.
//!
//! This module binds the feature file to the step definitions using
//! `rstest-bdd`, ensuring each scenario runs with isolated fixtures.

use rstest_bdd_macros::scenario;

#[path = "steps/cli_steps.rs"]
mod cli_steps;
use cli_steps::{CliState, cli_state};

/// Execute scenarios from `tests/features/cli.feature`.
#[scenario(path = "tests/features/cli.feature")]
fn cli_feature(cli_state: CliState) {
    let _ = cli_state;
}
