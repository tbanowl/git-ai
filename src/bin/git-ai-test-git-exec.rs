use git_ai::error::GitAiError;
use git_ai::git::repository::{InternalGitProfile, exec_git_with_profile, exec_git_with_timeout};
use serde::Serialize;
use std::env;
use std::time::Duration;

#[derive(Serialize)]
struct ExecResult {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn repo_git_dir_args() -> Vec<String> {
    vec![
        "-C".to_string(),
        env!("CARGO_MANIFEST_DIR").to_string(),
        "rev-parse".to_string(),
        "--git-dir".to_string(),
    ]
}

fn emit(result: &ExecResult) {
    println!(
        "{}",
        serde_json::to_string(result).expect("serialize git exec result")
    );
}

fn from_error(err: GitAiError) -> ExecResult {
    match err {
        GitAiError::GitCliError { code, stderr, .. } => ExecResult {
            ok: false,
            code,
            stdout: String::new(),
            stderr,
        },
        other => ExecResult {
            ok: false,
            code: None,
            stdout: String::new(),
            stderr: other.to_string(),
        },
    }
}

fn main() {
    let op = env::args().nth(1).expect("expected operation arg");
    let args = repo_git_dir_args();

    let result = match op.as_str() {
        "timeout" => match exec_git_with_timeout(&args, Duration::from_millis(100)) {
            Ok(output) => ExecResult {
                ok: true,
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            },
            Err(err) => from_error(err),
        },
        "profile" => match exec_git_with_profile(&args, InternalGitProfile::General) {
            Ok(output) => ExecResult {
                ok: true,
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            },
            Err(err) => from_error(err),
        },
        other => panic!("unknown operation: {other}"),
    };

    emit(&result);
}
