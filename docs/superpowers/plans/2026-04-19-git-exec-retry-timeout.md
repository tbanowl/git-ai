# Git Exec Retry And Timeout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a unified git execution flow in `src/git/repository.rs` that enforces real subprocess timeouts, retries timeout/transient git execution failures, preserves the existing public `exec_git*` APIs, and adds stable tests for the new behavior.

**Architecture:** Keep the public `exec_git*` and `exec_git_stdin*` entry points unchanged, but route them through a shared internal request/policy pipeline. Replace the current thread-channel timeout path with a real `spawn`-based execution flow that can kill and reap hung git children, then layer bounded retry on top using conservative transient-error classification. Extend the existing `git-ai-test-git-shim` test binary so timeout and retry behavior can be reproduced deterministically in integration tests.

**Tech Stack:** Rust 2024, `std::process::Command`, `std::process::Child`, existing `GitAiError`, existing `ConfigPatch`/test config patch flow, existing `git-ai-test-git-shim` test binary, Cargo test harness

---

### Task 1: Add deterministic test config support for git-path overrides

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs` existing `#[cfg(test)]` module (or add one if missing)

- [ ] **Step 1: Add `git_path` to `ConfigPatch`**

Add this field near the other optional patch fields in `src/config.rs`:

```rust
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_prompts_in_repositories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_oss_disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_version_checks: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_auto_updates: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_storage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_store: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_attributes: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature_flags: Option<serde_json::Value>,
}
```

- [ ] **Step 2: Apply `git_path` when building the runtime config in tests**

In the existing config patch application block in `src/config.rs`, add logic matching this shape:

```rust
#[cfg(any(test, feature = "test-support"))]
if let Ok(patch_json) = env::var("GIT_AI_TEST_CONFIG_PATCH") {
    if let Ok(patch) = serde_json::from_str::<ConfigPatch>(&patch_json) {
        if let Some(git_path) = patch.git_path {
            config.git_path = git_path;
        }

        if let Some(exclude_prompts) = patch.exclude_prompts_in_repositories {
            config.exclude_prompts_in_repositories = exclude_prompts
                .iter()
                .map(|value| Pattern::new(value).expect("valid exclude pattern"))
                .collect();
        }

        if let Some(disabled) = patch.telemetry_oss_disabled {
            config.telemetry_oss_disabled = disabled;
        }

        if let Some(disable_version_checks) = patch.disable_version_checks {
            config.disable_version_checks = disable_version_checks;
        }

        if let Some(disable_auto_updates) = patch.disable_auto_updates {
            config.disable_auto_updates = disable_auto_updates;
        }

        if let Some(prompt_storage) = patch.prompt_storage {
            config.prompt_storage = prompt_storage;
        }

        if let Some(notes_store) = patch.notes_store {
            config.notes_store = notes_store;
        }

        if let Some(custom_attributes) = patch.custom_attributes {
            config.custom_attributes = custom_attributes;
        }

        if let Some(feature_flags) = patch.feature_flags {
            apply_feature_flag_patch(&mut config.feature_flags, feature_flags);
        }
    }
}
```

Do not change non-test runtime config precedence; only extend the existing test-only patch flow.

- [ ] **Step 3: Add a config patch regression test**

Add a focused test in `src/config.rs` that proves `git_path` is patched:

```rust
#[test]
fn test_config_patch_overrides_git_path() {
    let patch = ConfigPatch {
        git_path: Some("/tmp/fake-git".to_string()),
        ..Default::default()
    };
    std::env::set_var(
        "GIT_AI_TEST_CONFIG_PATCH",
        serde_json::to_string(&patch).expect("serialize patch"),
    );

    let config = Config::fresh();
    assert_eq!(config.git_cmd(), "/tmp/fake-git");

    std::env::remove_var("GIT_AI_TEST_CONFIG_PATCH");
}
```

If `Config::fresh()` reads other process-global state that can race, mark the test `#[serial_test::serial]`.

- [ ] **Step 4: Run the focused config test**

Run: `cargo test --package git-ai config_patch_overrides_git_path -- --nocapture`

Expected: the new config-path override test passes.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "test(config): allow git_path overrides in config patches"
```

---

### Task 2: Extend the git test shim with deterministic failure modes

**Files:**
- Modify: `src/bin/git-ai-test-git-shim.rs`
- Test: `tests/integration/git_exec_retry.rs`

- [ ] **Step 1: Add shim env constants and invocation-state helpers**

At the top of `src/bin/git-ai-test-git-shim.rs`, add constants and helpers for the new deterministic modes:

```rust
const SHIM_MODE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_MODE";
const SHIM_STATE_FILE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_STATE_FILE";
const SHIM_SLEEP_MS_ENV: &str = "GIT_AI_TEST_GIT_SHIM_SLEEP_MS";
const SHIM_STDERR_ENV: &str = "GIT_AI_TEST_GIT_SHIM_STDERR";
const SHIM_EXIT_CODE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_EXIT_CODE";
const SHIM_PID_FILE_ENV: &str = "GIT_AI_TEST_GIT_SHIM_PID_FILE";
const SHIM_REAL_GIT_ENV: &str = "GIT_AI_TEST_GIT_SHIM_REAL_GIT";

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
        .unwrap_or_else(|_| "fatal: Unable to create '/tmp/repo/.git/index.lock': File exists.".to_string())
}
```

- [ ] **Step 2: Add pid-file writing and stderr failure helpers**

Add these helpers below the existing logging helpers:

```rust
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
```

- [ ] **Step 3: Add pass-through and mode dispatch support**

Replace the current direct `exec_target(...)` use in `main()` with a dispatch flow that:

```rust
let mode = env::var(SHIM_MODE_ENV).unwrap_or_else(|_| "pass_through".to_string());
let state_path = env::var(SHIM_STATE_FILE_ENV).ok();
let invocation = read_and_increment_invocation(state_path.as_deref());
maybe_write_pid_file();

match mode.as_str() {
    "pass_through" => exec_target(&target, &effective_argv, use_git_ai_wrapper_mode),
    "sleep_always" => {
        std::thread::sleep(configured_sleep_duration());
        exec_target(&target, &effective_argv, use_git_ai_wrapper_mode)
    }
    "sleep_then_success_once" => {
        if invocation == 0 {
            std::thread::sleep(configured_sleep_duration());
        }
        exec_target(&target, &effective_argv, use_git_ai_wrapper_mode)
    }
    "stderr_once_then_success" => {
        if invocation == 0 {
            fail_with_stderr(&configured_stderr(), configured_exit_code());
        }
        exec_target(&target, &effective_argv, use_git_ai_wrapper_mode)
    }
    other => {
        eprintln!("unknown shim mode: {other}");
        std::process::exit(2);
    }
}
```

Keep the existing test-sync logging behavior intact; only add the mode dispatch around the final execution step.

- [ ] **Step 4: Add a focused shim-mode integration test file**

Create `tests/integration/git_exec_retry.rs` with a smoke test that proves the shim modes are wired. Start with this skeleton:

```rust
use crate::repos::test_repo::{real_git_executable, TestRepo};
use std::path::PathBuf;
use std::process::Command;

fn shim_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai-test-git-shim"))
}

#[test]
fn test_git_exec_shim_passthrough_mode_smoke() {
    let repo = TestRepo::new();
    let output = Command::new(shim_binary())
        .arg("--version")
        .env("GIT_AI_TEST_GIT_SHIM_TARGET", real_git_executable())
        .env("GIT_AI_TEST_GIT_SHIM_REAL_GIT", real_git_executable())
        .env("GIT_AI_TEST_GIT_SHIM_MODE", "pass_through")
        .current_dir(repo.path())
        .output()
        .expect("run shim");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("git version"));
}
```

- [ ] **Step 5: Run the new shim smoke test**

Run: `cargo test --package git-ai --test git_exec_retry -- test_git_exec_shim_passthrough_mode_smoke --nocapture`

Expected: the shim smoke test passes and prints no unknown-mode errors.

- [ ] **Step 6: Commit**

```bash
git add src/bin/git-ai-test-git-shim.rs tests/integration/git_exec_retry.rs
git commit -m "test(git): add deterministic git shim retry modes"
```

---

### Task 3: Refactor repository git execution into a shared request/policy flow

**Files:**
- Modify: `src/git/repository.rs`
- Test: `src/git/repository.rs` internal test module (new if needed)

- [ ] **Step 1: Add shared execution constants and request/policy structs**

Near the existing `EXEC_GIT_TIMEOUT` constant in `src/git/repository.rs`, add:

```rust
const EXEC_GIT_MAX_ATTEMPTS: usize = 3;
const EXEC_GIT_RETRY_BACKOFFS: &[Duration] = &[
    Duration::from_millis(100),
    Duration::from_millis(300),
];
const EXEC_GIT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const EXEC_GIT_TIMEOUT_STDERR: &str = "Command timed out";

struct GitExecRequest {
    args: Vec<String>,
    profile: InternalGitProfile,
    stdin_data: Option<Vec<u8>>,
    env_overrides: Vec<(String, String)>,
    timeout: Duration,
}

struct GitExecPolicy {
    max_attempts: usize,
    backoff_delays: &'static [Duration],
    retry_on_timeout: bool,
    retry_on_transient_error: bool,
}

struct PreparedGitCommand {
    cmd: Command,
    effective_args: Vec<String>,
}
```

- [ ] **Step 2: Add default policy and command-building helpers**

Below the existing profile helper functions, add:

```rust
fn default_git_exec_policy() -> GitExecPolicy {
    GitExecPolicy {
        max_attempts: EXEC_GIT_MAX_ATTEMPTS,
        backoff_delays: EXEC_GIT_RETRY_BACKOFFS,
        retry_on_timeout: true,
        retry_on_transient_error: true,
    }
}

fn build_git_command(request: &GitExecRequest) -> PreparedGitCommand {
    let effective_args = args_with_internal_git_profile(
        &args_with_disabled_hooks_if_needed(&request.args),
        request.profile,
    );

    let mut cmd = Command::new(config::Config::get().git_cmd());
    cmd.args(&effective_args);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    for (k, v) in &request.env_overrides {
        cmd.env(k, v);
    }

    cmd.env_remove("GIT_EXTERNAL_DIFF");
    cmd.env_remove("GIT_DIFF_OPTS");

    #[cfg(windows)]
    {
        if !is_interactive_terminal() {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }

    if is_debug_enabled() {
        tracing::debug!("[exec_git] cmd = {:?}", cmd);
        cmd.env("GIT_TRACE", "1");
        cmd.env("GIT_TRACE2", "1");
    }

    PreparedGitCommand { cmd, effective_args }
}
```

Use `Stdio::piped()` for stdout/stderr on every path so one execution flow can collect output consistently.

- [ ] **Step 3: Implement the real spawn-based single-execution path**

Replace the current `cmd.output()`/thread-channel timeout model with helpers shaped like this:

```rust
fn write_stdin_in_background(
    child: &mut std::process::Child,
    stdin_data: &[u8],
) -> Option<std::thread::JoinHandle<std::io::Result<()>>> {
    let stdin = child.stdin.take()?;
    let data = stdin_data.to_vec();
    Some(std::thread::spawn(move || {
        use std::io::Write;
        let mut stdin = stdin;
        stdin.write_all(&data)
    }))
}

fn finalize_stdin_writer(
    handle: Option<std::thread::JoinHandle<std::io::Result<()>>>,
) -> Result<(), GitAiError> {
    if let Some(handle) = handle {
        let result = handle.join().expect("stdin writer thread panicked");
        if let Err(e) = result
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(GitAiError::IoError(e));
        }
    }
    Ok(())
}
```

Then add a `run_git_once(request: &GitExecRequest) -> Result<Output, GitAiError>` helper that:

1. calls `build_git_command()`
2. `spawn()`s the child
3. starts the stdin writer thread when `stdin_data` exists
4. waits for output on a separate worker thread using `child.wait_with_output()`
5. uses a channel `recv_timeout(request.timeout)` in the parent thread to enforce the timeout
6. on timeout, calls `child.kill()` and `child.wait()` (or equivalent shared-handle kill/reap path), then returns:

```rust
Err(GitAiError::GitCliError {
    code: Some(1),
    stderr: EXEC_GIT_TIMEOUT_STDERR.to_string(),
    args: effective_args.clone(),
})
```

7. finalizes the stdin writer thread on successful completion
8. preserves the existing slow-command logging and tracing fields

The worker-thread implementation must keep a parent-owned kill handle so timeout means the process is actually terminated, not just abandoned.

- [ ] **Step 4: Add conservative retry classification helpers**

Below `run_git_once`, add these helpers:

```rust
fn is_timeout_error(err: &GitAiError) -> bool {
    matches!(
        err,
        GitAiError::GitCliError { stderr, .. } if stderr == EXEC_GIT_TIMEOUT_STDERR
    )
}

fn is_retryable_io_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
            | std::io::ErrorKind::WouldBlock
    )
}

fn is_retryable_git_cli_error(code: Option<i32>, stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    let _ = code;

    stderr.contains("index.lock")
        || (stderr.contains(".lock") && stderr.contains("unable to create"))
        || stderr.contains("resource temporarily unavailable")
        || stderr.contains("timed out")
}

fn is_retryable_error(err: &GitAiError) -> bool {
    match err {
        GitAiError::IoError(io_err) => is_retryable_io_error(io_err),
        GitAiError::GitCliError { code, stderr, .. } => {
            is_timeout_error(err) || is_retryable_git_cli_error(*code, stderr)
        }
        _ => false,
    }
}
```

- [ ] **Step 5: Add the retry loop and route the public helpers through it**

Implement `run_git_with_retry(request: &GitExecRequest, policy: &GitExecPolicy)` like this:

```rust
fn run_git_with_retry(
    request: &GitExecRequest,
    policy: &GitExecPolicy,
) -> Result<Output, GitAiError> {
    let started = Instant::now();

    for attempt in 1..=policy.max_attempts {
        match run_git_once(request) {
            Ok(output) => {
                tracing::debug!(
                    "[exec_git retry] succeeded on attempt {}/{} after {}ms",
                    attempt,
                    policy.max_attempts,
                    started.elapsed().as_millis()
                );
                return Ok(output);
            }
            Err(err) => {
                let retryable = is_retryable_error(&err)
                    && ((policy.retry_on_timeout && is_timeout_error(&err))
                        || (policy.retry_on_transient_error && !is_timeout_error(&err)));

                if !retryable || attempt == policy.max_attempts {
                    return Err(err);
                }

                let delay = policy
                    .backoff_delays
                    .get(attempt - 1)
                    .copied()
                    .or_else(|| policy.backoff_delays.last().copied())
                    .unwrap_or(Duration::ZERO);

                tracing::debug!(
                    "[exec_git retry] retrying attempt {}/{} after {}ms due to: {}",
                    attempt + 1,
                    policy.max_attempts,
                    delay.as_millis(),
                    err
                );
                std::thread::sleep(delay);
            }
        }
    }

    Err(GitAiError::Generic("git retry loop exited unexpectedly".to_string()))
}
```

Then rewrite these public functions to build `GitExecRequest` values and use the new shared flow without changing their signatures:

- `exec_git_allow_nonzero_with_profile`
- `exec_git_with_profile`
- `exec_git_stdin_with_profile`
- `exec_git_stdin_with_env_with_profile`
- `exec_git_with_timeout_internal`

`exec_git_with_profile` and the stdin variants must still convert non-zero statuses into `GitAiError::GitCliError` exactly like the current implementation does. `exec_git_allow_nonzero_with_profile` must still return `Ok(Output)` for non-zero exit statuses.

- [ ] **Step 6: Add internal unit tests for retry classification**

At the bottom of `src/git/repository.rs`, add tests like:

```rust
#[test]
fn test_retryable_git_cli_error_detects_index_lock() {
    assert!(is_retryable_git_cli_error(
        Some(128),
        "fatal: Unable to create '/repo/.git/index.lock': File exists."
    ));
}

#[test]
fn test_retryable_git_cli_error_rejects_bad_revision() {
    assert!(!is_retryable_git_cli_error(
        Some(128),
        "fatal: bad revision 'abc'"
    ));
}

#[test]
fn test_is_timeout_error_matches_timeout_sentinel() {
    let err = GitAiError::GitCliError {
        code: Some(1),
        stderr: EXEC_GIT_TIMEOUT_STDERR.to_string(),
        args: vec!["status".to_string()],
    };
    assert!(is_timeout_error(&err));
}
```

- [ ] **Step 7: Run focused repository unit tests**

Run: `cargo test --package git-ai repository:: -- --nocapture`

Expected: the new retry-classification tests pass and no existing repository unit tests regress.

- [ ] **Step 8: Commit**

```bash
git add src/git/repository.rs
git commit -m "feat(git): add shared git execution retry and timeout flow"
```

---

### Task 4: Add integration tests for retry, timeout, and process cleanup

**Files:**
- Modify: `tests/integration/git_exec_retry.rs`
- Modify: `tests/integration/repos/test_repo.rs`

- [ ] **Step 1: Add test helpers for shim-backed git config patches**

In `tests/integration/git_exec_retry.rs`, add helpers like:

```rust
use git_ai::config::ConfigPatch;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn shim_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai-test-git-shim"))
}

fn shim_config_patch() -> String {
    serde_json::to_string(&ConfigPatch {
        git_path: Some(shim_binary().to_string_lossy().to_string()),
        ..Default::default()
    })
    .expect("serialize shim patch")
}

fn write_state_file(prefix: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "git-ai-shim-state-{}-{}-{}.txt",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, "0\n").expect("write state file");
    path
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
```

- [ ] **Step 2: Add a timeout-then-success integration test**

Add this test to `tests/integration/git_exec_retry.rs`:

```rust
#[test]
#[serial_test::serial]
fn test_git_retry_recovers_from_single_timeout() {
    let repo = TestRepo::new();
    let state_file = write_state_file("timeout-once");

    let result = repo.git_with_env(
        &["status", "--porcelain"],
        &[
            ("GIT_AI_TEST_CONFIG_PATCH", &shim_config_patch()),
            ("GIT_AI_TEST_GIT_SHIM_MODE", "sleep_then_success_once"),
            (
                "GIT_AI_TEST_GIT_SHIM_TARGET",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_REAL_GIT",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.to_str().expect("state file path"),
            ),
            ("GIT_AI_TEST_GIT_SHIM_SLEEP_MS", "15000"),
        ],
    );

    assert!(result.is_ok(), "expected retry to recover from a single timeout: {result:?}");
    let count = std::fs::read_to_string(&state_file).expect("read state file");
    assert_eq!(count.trim(), "2");
}
```

If the production timeout is too long for this test, add a test-only timeout override in `repository.rs` first, using the same thread-local override style already used in `bash_tool.rs`, then use that override in this test.

- [ ] **Step 3: Add an always-timeout integration test**

Add this test:

```rust
#[test]
#[serial_test::serial]
fn test_git_retry_fails_after_repeated_timeouts() {
    let repo = TestRepo::new();
    let state_file = write_state_file("timeout-always");

    let result = repo.git_with_env(
        &["status", "--porcelain"],
        &[
            ("GIT_AI_TEST_CONFIG_PATCH", &shim_config_patch()),
            ("GIT_AI_TEST_GIT_SHIM_MODE", "sleep_always"),
            (
                "GIT_AI_TEST_GIT_SHIM_TARGET",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_REAL_GIT",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.to_str().expect("state file path"),
            ),
            ("GIT_AI_TEST_GIT_SHIM_SLEEP_MS", "15000"),
        ],
    );

    let err = result.expect_err("expected repeated timeouts to fail");
    assert!(err.contains("Command timed out"), "unexpected stderr: {err}");
    let count = std::fs::read_to_string(&state_file).expect("read state file");
    assert_eq!(count.trim(), "3");
}
```

- [ ] **Step 4: Add transient-stderr success and non-retryable failure tests**

Add these two tests:

```rust
#[test]
#[serial_test::serial]
fn test_git_retry_recovers_from_retryable_stderr() {
    let repo = TestRepo::new();
    let state_file = write_state_file("stderr-once");

    let result = repo.git_with_env(
        &["status", "--porcelain"],
        &[
            ("GIT_AI_TEST_CONFIG_PATCH", &shim_config_patch()),
            ("GIT_AI_TEST_GIT_SHIM_MODE", "stderr_once_then_success"),
            (
                "GIT_AI_TEST_GIT_SHIM_TARGET",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_REAL_GIT",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.to_str().expect("state file path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STDERR",
                "fatal: Unable to create '/repo/.git/index.lock': File exists.",
            ),
            ("GIT_AI_TEST_GIT_SHIM_EXIT_CODE", "128"),
        ],
    );

    assert!(result.is_ok(), "expected retry to recover from retryable stderr: {result:?}");
    let count = std::fs::read_to_string(&state_file).expect("read state file");
    assert_eq!(count.trim(), "2");
}

#[test]
#[serial_test::serial]
fn test_git_retry_does_not_retry_non_retryable_stderr() {
    let repo = TestRepo::new();
    let state_file = write_state_file("stderr-bad-revision");

    let result = repo.git_with_env(
        &["status", "--porcelain"],
        &[
            ("GIT_AI_TEST_CONFIG_PATCH", &shim_config_patch()),
            ("GIT_AI_TEST_GIT_SHIM_MODE", "stderr_once_then_success"),
            (
                "GIT_AI_TEST_GIT_SHIM_TARGET",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_REAL_GIT",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.to_str().expect("state file path"),
            ),
            ("GIT_AI_TEST_GIT_SHIM_STDERR", "fatal: bad revision 'abc'"),
            ("GIT_AI_TEST_GIT_SHIM_EXIT_CODE", "128"),
        ],
    );

    let err = result.expect_err("expected bad revision to fail without retry");
    assert!(err.contains("bad revision"), "unexpected stderr: {err}");
    let count = std::fs::read_to_string(&state_file).expect("read state file");
    assert_eq!(count.trim(), "1");
}
```

- [ ] **Step 5: Add a timeout-cleanup test**

Add this Unix-only test:

```rust
#[cfg(unix)]
#[test]
#[serial_test::serial]
fn test_git_timeout_kills_hung_shim_process() {
    let repo = TestRepo::new();
    let state_file = write_state_file("timeout-kill");
    let pid_file = std::env::temp_dir().join(format!(
        "git-ai-shim-pid-{}-{}.txt",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));

    let result = repo.git_with_env(
        &["status", "--porcelain"],
        &[
            ("GIT_AI_TEST_CONFIG_PATCH", &shim_config_patch()),
            ("GIT_AI_TEST_GIT_SHIM_MODE", "sleep_always"),
            (
                "GIT_AI_TEST_GIT_SHIM_TARGET",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_REAL_GIT",
                real_git_executable().to_str().expect("real git path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_STATE_FILE",
                state_file.to_str().expect("state file path"),
            ),
            (
                "GIT_AI_TEST_GIT_SHIM_PID_FILE",
                pid_file.to_str().expect("pid file path"),
            ),
            ("GIT_AI_TEST_GIT_SHIM_SLEEP_MS", "15000"),
        ],
    );

    assert!(result.is_err(), "expected timeout path to fail");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("read pid file")
        .trim()
        .parse()
        .expect("parse pid");
    assert!(!process_exists(pid), "timed out shim process {pid} should have been killed");
}
```

- [ ] **Step 6: Run the new integration test file**

Run: `cargo test --package git-ai --test git_exec_retry -- --nocapture`

Expected: all timeout/retry integration tests pass deterministically.

- [ ] **Step 7: Commit**

```bash
git add tests/integration/git_exec_retry.rs tests/integration/repos/test_repo.rs
git commit -m "test(git): cover retry and timeout execution behavior"
```

---

### Task 5: Run focused verification and cleanup

**Files:**
- Modify: `src/git/repository.rs` (if verification exposes issues)
- Modify: `src/bin/git-ai-test-git-shim.rs` (if verification exposes issues)
- Modify: `tests/integration/git_exec_retry.rs` (if verification exposes issues)

- [ ] **Step 1: Format the touched files**

Run: `cargo fmt --all`

Expected: formatting completes without error.

- [ ] **Step 2: Run the focused verification set**

Run these commands in order:

```bash
cargo test --package git-ai repository:: -- --nocapture
cargo test --package git-ai --test git_exec_retry -- --nocapture
cargo test --package git-ai --test bash_tool_timeouts -- --nocapture
```

Expected: all three test commands pass.

- [ ] **Step 3: Run a final targeted build check**

Run: `cargo build --features test-support`

Expected: build completes successfully.

- [ ] **Step 4: Review the final diff for scope control**

Run:

```bash
git diff -- src/config.rs src/git/repository.rs src/bin/git-ai-test-git-shim.rs tests/integration/git_exec_retry.rs tests/integration/repos/test_repo.rs
```

Expected: the diff is limited to config patch support, git execution retry/timeout flow, shim behavior, and related tests.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/git/repository.rs src/bin/git-ai-test-git-shim.rs tests/integration/git_exec_retry.rs tests/integration/repos/test_repo.rs
git commit -m "fix(git): retry hung and transient internal git executions"
```
