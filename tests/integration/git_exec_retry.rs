use crate::repos::test_repo::real_git_executable;
use serde::Deserialize;
use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn shim_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai-test-git-shim"))
}

fn exec_helper_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai-test-git-exec"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn temp_test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("git-ai-{name}-{}", Uuid::new_v4()))
}

#[derive(Debug, Deserialize)]
struct ExecResult {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_exec_helper(operation: &str, extra_env: &[(&str, String)]) -> ExecResult {
    let patch = git_ai::config::ConfigPatch {
        git_path: Some(shim_binary().display().to_string()),
        ..Default::default()
    };

    let output = Command::new(exec_helper_binary())
        .arg(operation)
        .current_dir(repo_root())
        .env(
            "GIT_AI_TEST_CONFIG_PATCH",
            serde_json::to_string(&patch).expect("serialize config patch"),
        )
        .env("GIT_AI_TEST_GIT_SHIM_TARGET", real_git_executable())
        .env(
            "GIT_AI_TEST_GIT_SHIM_FALLBACK_TARGET",
            real_git_executable(),
        )
        .envs(extra_env.iter().map(|(k, v)| (*k, v)))
        .output()
        .expect("run git exec helper");

    assert!(
        output.status.success(),
        "helper failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("parse helper json")
}

#[cfg(unix)]
fn shim_process_still_running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .expect("run ps for shim pid");

    if !output.status.success() {
        return false;
    }

    let command = String::from_utf8_lossy(&output.stdout);
    command.contains("git-ai-test-git-shim")
}

#[test]
fn test_git_exec_shim_passthrough_mode_smoke() {
    let output = Command::new(shim_binary())
        .arg("--version")
        .env("GIT_AI_TEST_GIT_SHIM_TARGET", real_git_executable())
        .env("GIT_AI_TEST_GIT_SHIM_REAL_GIT", real_git_executable())
        .env("GIT_AI_TEST_GIT_SHIM_MODE", "pass_through")
        .current_dir(repo_root())
        .output()
        .expect("run shim");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("git version"));
}

#[test]
#[serial]
fn test_exec_git_with_timeout_returns_timeout_error_without_retry() {
    let started = Instant::now();
    let result = run_exec_helper(
        "timeout",
        &[
            ("GIT_AI_TEST_GIT_SHIM_MODE", "sleep_always".to_string()),
            ("GIT_AI_TEST_GIT_SHIM_SLEEP_MS", "1000".to_string()),
        ],
    );

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!result.ok);
    assert_eq!(result.code, Some(1));
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, "Command timed out");
}

#[test]
#[serial]
fn test_exec_git_default_helper_does_not_retry_retryable_stderr_failures() {
    let state_file = temp_test_path("git-exec-no-retry-state.txt");
    let result = run_exec_helper(
        "profile",
        &[
            (
                "GIT_AI_TEST_GIT_SHIM_MODE",
                "stderr_once_then_success".to_string(),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.display().to_string(),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STDERR",
                "fatal: Unable to create '/tmp/repo/.git/index.lock': File exists.".to_string(),
            ),
            ("GIT_AI_TEST_GIT_SHIM_EXIT_CODE", "128".to_string()),
        ],
    );

    assert!(!result.ok);
    assert_eq!(result.code, Some(128));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.contains("index.lock"));
    assert_eq!(
        fs::read_to_string(&state_file)
            .expect("read shim state file")
            .trim(),
        "1"
    );
}

#[test]
#[cfg(unix)]
#[serial]
fn test_exec_git_timeout_kills_and_reaps_hung_shim_process() {
    let pid_file = temp_test_path("git-exec-timeout.pid");
    let result = run_exec_helper(
        "timeout",
        &[
            ("GIT_AI_TEST_GIT_SHIM_MODE", "sleep_always".to_string()),
            (
                "GIT_AI_TEST_GIT_SHIM_PID_FILE",
                pid_file.display().to_string(),
            ),
            ("GIT_AI_TEST_GIT_SHIM_SLEEP_MS", "5000".to_string()),
        ],
    );

    assert!(!result.ok);
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, "Command timed out");

    let pid: u32 = fs::read_to_string(&pid_file)
        .expect("read shim pid file")
        .trim()
        .parse()
        .expect("parse shim pid");

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if !shim_process_still_running(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    panic!("timed out waiting for shim pid {pid} to exit after timeout kill");
}
