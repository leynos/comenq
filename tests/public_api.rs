//! Compiler-checked examples for the public client and daemon APIs.

use comenq::{Args, Command, RepoSlugParseError};
use comenqd::config::Config;
use std::path::PathBuf;

#[test]
fn changed_public_apis_compile_as_documented() -> Result<(), RepoSlugParseError> {
    let socket_path = PathBuf::from("/run/user/1000/comenq/comenq.sock");
    let args = Args {
        socket: Some(socket_path.clone()),
        command: Command::Put {
            repo_slug: "octocat/hello-world".parse()?,
            pr_number: 1,
            comment_body: "Hello".into(),
            now: false,
        },
    };
    let socket_candidates: Vec<PathBuf> = args.socket_candidates();
    assert_eq!(socket_candidates, vec![socket_path.clone()]);

    let token_file = PathBuf::from("/run/credentials/comenqd/token");
    let token_files = vec![
        PathBuf::from("/run/credentials/comenqd/token-a"),
        PathBuf::from("/run/credentials/comenqd/token-b"),
    ];
    let config = Config {
        github_token: "token".into(),
        github_token_file: Some(token_file.clone()),
        github_token_files: token_files.clone(),
        socket_path,
        queue_path: PathBuf::from("/var/lib/comenq/queue"),
        cooldown_period_seconds: 960,
        cooldown_flutter_seconds: 30,
        restart_min_delay_ms: 100,
        github_api_timeout_secs: 30,
    };

    let github_token_file: Option<PathBuf> = config.github_token_file;
    let github_token_files: Vec<PathBuf> = config.github_token_files;
    let cooldown_flutter_seconds: u64 = config.cooldown_flutter_seconds;
    assert_eq!(github_token_file, Some(token_file));
    assert_eq!(github_token_files, token_files);
    assert_eq!(cooldown_flutter_seconds, 30);

    Ok(())
}
