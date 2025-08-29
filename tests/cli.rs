use rstest_bdd_macros::scenario;

#[path = "steps/cli_steps.rs"]
mod cli_steps;

#[scenario(path = "tests/features/cli.feature")]
fn cli_feature() {}
