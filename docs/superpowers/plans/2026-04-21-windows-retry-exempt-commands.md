# Windows Retry Exempt Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Exempt specific git commands (log, pull, commit, fetch, push, rebase, merge, stash, config --list) from the Windows `proxy_to_git` timeout+retry loop, making them wait indefinitely like the Unix path.

**Architecture:** Add a `#[cfg(windows)]` predicate `is_command_exempt_from_retry` that uses the existing `parse_git_cli_args` to extract the command name and subcommand flags. Modify `wait_for_git_with_retry_windows` to accept an `exempt_from_retry` flag — when true, skip the poll-timeout-retry loop and use plain `child.wait()`. Wire the flag in `proxy_to_git`'s Windows branch.

**Tech Stack:** Rust, `#[cfg(windows)]` conditional compilation, existing `parse_git_cli_args` from `src/git/cli_parser.rs`.

**Spec:** `docs/superpowers/specs/2026-04-21-windows-retry-exempt-commands-design.md`

---

### Task 1: Add `is_command_exempt_from_retry` predicate

**Files:**
- Modify: `src/commands/git_handlers.rs` — add new function after the existing `parse_git_proxy_retry_count` (around line 1253)

- [ ] **Step 1: Add the predicate function**

Insert after the `parse_git_proxy_retry_count` function (after line 1253), before the `exit_with_status` function:

```rust
/// Returns true if the given git command should be exempt from the Windows
/// proxy timeout+retry loop. Exempt commands are either interactive (commit,
/// rebase, merge), long-running (log, fetch, pull, push), or dangerous to
/// retry (commit, push). Uses parse_git_cli_args to extract the command
/// regardless of global flag positioning.
#[cfg(windows)]
fn is_command_exempt_from_retry(args: &[String]) -> bool {
    let parsed = parse_git_cli_args(args);
    let Some(command) = parsed.command.as_deref() else {
        return false;
    };

    match command {
        "log" | "pull" | "commit" | "fetch" | "push" | "rebase" | "merge"
        | "stash" => true,
        "config" => parsed.command_args.iter().any(|a| a == "--list" || a == "-l"),
        _ => false,
    }
}
```

This reuses the existing `parse_git_cli_args` which already handles global flags (`-C`, `-c`, `--git-dir`, etc.) and extracts `command` and `command_args` correctly. No custom arg-parsing needed.

- [ ] **Step 2: Run `cargo check` to verify compilation**

Run: `cargo check`
Expected: Clean compilation (the function is `#[cfg(windows)]` so it won't be checked on macOS/Linux — but the types and imports must be correct).

Note: On macOS/Linux, `cargo check` will pass because `#[cfg(windows)]` code is excluded. To verify the Windows code compiles, we rely on CI or cross-compilation. The function is simple enough (uses only existing public APIs) that this is safe.

---

### Task 2: Add `exempt_from_retry` parameter to `wait_for_git_with_retry_windows`

**Files:**
- Modify: `src/commands/git_handlers.rs:1066-1115` — `wait_for_git_with_retry_windows` function

- [ ] **Step 1: Add parameter and early-return for exempt commands**

Change the function signature at line 1066 from:

```rust
#[cfg(windows)]
fn wait_for_git_with_retry_windows(
    mut child: std::process::Child,
    args: &[String],
    child_hooks_path_override: Option<&str>,
    wrapper_invocation_id: Option<&str>,
    suppress_trace2: bool,
) -> std::process::ExitStatus {
```

to:

```rust
#[cfg(windows)]
fn wait_for_git_with_retry_windows(
    mut child: std::process::Child,
    args: &[String],
    child_hooks_path_override: Option<&str>,
    wrapper_invocation_id: Option<&str>,
    suppress_trace2: bool,
    exempt_from_retry: bool,
) -> std::process::ExitStatus {
    if exempt_from_retry {
        return match child.wait() {
            Ok(status) => status,
            Err(e) => {
                eprintln!("Failed to wait for git process: {}", e);
                std::process::exit(1);
            }
        };
    }

```

The rest of the existing function body (the `max_retries`, `timeout`, retry loop) remains unchanged — it only runs when `exempt_from_retry` is `false`.

- [ ] **Step 2: Run `cargo check`**

Run: `cargo check`
Expected: Compilation error at the call site in `proxy_to_git` because the function now expects 6 arguments instead of 5. This is expected — we fix it in Task 3.

---

### Task 3: Wire `exempt_from_retry` in `proxy_to_git` Windows branch

**Files:**
- Modify: `src/commands/git_handlers.rs:1025-1038` — the `#[cfg(windows)]` block inside `proxy_to_git`

- [ ] **Step 1: Detect exempt command and pass flag to wait function**

Change lines 1025-1038 from:

```rust
            #[cfg(windows)]
            {
                let status = wait_for_git_with_retry_windows(
                    child,
                    args,
                    child_hooks_path_override,
                    wrapper_invocation_id,
                    suppress_trace2,
                );
                if exit_on_completion {
                    exit_with_status(status);
                }
                status
            }
```

to:

```rust
            #[cfg(windows)]
            {
                let exempt = is_command_exempt_from_retry(args);
                let status = wait_for_git_with_retry_windows(
                    child,
                    args,
                    child_hooks_path_override,
                    wrapper_invocation_id,
                    suppress_trace2,
                    exempt,
                );
                if exit_on_completion {
                    exit_with_status(status);
                }
                status
            }
```

- [ ] **Step 2: Run `cargo check`**

Run: `cargo check`
Expected: Clean compilation. All call sites now match.

- [ ] **Step 3: Run `cargo clippy`**

Run: `cargo clippy`
Expected: No new warnings related to our changes.

---

### Task 4: Add unit tests for `is_command_exempt_from_retry`

**Files:**
- Modify: `src/commands/git_handlers.rs` — add tests inside the existing `#[cfg(test)] mod tests` block (starts at line 1297)

- [ ] **Step 1: Add test module for the Windows-only predicate**

Since `is_command_exempt_from_retry` is `#[cfg(windows)]`, we need `#[cfg(windows)]` on the tests too. Add at the end of the `mod tests` block, before the closing `}`:

```rust
    #[cfg(windows)]
    mod windows_retry_exempt {
        use super::*;

        #[test]
        fn exempt_commands_are_detected() {
            for cmd in &[
                "log", "pull", "commit", "fetch", "push", "rebase", "merge",
                "stash",
            ] {
                let args: Vec<String> = vec![cmd.to_string()];
                assert!(
                    is_command_exempt_from_retry(&args),
                    "{cmd} should be exempt from retry"
                );
            }
        }

        #[test]
        fn non_exempt_commands_are_not_detected() {
            for cmd in &["status", "diff", "rev-parse", "blame", "init", "add"] {
                let args: Vec<String> = vec![cmd.to_string()];
                assert!(
                    !is_command_exempt_from_retry(&args),
                    "{cmd} should NOT be exempt from retry"
                );
            }
        }

        #[test]
        fn config_list_is_exempt() {
            let args: Vec<String> = vec!["config".to_string(), "--list".to_string()];
            assert!(is_command_exempt_from_retry(&args));

            let args: Vec<String> = vec!["config".to_string(), "-l".to_string()];
            assert!(is_command_exempt_from_retry(&args));
        }

        #[test]
        fn config_without_list_is_not_exempt() {
            let args: Vec<String> = vec!["config".to_string(), "user.name".to_string()];
            assert!(!is_command_exempt_from_retry(&args));

            let args: Vec<String> = vec!["config".to_string()];
            assert!(!is_command_exempt_from_retry(&args));
        }

        #[test]
        fn global_flags_before_command_are_handled() {
            let args: Vec<String> = vec![
                "-C".to_string(),
                "/some/path".to_string(),
                "commit".to_string(),
            ];
            assert!(is_command_exempt_from_retry(&args));

            let args: Vec<String> = vec![
                "-c".to_string(),
                "core.bare=false".to_string(),
                "push".to_string(),
            ];
            assert!(is_command_exempt_from_retry(&args));
        }

        #[test]
        fn empty_and_unknown_args_return_false() {
            let args: Vec<String> = vec![];
            assert!(!is_command_exempt_from_retry(&args));

            let args: Vec<String> = vec!["--help".to_string()];
            assert!(!is_command_exempt_from_retry(&args));
        }

        #[test]
        fn config_list_with_other_flags_is_exempt() {
            let args: Vec<String> = vec![
                "config".to_string(),
                "--list".to_string(),
                "--show-origin".to_string(),
            ];
            assert!(is_command_exempt_from_retry(&args));
        }
    }
```

- [ ] **Step 2: Run `cargo check`**

Run: `cargo check`
Expected: Clean compilation (on Windows) or no change (on Unix, since tests are `#[cfg(windows)]`).

Note: These tests only compile and run on Windows. On macOS/Linux CI, they are excluded by `#[cfg(windows)]`. The function itself is also `#[cfg(windows)]`, so this is consistent.

---

### Task 5: Final verification

- [ ] **Step 1: Run `cargo clippy` on the whole project**

Run: `cargo clippy`
Expected: No new warnings.

- [ ] **Step 2: Run `cargo fmt -- --check`**

Run: `cargo fmt -- --check`
Expected: No formatting issues.

- [ ] **Step 3: Run existing tests**

Run: `cargo test`
Expected: All existing tests pass. No regressions.

- [ ] **Step 4: Commit**

```bash
git add src/commands/git_handlers.rs docs/superpowers/specs/2026-04-21-windows-retry-exempt-commands-design.md docs/superpowers/plans/2026-04-21-windows-retry-exempt-commands.md
git commit -m "feat(windows): exempt interactive/long-running commands from proxy retry

Commands log, pull, commit, fetch, push, rebase, merge, stash, and
config --list are now exempt from the Windows proxy_to_git timeout+retry
loop. These commands wait indefinitely (matching Unix behavior) because
they are either interactive, network-dependent, or dangerous to retry.

Exempt commands use plain child.wait() instead of the poll-timeout-retry
loop in wait_for_git_with_retry_windows."
```
