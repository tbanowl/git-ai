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

const SHIM_MODE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_MODE";
const SHIM_STATE_FILE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_STATE_FILE";
const SHIM_SLEEP_MS_ENV: &str = "GIT_AI_TEST_GIT_SHIM_SLEEP_MS";
const SHIM_STDERR_ENV: &str = "GIT_AI_TEST_GIT_SHIM_STDERR";
const SHIM_EXIT_CODE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_EXIT_CODE";
const SHIM_PID_FILE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_PID_FILE";

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

fn read_and_increment_invocation(path: Option<&str>) -> usize {
    let Some(path) = path else {
        return 0;
    };
    let path = PathBuf::from(path);
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    fs::write(&path, format!("{}\n", current + 1)).expect("write shim state");
    current
}

fn configured_sleep_duration() -> std::time::Duration {
    let millis = env::var(SHIM_SLEEP_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000);
    std::time::Duration::from_millis(millis)
}

fn configured_exit_code() -> i32 {
    env::var(SHIM_EXIT_CODE_ENV)
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(128)
}

fn configured_stderr() -> String {
    env::var(SHIM_STDERR_ENV)
        .unwrap_or_else(|_| "fatal: temporary git lock unavailable".to_string())
}

fn maybe_write_pid_file() {
    let Ok(path) = env::var(SHIM_PID_FILE_ENV) else {
        return;
    };
    fs::write(path, format!("{}\n", std::process::id())).expect("write shim pid file");
}

fn fail_with_stderr(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    std::process::exit(code);
}

fn run_shim(target: &str, effective_argv: &[String], use_git_ai_wrapper_mode: bool) -> ! {
    let mode = env::var(SHIM_MODE_ENV).unwrap_or_else(|_| "pass_through".to_string());
    let state_path = env::var(SHIM_STATE_FILE_ENV).ok();
    let invocation = read_and_increment_invocation(state_path.as_deref());
    maybe_write_pid_file();

    match mode.as_str() {
        "pass_through" => exec_target(target, effective_argv, use_git_ai_wrapper_mode),
        "sleep_always" => {
            std::thread::sleep(configured_sleep_duration());
            exec_target(target, effective_argv, use_git_ai_wrapper_mode)
        }
        "sleep_then_success_once" => {
            if invocation == 0 {
                std::thread::sleep(configured_sleep_duration());
            }
            exec_target(target, effective_argv, use_git_ai_wrapper_mode)
        }
        "stderr_once_then_success" => {
            if invocation == 0 {
                fail_with_stderr(&configured_stderr(), configured_exit_code());
            }
            exec_target(target, effective_argv, use_git_ai_wrapper_mode)
        }
        other => {
            eprintln!("unknown shim mode: {other}");
            std::process::exit(2);
        }
    }
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
    run_shim(&target, &effective_argv, use_git_ai_wrapper_mode);
}

#[cfg(not(unix))]
fn main() {
    let argv = env::args().skip(1).collect::<Vec<_>>();
    let (target, use_git_ai_wrapper_mode) =
        select_target(&argv).unwrap_or_else(|error| panic!("{error}"));
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
    run_shim(&target, &effective_argv, use_git_ai_wrapper_mode)
}
