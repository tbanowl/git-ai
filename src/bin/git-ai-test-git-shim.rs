use git_ai::daemon::test_sync::{
    TEST_SYNC_SESSION_CONFIG_KEY, tracked_parsed_git_invocation_for_test_sync,
    tracks_parsed_git_invocation_for_test_sync,
};
use serde::Serialize;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
#[cfg(not(unix))]
use std::process::Stdio;
use uuid::Uuid;

fn shim_mode() -> Option<String> {
    env::var("GIT_AI_TEST_GIT_SHIM_MODE").ok()
}

fn write_optional_file(path_var: &str, contents: &str) -> Result<(), String> {
    let Ok(path) = env::var(path_var) else {
        return Ok(());
    };
    fs::write(path, contents).map_err(|e| format!("write {path_var} failed: {e}"))
}

fn run_sleep_always_mode() -> Result<(), String> {
    write_optional_file(
        "GIT_AI_TEST_GIT_SHIM_PID_FILE",
        &std::process::id().to_string(),
    )?;

    let sleep_ms = env::var("GIT_AI_TEST_GIT_SHIM_SLEEP_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1000);
    std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
    Ok(())
}

fn run_stderr_once_then_success_mode() -> Result<bool, String> {
    let state_file = env::var("GIT_AI_TEST_GIT_SHIM_STATE_FILE")
        .map_err(|_| "GIT_AI_TEST_GIT_SHIM_STATE_FILE is required".to_string())?;
    let current_attempt = fs::read_to_string(&state_file)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let next_attempt = current_attempt + 1;
    fs::write(&state_file, next_attempt.to_string())
        .map_err(|e| format!("write state file failed: {e}"))?;

    if current_attempt == 0 {
        let stderr = env::var("GIT_AI_TEST_GIT_SHIM_STDERR")
            .unwrap_or_else(|_| "shim stderr_once_then_success".to_string());
        let exit_code = env::var("GIT_AI_TEST_GIT_SHIM_EXIT_CODE")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(1);
        eprintln!("{stderr}");
        std::process::exit(exit_code);
    }

    Ok(true)
}

fn maybe_run_test_mode() -> Result<bool, String> {
    match shim_mode().as_deref() {
        Some("sleep_always") => {
            run_sleep_always_mode()?;
            Ok(true)
        }
        Some("stderr_once_then_success") => run_stderr_once_then_success_mode(),
        _ => Ok(false),
    }
}

#[derive(Serialize)]
struct StartedGitInvocationLogEntry {
    command: Option<String>,
    command_args: Vec<String>,
    cwd: Option<String>,
    test_sync_session: Option<String>,
}

fn select_target(argv: &[String]) -> Result<(String, bool), String> {
    let tracked_target = env::var("GIT_AI_TEST_GIT_SHIM_TARGET")
        .map_err(|_| "GIT_AI_TEST_GIT_SHIM_TARGET is required".to_string())?;
    let fallback_target =
        env::var("GIT_AI_TEST_GIT_SHIM_FALLBACK_TARGET").unwrap_or_else(|_| tracked_target.clone());
    let tracked_target_uses_git_ai =
        env::var("GIT_AI_TEST_GIT_SHIM_TARGET_USE_GIT_AI").as_deref() == Ok("1");
    let cwd = env::current_dir().map_err(|e| format!("read shim cwd failed: {e}"))?;
    let parsed = tracked_parsed_git_invocation_for_test_sync(argv, &cwd);
    if tracks_parsed_git_invocation_for_test_sync(&parsed) {
        Ok((tracked_target, tracked_target_uses_git_ai))
    } else {
        Ok((fallback_target, false))
    }
}

fn append_started_log(
    log_path: &PathBuf,
    argv: &[String],
    test_sync_session: Option<&str>,
) -> Result<(), String> {
    let cwd = env::current_dir().map_err(|e| format!("read shim cwd failed: {e}"))?;
    let parsed = tracked_parsed_git_invocation_for_test_sync(argv, &cwd);
    if !tracks_parsed_git_invocation_for_test_sync(&parsed) {
        return Ok(());
    }

    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create shim log dir failed: {e}"))?;
    }

    let entry = StartedGitInvocationLogEntry {
        command: parsed.command.clone(),
        command_args: parsed.command_args.clone(),
        cwd: Some(cwd.to_string_lossy().to_string()),
        test_sync_session: test_sync_session.map(str::to_string),
    };
    let mut line = serde_json::to_vec(&entry).map_err(|e| format!("serialize shim log: {e}"))?;
    line.push(b'\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("open shim log failed: {e}"))?;
    file.write_all(&line)
        .map_err(|e| format!("write shim log failed: {e}"))?;
    file.flush()
        .map_err(|e| format!("flush shim log failed: {e}"))?;
    Ok(())
}

fn new_test_sync_session() -> String {
    format!("gt-shim-{}", Uuid::new_v4())
}

fn argv_with_test_sync_session(argv: &[String], test_sync_session: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len() + 2);
    out.push("-c".to_string());
    out.push(format!(
        "{}={}",
        TEST_SYNC_SESSION_CONFIG_KEY, test_sync_session
    ));
    out.extend(argv.iter().cloned());
    out
}

#[cfg(unix)]
fn exec_target(target: &str, argv: &[String], use_git_ai_wrapper_mode: bool) -> ! {
    let mut command = Command::new(target);
    command.args(argv);
    if use_git_ai_wrapper_mode {
        command.env("GIT_AI", "git");
    }
    let error = command.exec();
    eprintln!("git-ai-test-git-shim failed to exec {target}: {error}");
    std::process::exit(127);
}

#[cfg(not(unix))]
fn exec_target(target: &str, argv: &[String], use_git_ai_wrapper_mode: bool) -> ! {
    let mut command = Command::new(target);
    command
        .args(argv)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if use_git_ai_wrapper_mode {
        command.env("GIT_AI", "git");
    }
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("git-ai-test-git-shim failed to spawn {target}: {error}");
            std::process::exit(127);
        }
    }
}

#[cfg(unix)]
fn main() {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    let (target, use_git_ai_wrapper_mode) =
        select_target(&argv).unwrap_or_else(|error| panic!("{error}"));
    if maybe_run_test_mode().unwrap_or_else(|error| panic!("{error}")) {
        std::process::exit(0);
    }
    let mut effective_argv = argv.clone();
    let mut test_sync_session = None;
    if let Ok(log_path) = env::var("GIT_AI_TEST_SYNC_START_LOG") {
        let log_path = PathBuf::from(log_path);
        let cwd =
            env::current_dir().unwrap_or_else(|error| panic!("read shim cwd failed: {error}"));
        let parsed = tracked_parsed_git_invocation_for_test_sync(&argv, &cwd);
        if tracks_parsed_git_invocation_for_test_sync(&parsed) {
            test_sync_session = Some(new_test_sync_session());
            if let Some(session) = test_sync_session.as_deref() {
                effective_argv = argv_with_test_sync_session(&argv, session);
            }
        }
        if let Err(error) = append_started_log(&log_path, &argv, test_sync_session.as_deref()) {
            panic!("git-ai-test-git-shim failed: {error}");
        }
    }
    exec_target(&target, &effective_argv, use_git_ai_wrapper_mode);
}

#[cfg(not(unix))]
fn main() {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    let (target, use_git_ai_wrapper_mode) =
        select_target(&argv).unwrap_or_else(|error| panic!("{error}"));
    if maybe_run_test_mode().unwrap_or_else(|error| panic!("{error}")) {
        std::process::exit(0);
    }
    let mut effective_argv = argv.clone();
    let mut test_sync_session = None;
    if let Ok(log_path) = env::var("GIT_AI_TEST_SYNC_START_LOG") {
        let log_path = PathBuf::from(log_path);
        let cwd =
            env::current_dir().unwrap_or_else(|error| panic!("read shim cwd failed: {error}"));
        let parsed = tracked_parsed_git_invocation_for_test_sync(&argv, &cwd);
        if tracks_parsed_git_invocation_for_test_sync(&parsed) {
            test_sync_session = Some(new_test_sync_session());
            if let Some(session) = test_sync_session.as_deref() {
                effective_argv = argv_with_test_sync_session(&argv, session);
            }
        }
        if let Err(error) = append_started_log(&log_path, &argv, test_sync_session.as_deref()) {
            panic!("git-ai-test-git-shim failed: {error}");
        }
    }
    exec_target(&target, &effective_argv, use_git_ai_wrapper_mode)
}
