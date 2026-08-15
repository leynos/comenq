use comenq::Args;
use comenqd::config::Config;
use std::path::PathBuf;

fn main() {
    let args = Args {
        repo_slug: "octocat/hello-world".parse().expect("parse repository slug"),
        pr_number: 1,
        comment_body: "Hello".into(),
        socket: None,
    };
    let _: Vec<PathBuf> = args.socket_candidates();

    let config = Config {
        github_token: "token".into(),
        github_token_file: Some(PathBuf::from("/run/credentials/comenqd/token")),
        socket_path: PathBuf::from("/run/user/1000/comenq/comenq.sock"),
        queue_path: PathBuf::from("/var/lib/comenq/queue"),
        cooldown_period_seconds: 960,
        cooldown_flutter_seconds: 30,
        restart_min_delay_ms: 100,
        github_api_timeout_secs: 30,
        client_channel_capacity: 1024,
    };
    let _: Option<PathBuf> = config.github_token_file;
    let _: u64 = config.cooldown_flutter_seconds;
    let _: usize = config.client_channel_capacity;
}
