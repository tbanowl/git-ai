# Windows proxy_to_git Retry Exempt Commands

**Date**: 2026-04-21
**Status**: Approved

## Problem

The Windows `proxy_to_git` path has a 20-second timeout with 1 retry for all git commands. This is problematic for commands that:

- Are **long-running** by nature (`log`, `fetch`, `pull`, `push` on large repos or slow networks)
- Are **interactive** (`commit` opens an editor, `pull` may prompt for credentials)
- Are **dangerous to retry** (`commit`, `push` — retrying could cause duplicate operations)
- Involve **conflict resolution** (`rebase`, `merge`, `stash`)

Unix is unaffected — it uses plain `child.wait()` with no timeout or retry.

## Design

### Approach

Add a command-based predicate `is_command_exempt_from_retry()` that identifies commands which should skip timeout/retry on Windows. When a command is exempt, `wait_for_git_with_retry_windows` uses plain `child.wait()` instead of the poll-timeout-retry loop.

### Exempt Commands

| Command | Reason |
|---------|--------|
| `log` | Can take minutes on large repos |
| `pull` | Network-dependent, interactive credentials |
| `commit` | Interactive (opens editor), retrying is dangerous |
| `fetch` | Network-dependent, can be slow |
| `push` | Network-dependent, retrying is dangerous (duplicate push) |
| `rebase` | Interactive, multi-step, long-running |
| `merge` | Interactive (conflict resolution), slow on large repos |
| `stash` | Can be slow with large working trees |
| `config --list` | Read-only, but classified as mutating by git-ai |

### Implementation

**File**: `src/commands/git_handlers.rs` only. No Unix code changes.

1. **New function** `is_command_exempt_from_retry(args: &[String]) -> bool` — `#[cfg(windows)]`
   - Parses args to find the git subcommand, skipping global flags that take a parameter (`-C`, `-c`, `--git-dir`, `--work-tree`, `--namespace`, `--super-prefix`) and standalone global flags (`--bare`, `--no-replace-objects`, `--literal-pathspecs`)
   - Returns `true` for exempt commands
   - Special case: `config` is exempt only when `--list` or `-l` is present

2. **Modified function** `wait_for_git_with_retry_windows` — add `exempt_from_retry: bool` parameter
   - When `true`: skip the retry loop, call `child.wait()` directly and return
   - When `false`: existing timeout+retry logic unchanged

3. **Wire in `proxy_to_git`** Windows branch — detect the command before entering the wait function and pass the flag

### Unix Behavior

**No changes**. Unix `proxy_to_git` continues to use `child.wait()` with no timeout, no retry.

### Testing

- Unit test `is_command_exempt_from_retry`:
  - All exempt commands return `true`
  - Non-exempt commands (`status`, `diff`, `rev-parse`, `blame`) return `false`
  - `config --list` returns `true`, `config user.name` returns `false`
  - Global flags before command are handled: `-C /foo commit` detects `commit`
  - Unknown/empty args return `false`
- Integration test: verify exempt command waits indefinitely (no timeout kill)

## Scope

- **In scope**: Windows `proxy_to_git` retry exemption
- **Out of scope**: Unix changes, `run_git_with_retry` in `repository.rs`, config/env var changes
