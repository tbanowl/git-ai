# Rebase Conflict Manual Commit Authorship Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve AI authorship when a rebase conflict is resolved by direct `git commit`, while preventing stale `REBASE_HEAD` from affecting later commits and defining REST rewrite semantics for server-side notes.

**Architecture:** Add a worktree-scoped `PendingRebasePick` state stored under `.git/ai`, created only when rebase conflict state is observed, and consumed atomically by the next validated commit. Rewrite side effects continue to use existing `CherryPickComplete`/`RebaseComplete` machinery, but the commit path never directly reads `REBASE_HEAD`. REST notes get a new `/worker/authorship_notes/rewrite` client contract so the server can atomically activate the rewritten note and supersede the old one.

**Tech Stack:** Rust 2024, serde JSON, git CLI subprocesses, existing `RepoStorage`, wrapper hooks, daemon coordinator, `task test`, `task fmt`, `task build`.

---

## File Structure

- Create `src/git/pending_rebase_pick.rs`
  - Owns the `PendingRebasePick` data model and worktree-scoped persistence helpers.
  - Exposes atomic-ish create/take/mark helpers used by wrapper hooks and daemon side effects.
- Modify `src/git/mod.rs`
  - Export the new `pending_rebase_pick` module.
- Modify `src/git/repo_storage.rs`
  - Add the `pending_rebase_pick` field and initialize its directory/file parent.
  - Add thin methods that delegate to `pending_rebase_pick` helpers where useful.
- Modify `src/commands/hooks/fetch_hooks.rs`
  - On paused `pull --rebase`, create pending state from the stopped source commit and current HEAD.
  - On non-conflict pull failure or successful non-conflict pull, clear/abort pending state for this operation.
- Modify `src/commands/hooks/rebase_hooks.rs`
  - On paused standalone rebase, create pending state.
  - On abort/skip, mark pending state aborted/skipped.
  - On successful `rebase --continue`, avoid duplicating a consumed `B -> D` mapping.
- Modify `src/commands/hooks/commit_hooks.rs`
  - Remove direct `REBASE_HEAD` read helper and current speculative `CherryPickComplete` branch.
  - Consume pending state after successful non-amend commit when parent validation passes.
- Modify `src/daemon.rs`
  - Remove direct `REBASE_HEAD`/`stopped-sha` reads in `CommitCreated` semantic side-effect generation.
  - Consume the same pending state API as the wrapper, so wrapper and daemon do not duplicate rewrites.
- Modify `src/api/types.rs`
  - Add request/response types for `AuthorshipNotesRewriteRequest`.
- Modify `src/api/authorship_notes.rs`
  - Add `ApiClient::authorship_notes_rewrite`.
- Modify `src/git/sync_authorship.rs`
  - Add helpers that build and send REST rewrite requests from rewrite mappings.
  - Keep normal notes push behavior unchanged.
- Modify `tests/integration/pull_rebase_ff.rs`
  - Keep the two existing regression tests.
  - Add rebase-continue-after-direct-commit and abort/skip pending cleanup tests.
- Modify `tests/rest_notes_sync.rs`
  - Add mock server assertions for the new rewrite endpoint, idempotent request shape, and conflict response parsing.

## Task 1: Confirm Red Regression And Remove Unsafe Production Attempt

**Files:**
- Modify: `src/commands/hooks/commit_hooks.rs`
- Modify: `src/daemon.rs`
- Test: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Run the stale `REBASE_HEAD` regression**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head NO_CAPTURE=true
```

Expected before fixes:

```text
FAILED
follow-up commit should not inherit Session B prompt metrics from stale REBASE_HEAD
left: 2
right: 0
```

- [ ] **Step 2: Remove direct `REBASE_HEAD` use from commit hook**

In `src/commands/hooks/commit_hooks.rs`, delete:

```rust
if !parsed_args.has_command_flag("--amend")
    && let (Some(orig), Some(sha), Some(rebase_head)) = (
        original_commit.clone(),
        new_sha.clone(),
        rebase_head_commit(repository),
    )
{
    let rebase_conflict_commit = RewriteLogEvent::cherry_pick_complete(
        crate::git::rewrite_log::CherryPickCompleteEvent::new(
            orig,
            sha.clone(),
            vec![rebase_head],
            vec![sha],
        ),
    );
    repository.handle_rewrite_log_event(
        rebase_conflict_commit,
        commit_author,
        supress_output,
        true,
    );
    crate::observability::spawn_background_flush();
    return;
}
```

Also delete:

```rust
fn rebase_head_commit(repo: &Repository) -> Option<String> {
    let rebase_head_path = repo.path().join("REBASE_HEAD");
    std::fs::read_to_string(rebase_head_path)
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|sha| crate::git::repo_state::is_valid_git_oid(sha))
}
```

- [ ] **Step 3: Remove direct `REBASE_HEAD` use from daemon**

In `src/daemon.rs`, delete:

```rust
fn resolve_rebase_current_source_for_worktree(worktree: &Path) -> Option<String> {
    let git_dir = git_dir_for_worktree(worktree)?;

    for candidate in [
        git_dir.join("REBASE_HEAD"),
        git_dir.join("rebase-merge").join("stopped-sha"),
        git_dir.join("rebase-apply").join("stopped-sha"),
    ] {
        if let Ok(contents) = fs::read_to_string(candidate)
            && let Some(oid) = contents
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
            && is_valid_oid(oid)
            && !is_zero_oid(oid)
        {
            return Some(oid.to_string());
        }
    }

    read_ref_oid_for_worktree(worktree, "REBASE_HEAD")
        .filter(|oid| is_valid_oid(oid) && !is_zero_oid(oid))
}
```

Then replace the current `CommitCreated` branch that calls
`resolve_rebase_current_source_for_worktree` with the original plain commit
event:

```rust
out.push(RewriteLogEvent::commit(base.clone(), new_head.clone()));
```

- [ ] **Step 4: Verify the direct-commit preservation test now fails**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_after_failed_push_commit_preserves_ai_notes NO_CAPTURE=true
```

Expected:

```text
FAILED
```

The exact failure should show that the manual conflict-resolution commit no
longer preserves Session B AI attribution. This confirms the pending-state
implementation is actually needed.

- [ ] **Step 5: Commit cleanup if working on an implementation branch**

Only commit if the user asked for commits during implementation. If committing:

```bash
git add src/commands/hooks/commit_hooks.rs src/daemon.rs
git commit -m "refactor: remove unsafe rebase head commit mapping"
```

## Task 2: Add Pending Rebase Pick Storage

**Files:**
- Create: `src/git/pending_rebase_pick.rs`
- Modify: `src/git/mod.rs`
- Modify: `src/git/repo_storage.rs`
- Unit tests: `src/git/pending_rebase_pick.rs`

- [ ] **Step 1: Write storage unit tests**

Create `src/git/pending_rebase_pick.rs` with the module imports and tests first:

```rust
use crate::error::GitAiError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingRebasePickStatus {
    Pending,
    Consumed,
    Aborted,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRebasePick {
    pub source_commit: String,
    pub expected_parent: String,
    pub original_head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onto_head: Option<String>,
    pub operation: String,
    pub status: PendingRebasePickStatus,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_by: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pick() -> PendingRebasePick {
        PendingRebasePick {
            source_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            expected_parent: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            original_head: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            onto_head: Some("cccccccccccccccccccccccccccccccccccccccc".to_string()),
            operation: "pull_rebase_conflict".to_string(),
            status: PendingRebasePickStatus::Pending,
            created_at_ms: 123,
            consumed_by: None,
        }
    }

    #[test]
    fn create_and_read_pending_pick_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");

        write_pending_rebase_pick(&path, &pick()).unwrap();
        let read = read_pending_rebase_pick(&path).unwrap().unwrap();

        assert_eq!(read, pick());
    }

    #[test]
    fn take_pending_pick_consumes_only_matching_parent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");
        write_pending_rebase_pick(&path, &pick()).unwrap();

        let missed = take_pending_rebase_pick_for_commit(
            &path,
            "dddddddddddddddddddddddddddddddddddddddd",
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        )
        .unwrap();
        assert!(missed.is_none());
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Pending
        );

        let consumed = take_pending_rebase_pick_for_commit(
            &path,
            "cccccccccccccccccccccccccccccccccccccccc",
            "dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap()
        .unwrap();
        assert_eq!(consumed.source_commit, pick().source_commit);

        let stored = read_pending_rebase_pick(&path).unwrap().unwrap();
        assert_eq!(stored.status, PendingRebasePickStatus::Consumed);
        assert_eq!(
            stored.consumed_by.as_deref(),
            Some("dddddddddddddddddddddddddddddddddddddddd")
        );
    }

    #[test]
    fn mark_pending_pick_aborted_and_skipped_are_persistent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");
        write_pending_rebase_pick(&path, &pick()).unwrap();

        mark_pending_rebase_pick_aborted(&path).unwrap();
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Aborted
        );

        write_pending_rebase_pick(&path, &pick()).unwrap();
        mark_pending_rebase_pick_skipped(&path).unwrap();
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Skipped
        );
    }
}
```

- [ ] **Step 2: Run the storage tests and verify compile failures**

Run:

```bash
task test TEST_FILTER=pending_rebase_pick NO_CAPTURE=true
```

Expected:

```text
FAILED
cannot find function `write_pending_rebase_pick`
cannot find function `read_pending_rebase_pick`
cannot find function `take_pending_rebase_pick_for_commit`
```

- [ ] **Step 3: Implement pending storage helpers**

Add the implementation below the structs in
`src/git/pending_rebase_pick.rs`:

```rust
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn pending_rebase_pick(
    source_commit: String,
    expected_parent: String,
    original_head: String,
    onto_head: Option<String>,
    operation: impl Into<String>,
) -> PendingRebasePick {
    PendingRebasePick {
        source_commit,
        expected_parent,
        original_head,
        onto_head,
        operation: operation.into(),
        status: PendingRebasePickStatus::Pending,
        created_at_ms: now_ms(),
        consumed_by: None,
    }
}

pub fn read_pending_rebase_pick(
    path: &Path,
) -> Result<Option<PendingRebasePick>, GitAiError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let pick = serde_json::from_str(&raw)?;
    Ok(Some(pick))
}

pub fn write_pending_rebase_pick(
    path: &Path,
    pick: &PendingRebasePick,
) -> Result<(), GitAiError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_vec_pretty(pick)?;
    fs::write(&tmp, raw)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn take_pending_rebase_pick_for_commit(
    path: &Path,
    pre_head: &str,
    new_commit: &str,
) -> Result<Option<PendingRebasePick>, GitAiError> {
    let Some(mut pick) = read_pending_rebase_pick(path)? else {
        return Ok(None);
    };
    if pick.status != PendingRebasePickStatus::Pending {
        return Ok(None);
    }
    if pick.expected_parent != pre_head {
        return Ok(None);
    }

    let consumed = pick.clone();
    pick.status = PendingRebasePickStatus::Consumed;
    pick.consumed_by = Some(new_commit.to_string());
    write_pending_rebase_pick(path, &pick)?;
    Ok(Some(consumed))
}

pub fn mark_pending_rebase_pick_aborted(path: &Path) -> Result<(), GitAiError> {
    if let Some(mut pick) = read_pending_rebase_pick(path)? {
        if pick.status == PendingRebasePickStatus::Pending {
            pick.status = PendingRebasePickStatus::Aborted;
            write_pending_rebase_pick(path, &pick)?;
        }
    }
    Ok(())
}

pub fn mark_pending_rebase_pick_skipped(path: &Path) -> Result<(), GitAiError> {
    if let Some(mut pick) = read_pending_rebase_pick(path)? {
        if pick.status == PendingRebasePickStatus::Pending {
            pick.status = PendingRebasePickStatus::Skipped;
            write_pending_rebase_pick(path, &pick)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Export module**

In `src/git/mod.rs`, add:

```rust
pub mod pending_rebase_pick;
```

- [ ] **Step 5: Wire path into repo storage**

In `src/git/repo_storage.rs`, add a field:

```rust
pub pending_rebase_pick: PathBuf,
```

In `RepoStorage::for_ai_dir`, initialize it:

```rust
let pending_rebase_pick_file = ai_dir.join("pending_rebase_pick.json");

let config = RepoStorage {
    ai_dir: ai_dir.to_path_buf(),
    repo_workdir: repo_workdir.to_path_buf(),
    working_logs: working_logs_dir,
    rewrite_log: rewrite_log_file,
    pending_rebase_pick: pending_rebase_pick_file,
    logs: logs_dir,
};
```

Add methods:

```rust
pub fn write_pending_rebase_pick(
    &self,
    pick: &crate::git::pending_rebase_pick::PendingRebasePick,
) -> Result<(), GitAiError> {
    crate::git::pending_rebase_pick::write_pending_rebase_pick(
        &self.pending_rebase_pick,
        pick,
    )
}

pub fn take_pending_rebase_pick_for_commit(
    &self,
    pre_head: &str,
    new_commit: &str,
) -> Result<Option<crate::git::pending_rebase_pick::PendingRebasePick>, GitAiError> {
    crate::git::pending_rebase_pick::take_pending_rebase_pick_for_commit(
        &self.pending_rebase_pick,
        pre_head,
        new_commit,
    )
}

pub fn mark_pending_rebase_pick_aborted(&self) -> Result<(), GitAiError> {
    crate::git::pending_rebase_pick::mark_pending_rebase_pick_aborted(
        &self.pending_rebase_pick,
    )
}

pub fn mark_pending_rebase_pick_skipped(&self) -> Result<(), GitAiError> {
    crate::git::pending_rebase_pick::mark_pending_rebase_pick_skipped(
        &self.pending_rebase_pick,
    )
}
```

- [ ] **Step 6: Run storage tests**

Run:

```bash
task test TEST_FILTER=pending_rebase_pick NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 7: Commit storage task**

If committing:

```bash
git add src/git/pending_rebase_pick.rs src/git/mod.rs src/git/repo_storage.rs
git commit -m "feat: add pending rebase pick storage"
```

## Task 3: Capture Pending State When Rebase Pauses

**Files:**
- Modify: `src/commands/hooks/fetch_hooks.rs`
- Modify: `src/commands/hooks/rebase_hooks.rs`
- Test: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Add helper functions to `fetch_hooks.rs`**

Near the bottom of `src/commands/hooks/fetch_hooks.rs`, add:

```rust
fn rebase_in_progress(repository: &Repository) -> bool {
    repository.path().join("rebase-merge").exists()
        || repository.path().join("rebase-apply").exists()
}

fn read_first_valid_oid(paths: &[std::path::PathBuf]) -> Option<String> {
    paths.iter().find_map(|path| {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| raw.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_string))
            .filter(|oid| crate::git::repo_state::is_valid_git_oid(oid))
    })
}

fn stopped_rebase_source_commit(repository: &Repository) -> Option<String> {
    read_first_valid_oid(&[
        repository.path().join("REBASE_HEAD"),
        repository.path().join("rebase-merge").join("stopped-sha"),
        repository.path().join("rebase-apply").join("stopped-sha"),
    ])
}

fn create_pending_rebase_pick_for_paused_pull(
    repository: &Repository,
    original_head: &str,
    onto_head: Option<String>,
) {
    let Some(source_commit) = stopped_rebase_source_commit(repository) else {
        tracing::debug!("pull rebase paused but no stopped source commit found");
        return;
    };
    let Some(expected_parent) = repository.head().ok().and_then(|head| head.target().ok()) else {
        tracing::debug!("pull rebase paused but HEAD could not be resolved");
        return;
    };

    let pick = crate::git::pending_rebase_pick::pending_rebase_pick(
        source_commit,
        expected_parent,
        original_head.to_string(),
        onto_head,
        "pull_rebase_conflict",
    );
    if let Err(error) = repository.storage.write_pending_rebase_pick(&pick) {
        tracing::debug!("failed to write pending rebase pick: {}", error);
    }
}
```

If `rustfmt` splits the `read_first_valid_oid` chain, keep the behavior the same.

- [ ] **Step 2: Create pending state in failed pull rebase**

In `pull_post_command_hook`, inside:

```rust
if !exit_status.success() {
```

replace the repeated local `rebase_dir` / `rebase_apply_dir` conflict check with
`rebase_in_progress(repository)`. In the branch where the conflict is detected
and `original_head` exists, after writing `RebaseStart`, call:

```rust
create_pending_rebase_pick_for_paused_pull(repository, original_head, onto_head);
```

Because `onto_head` is moved into `RebaseStartEvent::new_with_onto`, compute it
as:

```rust
let onto_head = resolve_pull_rebase_onto_head(repository);
let start_event = RewriteLogEvent::rebase_start(
    crate::git::rewrite_log::RebaseStartEvent::new_with_onto(
        original_head.clone(),
        false,
        onto_head.clone(),
    ),
);
let _ = repository.storage.append_rewrite_event(start_event);
create_pending_rebase_pick_for_paused_pull(repository, original_head, onto_head);
```

- [ ] **Step 3: Mark pending aborted on non-conflict failed pull**

In the existing `else if config.is_rebase` branch for failed pull without a
rebase directory, add:

```rust
let _ = repository.storage.mark_pending_rebase_pick_aborted();
```

Keep `cancel_speculative_rebase_start(repository);`.

- [ ] **Step 4: Add equivalent helpers to `rebase_hooks.rs`**

In `src/commands/hooks/rebase_hooks.rs`, add local helpers or move shared
helpers to `pending_rebase_pick.rs`. Prefer shared helpers only if both files
need identical code. A minimal shared function in `pending_rebase_pick.rs` can
be:

```rust
pub fn stopped_rebase_source_commit(git_dir: &Path) -> Option<String> {
    read_first_valid_oid(&[
        git_dir.join("REBASE_HEAD"),
        git_dir.join("rebase-merge").join("stopped-sha"),
        git_dir.join("rebase-apply").join("stopped-sha"),
    ])
}
```

Then use it from both hooks.

- [ ] **Step 5: Create pending state for standalone failed rebase**

In `handle_rebase_post_command`, if `is_in_progress` is true, before returning,
create pending state:

```rust
if let Some(original_head) = context
    .rebase_original_head
    .clone()
    .or_else(|| find_rebase_start_event(repository).map(|event| event.original_head))
{
    let source_commit =
        crate::git::pending_rebase_pick::stopped_rebase_source_commit(repository.path());
    if let Some(source_commit) = source_commit
        && let Some(expected_parent) = repository.head().ok().and_then(|head| head.target().ok())
    {
        let pick = crate::git::pending_rebase_pick::pending_rebase_pick(
            source_commit,
            expected_parent,
            original_head,
            context.rebase_onto.clone(),
            "rebase_conflict",
        );
        let _ = repository.storage.write_pending_rebase_pick(&pick);
    }
}
return;
```

- [ ] **Step 6: Mark pending on abort and skip**

In `pre_rebase_hook`, when `parsed_args.has_command_flag("--abort")`, call:

```rust
let _ = repository.storage.mark_pending_rebase_pick_aborted();
```

When `parsed_args.has_command_flag("--skip")`, call:

```rust
let _ = repository.storage.mark_pending_rebase_pick_skipped();
```

- [ ] **Step 7: Run existing conflict tests**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_after_failed_push_commit_preserves_ai_notes NO_CAPTURE=true
```

Expected at this point:

```text
FAILED
```

Pending is created, but commit consumption is not implemented yet.

- [ ] **Step 8: Commit pending capture task**

If committing:

```bash
git add src/commands/hooks/fetch_hooks.rs src/commands/hooks/rebase_hooks.rs src/git/pending_rebase_pick.rs
git commit -m "feat: capture pending rebase picks"
```

## Task 4: Consume Pending State In Commit Hooks

**Files:**
- Modify: `src/commands/hooks/commit_hooks.rs`
- Test: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Add first-parent validator**

In `src/commands/hooks/commit_hooks.rs`, add:

```rust
fn first_parent_is(repository: &Repository, commit_sha: &str, expected_parent: &str) -> bool {
    let mut args = repository.global_args_for_exec();
    args.extend([
        "rev-parse".to_string(),
        format!("{}^", commit_sha),
    ]);
    crate::git::repository::exec_git(&args)
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|parent| parent.trim() == expected_parent)
        .unwrap_or(false)
}
```

- [ ] **Step 2: Add pending consumption before normal commit handling**

In `commit_post_command_hook`, after `let commit_author = ...;` and before the
`--amend` branch, add:

```rust
if !parsed_args.has_command_flag("--amend")
    && let (Some(orig), Some(sha)) = (original_commit.clone(), new_sha.clone())
    && first_parent_is(repository, &sha, &orig)
{
    match repository
        .storage
        .take_pending_rebase_pick_for_commit(&orig, &sha)
    {
        Ok(Some(pending)) => {
            let event = RewriteLogEvent::cherry_pick_complete(
                crate::git::rewrite_log::CherryPickCompleteEvent::new(
                    orig,
                    sha.clone(),
                    vec![pending.source_commit],
                    vec![sha],
                ),
            );
            repository.handle_rewrite_log_event(event, commit_author, supress_output, true);
            crate::observability::spawn_background_flush();
            return;
        }
        Ok(None) => {}
        Err(error) => {
            tracing::debug!("failed to consume pending rebase pick: {}", error);
        }
    }
}
```

- [ ] **Step 3: Run direct-commit preservation test**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_after_failed_push_commit_preserves_ai_notes NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 4: Run stale follow-up regression**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit wrapper consumption task**

If committing:

```bash
git add src/commands/hooks/commit_hooks.rs
git commit -m "feat: consume pending rebase pick on manual commit"
```

## Task 5: Prevent Duplicate Rewrite On `rebase --continue`

**Files:**
- Modify: `src/commands/hooks/rebase_hooks.rs`
- Modify: `src/git/pending_rebase_pick.rs`
- Test: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Add regression test**

In `tests/integration/pull_rebase_ff.rs`, add a test after
`test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head`:

```rust
#[test]
fn test_pull_rebase_conflict_manual_commit_then_continue_does_not_double_count() {
    let setup = setup_conflict_pull_test();
    let local = setup.local;

    local
        .git_og(&["push", "origin", "refs/notes/ai:refs/notes/ai"])
        .expect("push authorship notes should succeed");
    assert!(local.git(&["push"]).is_err());
    local.git(&["config", "pull.rebase", "true"]).unwrap();
    assert!(local.git(&["pull"]).is_err());

    std::fs::write(
        local.path().join("README.md"),
        "# Project\nSession A: AI-enhanced line 1\nSession A: AI-enhanced line 2\nSession B: AI-generated line 1\nSession B: AI-generated line 2\n",
    )
    .unwrap();
    local.git(&["add", "README.md"]).unwrap();
    local.git(&["commit", "-m", "Session B: AI feature"]).unwrap();
    let manual_commit = local.git(&["rev-parse", "HEAD"]).unwrap().trim().to_string();

    local
        .git_with_env(&["rebase", "--continue"], &[("GIT_EDITOR", "true")], None)
        .expect("rebase --continue should clear rebase state");
    assert_eq!(local.git(&["rev-parse", "HEAD"]).unwrap().trim(), manual_commit);

    let stats = local.stats().expect("stats should parse");
    assert_eq!(stats.ai_additions, 2);
    assert_eq!(stats.total_ai_additions, 2);
}
```

Add it to `crate::reuse_tests_in_worktree!`.

- [ ] **Step 2: Run test and verify failure if duplicate exists**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_manual_commit_then_continue_does_not_double_count NO_CAPTURE=true
```

Expected before implementation:

```text
FAILED
```

If it unexpectedly passes, inspect `rewrite_log` in the test temp repo by adding
temporary debug output locally, then remove the debug output before continuing.

- [ ] **Step 3: Add consumed lookup helper**

In `src/git/pending_rebase_pick.rs`, add:

```rust
pub fn read_consumed_rebase_pick_for_pair(
    path: &Path,
    source_commit: &str,
    new_commit: &str,
) -> Result<Option<PendingRebasePick>, GitAiError> {
    let Some(pick) = read_pending_rebase_pick(path)? else {
        return Ok(None);
    };
    if pick.status == PendingRebasePickStatus::Consumed
        && pick.source_commit == source_commit
        && pick.consumed_by.as_deref() == Some(new_commit)
    {
        Ok(Some(pick))
    } else {
        Ok(None)
    }
}
```

Add a delegating method to `RepoStorage`:

```rust
pub fn read_consumed_rebase_pick_for_pair(
    &self,
    source_commit: &str,
    new_commit: &str,
) -> Result<Option<crate::git::pending_rebase_pick::PendingRebasePick>, GitAiError> {
    crate::git::pending_rebase_pick::read_consumed_rebase_pick_for_pair(
        &self.pending_rebase_pick,
        source_commit,
        new_commit,
    )
}
```

- [ ] **Step 4: Filter consumed pairs in completed rebase**

In `src/commands/hooks/rebase_hooks.rs`, after building
`original_commits` and `new_commits` in `process_completed_rebase`, filter
one-to-one consumed pairs before creating `RebaseCompleteEvent`:

```rust
let mut filtered_original_commits = Vec::new();
let mut filtered_new_commits = Vec::new();
for (source, target) in original_commits.iter().zip(new_commits.iter()) {
    let already_consumed = repository
        .storage
        .read_consumed_rebase_pick_for_pair(source, target)
        .ok()
        .flatten()
        .is_some();
    if !already_consumed {
        filtered_original_commits.push(source.clone());
        filtered_new_commits.push(target.clone());
    }
}

let original_commits = filtered_original_commits;
let new_commits = filtered_new_commits;
```

If the lengths differ, only apply pair filtering when `original_commits.len() ==
new_commits.len()`. For non-one-to-one rebases, leave the existing mapping
unchanged to avoid dropping valid many-to-one rewrites.

- [ ] **Step 5: Run continue test**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_manual_commit_then_continue_does_not_double_count NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit duplicate prevention task**

If committing:

```bash
git add src/git/pending_rebase_pick.rs src/git/repo_storage.rs src/commands/hooks/rebase_hooks.rs tests/integration/pull_rebase_ff.rs
git commit -m "fix: avoid duplicate rewrite after manual rebase commit"
```

## Task 6: Wire Daemon To Pending State Without Reading Git Rebase Files

**Files:**
- Modify: `src/daemon.rs`
- Test: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Locate daemon `CommitCreated` mapping**

In `src/daemon.rs`, find the `SemanticEvent::CommitCreated { base, new_head }`
branch in `ActorDaemonCoordinator`.

Ensure the branch no longer calls any helper named
`resolve_rebase_current_source_for_worktree`.

- [ ] **Step 2: Add pending consumption in daemon side-effect mapping**

Replace the plain commit event push in the `CommitCreated` branch with:

```rust
let maybe_pending_event = if let (Some(worktree), Some(pre_head)) =
    (cmd.worktree.as_deref(), base.as_deref())
{
    crate::git::repository::find_repository_in_path(
        worktree.to_string_lossy().as_ref(),
    )
    .ok()
    .and_then(|repo| {
        repo.storage
            .take_pending_rebase_pick_for_commit(pre_head, new_head)
            .ok()
            .flatten()
            .map(|pending| {
                RewriteLogEvent::cherry_pick_complete(
                    crate::git::rewrite_log::CherryPickCompleteEvent::new(
                        pre_head.to_string(),
                        new_head.clone(),
                        vec![pending.source_commit],
                        vec![new_head.clone()],
                    ),
                )
            })
    })
} else {
    None
};

if let Some(event) = maybe_pending_event {
    out.push(event);
} else {
    out.push(RewriteLogEvent::commit(base.clone(), new_head.clone()));
}
```

If `find_repository_in_path` is not available in this module under that path,
use the existing repository-opening helper already used by neighboring daemon
side-effect code.

- [ ] **Step 3: Run wrapper-daemon targeted tests**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

Then run explicitly in wrapper-daemon mode if the project supports that mode in
the current environment:

```bash
task test:wrapper-daemon TEST_FILTER=test_pull_rebase_conflict_manual_commit_does_not_reuse_stale_rebase_head NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 4: Commit daemon task**

If committing:

```bash
git add src/daemon.rs
git commit -m "fix: consume pending rebase picks in daemon"
```

## Task 7: Add REST Rewrite API Types And Client

**Files:**
- Modify: `src/api/types.rs`
- Modify: `src/api/authorship_notes.rs`
- Test: unit tests in `src/api/types.rs` or `src/api/authorship_notes.rs`

- [ ] **Step 1: Add API types**

In `src/api/types.rs`, after `AuthorshipNotesPushResponse`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorshipNotesRewriteMapping {
    pub source_commit: String,
    pub target_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_note_blob_oid: Option<String>,
    pub target_note_blob_oid: String,
    pub target_content: String,
    pub commit_time: i64,
    pub author_name: String,
    pub author_email: String,
    pub disposition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorshipNotesRewriteRequest {
    pub repo_url: String,
    pub rewrite_id: String,
    pub operation: String,
    pub branch: String,
    pub original_head: String,
    pub new_head: String,
    pub mappings: Vec<AuthorshipNotesRewriteMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorshipNotesRewriteConflict {
    pub source_commit: String,
    pub target_commit: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorshipNotesRewriteData {
    pub created: usize,
    pub updated: usize,
    pub superseded: usize,
    #[serde(default)]
    pub unchanged: usize,
    #[serde(default)]
    pub conflicts: Vec<AuthorshipNotesRewriteConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorshipNotesRewriteResponse {
    pub ok: bool,
    pub data: AuthorshipNotesRewriteData,
}
```

- [ ] **Step 2: Add serialization unit test**

In `src/api/types.rs` tests module, add:

```rust
#[test]
fn authorship_notes_rewrite_request_serializes_expected_shape() {
    let request = AuthorshipNotesRewriteRequest {
        repo_url: "https://github.com/org/repo".to_string(),
        rewrite_id: "rewrite-1".to_string(),
        operation: "rebase_conflict_manual_commit".to_string(),
        branch: "main".to_string(),
        original_head: "b".repeat(40),
        new_head: "d".repeat(40),
        mappings: vec![AuthorshipNotesRewriteMapping {
            source_commit: "b".repeat(40),
            target_commit: "d".repeat(40),
            source_note_blob_oid: Some("old-note".to_string()),
            target_note_blob_oid: "new-note".to_string(),
            target_content: "{}".to_string(),
            commit_time: 1710000000,
            author_name: "User".to_string(),
            author_email: "user@example.com".to_string(),
            disposition: "supersede_source".to_string(),
        }],
    };

    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["repo_url"], "https://github.com/org/repo");
    assert_eq!(value["mappings"][0]["disposition"], "supersede_source");
    assert_eq!(value["mappings"][0]["source_note_blob_oid"], "old-note");
}
```

- [ ] **Step 3: Run type test**

Run:

```bash
task test TEST_FILTER=authorship_notes_rewrite_request_serializes_expected_shape NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 4: Add API client method**

In `src/api/authorship_notes.rs`, import the new types:

```rust
AuthorshipNotesRewriteRequest, AuthorshipNotesRewriteResponse,
```

Add method:

```rust
pub fn authorship_notes_rewrite(
    &self,
    request: &AuthorshipNotesRewriteRequest,
) -> Result<AuthorshipNotesRewriteResponse, GitAiError> {
    let response = self
        .context()
        .post_json("/worker/authorship_notes/rewrite", request)?;
    let status_code = response.status_code;
    let body = response
        .as_str()
        .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

    if status_code != 200 {
        let message = parse_api_error_message(body, "Notes rewrite request failed");
        return Err(GitAiError::Generic(format!(
            "Notes rewrite failed with status {}: {}",
            status_code, message
        )));
    }

    let parsed: AuthorshipNotesRewriteResponse =
        serde_json::from_str(body).map_err(GitAiError::JsonError)?;
    if !parsed.ok {
        return Err(GitAiError::Generic(
            "Notes rewrite returned ok=false".to_string(),
        ));
    }
    Ok(parsed)
}
```

- [ ] **Step 5: Commit REST API type task**

If committing:

```bash
git add src/api/types.rs src/api/authorship_notes.rs
git commit -m "feat: add REST authorship notes rewrite API"
```

## Task 8: Send REST Rewrite Requests From Rewrite Side Effects

**Files:**
- Modify: `src/git/sync_authorship.rs`
- Modify: `src/authorship/rebase_authorship.rs` or `src/git/repository.rs`
- Test: `tests/rest_notes_sync.rs`

- [ ] **Step 1: Add rewrite request builder test**

In `tests/rest_notes_sync.rs`, add a test using the existing mock server style:

```rust
#[test]
fn rest_notes_rewrite_posts_supersede_mapping() {
    let server = RestNotesMockServer::start(vec![serde_json::json!({
        "ok": true,
        "data": {
            "created": 1,
            "updated": 0,
            "superseded": 1,
            "unchanged": 0,
            "conflicts": []
        }
    })]);

    let api = git_ai::api::client::ApiClient::new(
        git_ai::api::client::ApiContext::new(Some(server.base_url().to_string())),
    );

    let request = git_ai::api::types::AuthorshipNotesRewriteRequest {
        repo_url: "https://github.com/org/repo".to_string(),
        rewrite_id: "rewrite-1".to_string(),
        operation: "rebase_conflict_manual_commit".to_string(),
        branch: "main".to_string(),
        original_head: "b".repeat(40),
        new_head: "d".repeat(40),
        mappings: vec![git_ai::api::types::AuthorshipNotesRewriteMapping {
            source_commit: "b".repeat(40),
            target_commit: "d".repeat(40),
            source_note_blob_oid: Some("old-note".to_string()),
            target_note_blob_oid: "new-note".to_string(),
            target_content: "{}".to_string(),
            commit_time: 1710000000,
            author_name: "User".to_string(),
            author_email: "user@example.com".to_string(),
            disposition: "supersede_source".to_string(),
        }],
    };

    api.authorship_notes_rewrite(&request).unwrap();

    let recorded = server.recv_request();
    assert_eq!(recorded.path, "/worker/authorship_notes/rewrite");
    assert_eq!(recorded.body["rewrite_id"], "rewrite-1");
    assert_eq!(recorded.body["mappings"][0]["source_commit"], "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(recorded.body["mappings"][0]["target_commit"], "dddddddddddddddddddddddddddddddddddddddd");
}
```

- [ ] **Step 2: Run REST test**

Run:

```bash
task test TEST_FILTER=rest_notes_rewrite_posts_supersede_mapping NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Add sync helper**

In `src/git/sync_authorship.rs`, add a public helper:

```rust
pub fn rest_rewrite_authorship_notes(
    repository: &Repository,
    repo_url: &str,
    operation: &str,
    original_head: &str,
    new_head: &str,
    mappings: &[(String, String)],
) -> Result<(), GitAiError> {
    if Config::fresh().notes_store() != "rest" || mappings.is_empty() {
        return Ok(());
    }

    let api = ApiClient::new(ApiContext::new(None));
    let branch = get_current_branch(repository).unwrap_or_else(|_| "main".to_string());
    let commit_authorships = get_commits_with_notes_from_list(
        repository,
        &mappings.iter().map(|(_, target)| target.clone()).collect::<Vec<_>>(),
    )?;

    let mut commit_author_map: HashMap<String, (String, i64)> = HashMap::new();
    for authorship in commit_authorships {
        match authorship {
            CommitAuthorship::NoLog { sha, git_author, commit_time }
            | CommitAuthorship::Log { sha, git_author, commit_time, .. } => {
                commit_author_map.insert(sha, (git_author, commit_time));
            }
        }
    }

    let mut rewrite_mappings = Vec::new();
    for (source, target) in mappings {
        let Some(target_content) = show_authorship_note(repository, target) else {
            continue;
        };
        let target_note_blob_oid = repository.blob(target_content.as_bytes())?;
        let source_note_blob_oid = show_authorship_note(repository, source)
            .map(|content| repository.blob(content.as_bytes()))
            .transpose()?;
        let (git_author, commit_time) = commit_author_map
            .get(target)
            .cloned()
            .unwrap_or_else(|| ("Unknown <unknown@example.com>".to_string(), 0));
        let (author_name, author_email) = parse_author_identity(&git_author);

        rewrite_mappings.push(crate::api::types::AuthorshipNotesRewriteMapping {
            source_commit: source.clone(),
            target_commit: target.clone(),
            source_note_blob_oid,
            target_note_blob_oid,
            target_content,
            commit_time,
            author_name,
            author_email,
            disposition: "supersede_source".to_string(),
        });
    }

    if rewrite_mappings.is_empty() {
        return Ok(());
    }

    let rewrite_id = sha256_note_content(&format!(
        "{}:{}:{}:{}:{}",
        repo_url, operation, original_head, new_head, rewrite_mappings[0].target_note_blob_oid
    ));

    api.authorship_notes_rewrite(&crate::api::types::AuthorshipNotesRewriteRequest {
        repo_url: repo_url.to_string(),
        rewrite_id,
        operation: operation.to_string(),
        branch,
        original_head: original_head.to_string(),
        new_head: new_head.to_string(),
        mappings: rewrite_mappings,
    })?;

    Ok(())
}
```

If `repository.blob` writes unnecessary blobs for existing note content, replace
it with existing note blob lookup helpers already used by `rest_push_notes`.

- [ ] **Step 4: Call REST rewrite after local rewrite side effects**

In `src/authorship/rebase_authorship.rs`, after successful
`rewrite_authorship_after_rebase_v2` and after successful
`rewrite_authorship_after_cherry_pick`, call the REST helper only for one-to-one
mappings:

```rust
if rebase_complete.original_commits.len() == rebase_complete.new_commits.len()
    && let Ok(Some(remote)) = repo.upstream_remote()
    && let Ok(repo_url) = crate::git::sync_authorship::normalized_rest_repo_url(repo, &remote)
{
    let mappings: Vec<(String, String)> = rebase_complete
        .original_commits
        .iter()
        .cloned()
        .zip(rebase_complete.new_commits.iter().cloned())
        .collect();
    let _ = crate::git::sync_authorship::rest_rewrite_authorship_notes(
        repo,
        &repo_url,
        "rebase_complete",
        &rebase_complete.original_head,
        &rebase_complete.new_head,
        &mappings,
    );
}
```

For `CherryPickComplete`, use:

```rust
if cherry_pick_complete.source_commits.len() == cherry_pick_complete.new_commits.len()
    && let Ok(Some(remote)) = repo.upstream_remote()
    && let Ok(repo_url) = crate::git::sync_authorship::normalized_rest_repo_url(repo, &remote)
{
    let mappings: Vec<(String, String)> = cherry_pick_complete
        .source_commits
        .iter()
        .cloned()
        .zip(cherry_pick_complete.new_commits.iter().cloned())
        .collect();
    let _ = crate::git::sync_authorship::rest_rewrite_authorship_notes(
        repo,
        &repo_url,
        "cherry_pick_complete",
        &cherry_pick_complete.original_head,
        &cherry_pick_complete.new_head,
        &mappings,
    );
}
```

If `normalized_rest_repo_url` is private, make it `pub(crate)` and keep the
visibility scoped to the crate.

- [ ] **Step 5: Run REST tests**

Run:

```bash
task test TEST_FILTER=rest_notes_rewrite NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit REST rewrite sending task**

If committing:

```bash
git add src/git/sync_authorship.rs src/authorship/rebase_authorship.rs tests/rest_notes_sync.rs
git commit -m "feat: send REST notes rewrite mappings"
```

## Task 9: Add Abort And Skip Integration Coverage

**Files:**
- Modify: `tests/integration/pull_rebase_ff.rs`

- [ ] **Step 1: Add abort cleanup test**

Add:

```rust
#[test]
fn test_pull_rebase_conflict_abort_clears_pending_manual_commit_mapping() {
    let setup = setup_conflict_pull_test();
    let local = setup.local;

    local.git(&["config", "pull.rebase", "true"]).unwrap();
    assert!(local.git(&["pull"]).is_err());
    local.git(&["rebase", "--abort"]).unwrap();

    std::fs::write(
        local.path().join("README.md"),
        "# Project\nManual after abort\n",
    )
    .unwrap();
    local.git(&["add", "README.md"]).unwrap();
    local.git(&["commit", "-m", "Manual after abort"]).unwrap();

    let stats = local.stats().unwrap();
    assert_eq!(stats.ai_additions, 0);
    assert_eq!(stats.total_ai_additions, 0);
}
```

Add the test to `crate::reuse_tests_in_worktree!`.

- [ ] **Step 2: Run abort cleanup test**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_abort_clears_pending_manual_commit_mapping NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Add skip cleanup test if scenario can be made non-interactive**

If `git rebase --skip` is valid in the conflict fixture, add:

```rust
#[test]
fn test_pull_rebase_conflict_skip_clears_pending_manual_commit_mapping() {
    let setup = setup_conflict_pull_test();
    let local = setup.local;

    local.git(&["config", "pull.rebase", "true"]).unwrap();
    assert!(local.git(&["pull"]).is_err());
    local.git(&["rebase", "--skip"]).unwrap();

    std::fs::write(
        local.path().join("README.md"),
        "# Project\nManual after skip\n",
    )
    .unwrap();
    local.git(&["add", "README.md"]).unwrap();
    local.git(&["commit", "-m", "Manual after skip"]).unwrap();

    let stats = local.stats().unwrap();
    assert_eq!(stats.ai_additions, 0);
    assert_eq!(stats.total_ai_additions, 0);
}
```

If the fixture cannot use `--skip` because there are no remaining commits and
Git exits with no commit created, do not add a brittle test. Keep the unit
coverage from Task 2 for skip state persistence.

- [ ] **Step 4: Run skip cleanup test if added**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict_skip_clears_pending_manual_commit_mapping NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit cleanup coverage task**

If committing:

```bash
git add tests/integration/pull_rebase_ff.rs
git commit -m "test: cover pending rebase pick cleanup"
```

## Task 10: Final Verification

**Files:**
- All modified files

- [ ] **Step 1: Run targeted regression suite**

Run:

```bash
task test TEST_FILTER=test_pull_rebase_conflict NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 2: Run REST notes tests**

Run:

```bash
task test TEST_FILTER=rest_notes NO_CAPTURE=true
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Build**

Run:

```bash
task build
```

Expected:

```text
Finished
```

- [ ] **Step 4: Format**

Run:

```bash
task fmt
```

Expected:

```text
cargo fmt
```

- [ ] **Step 5: Lint**

Run:

```bash
task lint
```

Expected:

```text
no warnings or errors
```

- [ ] **Step 6: Review final diff**

Run:

```bash
git diff --stat
git diff -- src/commands/hooks/commit_hooks.rs src/commands/hooks/fetch_hooks.rs src/commands/hooks/rebase_hooks.rs src/git/pending_rebase_pick.rs src/git/repo_storage.rs src/daemon.rs src/api/types.rs src/api/authorship_notes.rs src/git/sync_authorship.rs src/authorship/rebase_authorship.rs tests/integration/pull_rebase_ff.rs tests/rest_notes_sync.rs
```

Expected:

```text
Diff only contains pending rebase pick state, hook consumption, REST rewrite API, and tests.
No direct commit-time REBASE_HEAD reads remain.
```

- [ ] **Step 7: Final commit**

If committing all remaining implementation changes together:

```bash
git add src/commands/hooks/commit_hooks.rs src/commands/hooks/fetch_hooks.rs src/commands/hooks/rebase_hooks.rs src/git/pending_rebase_pick.rs src/git/mod.rs src/git/repo_storage.rs src/daemon.rs src/api/types.rs src/api/authorship_notes.rs src/git/sync_authorship.rs src/authorship/rebase_authorship.rs tests/integration/pull_rebase_ff.rs tests/rest_notes_sync.rs
git commit -m "fix: preserve authorship for manual rebase conflict commits"
```

## Self-Review

Spec coverage:

- Pending one-shot state: Tasks 2, 3, 4, 5, 6, 9.
- No stale `REBASE_HEAD` in commit path: Tasks 1, 4, 6, 10.
- Direct manual commit preserves B -> D authorship: Task 4.
- Follow-up E does not inherit B metrics: Task 4.
- `rebase --continue` deduplication: Task 5.
- Abort/skip cleanup: Tasks 3 and 9.
- Wrapper/daemon consistency: Task 6.
- REST rewrite API and supersede semantics: Tasks 7 and 8.
- Verification: Task 10.

Placeholder scan:

- No placeholder markers or unspecified "add tests" steps remain.
- Steps include concrete files, commands, and expected results.

Type consistency:

- `PendingRebasePick`, `PendingRebasePickStatus`, and helper names are used consistently.
- REST type names use the `AuthorshipNotesRewrite*` prefix.
- The implementation intentionally reuses `CherryPickComplete` for pending pick consumption.
