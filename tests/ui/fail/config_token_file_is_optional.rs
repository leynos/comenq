//! Compile-fail fixture proving that `Config::github_token_file` is optional.

use comenqd::config::Config;
use std::path::PathBuf;

fn main() {
    let config = Config {
        github_token: "token".into(),
        github_token_file: None,
        socket_path: PathBuf::from("/run/comenq/comenq.sock"),
        queue_path: PathBuf::from("/var/lib/comenq/queue"),
        cooldown_period_seconds: 960,
        cooldown_flutter_seconds: 0,
        restart_min_delay_ms: 100,
        github_api_timeout_secs: 30,
    };
    let _: PathBuf = config.github_token_file;
}
