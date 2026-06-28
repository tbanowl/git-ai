# Merge Conflict AI Attribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the post-commit overlay fix for ordinary merge commits so AI-authored conflict-resolution lines are written to the merge commit authorship note.

**Architecture:** Add a small `authorship::conflict_resolution` module that can merge an existing authorship log with an AI resolution log using uncovered-line semantics. Wire this module into `post_commit_with_final_state` only for merge commits that have non-human checkpoint entries in the first-parent working log. Keep checkpoint `EntryKind::Unmerged` behavior unchanged in Phase 1.

**Tech Stack:** Rust 2024, existing git-ai authorship log model, existing working log/checkpoint storage, integration tests in `tests/integration/merge_rebase.rs`.

## Global Constraints

- Do not migrate the daemon / trace architecture from `D:/banz/code/fantacy/git-ai`.
- Do not add a full `MergeStart / MergeComplete / MergeAbort` rewrite event lifecycle in Phase 1.
- Do not change the default `EntryKind::Unmerged` checkpoint skip behavior in Phase 1.
- Do not infer AI authorship heuristically; only use checkpoint / working log / authorship note evidence.
- Do not alter existing `merge --squash`, rebase, cherry-pick, stash, or commit amend logic except through shared helper code that is explicitly covered by tests.
- Keep merge commit stats behavior unchanged: merge commits may continue to skip expensive stats.

---

## File Structure

- Create `src/authorship/conflict_resolution.rs`
  - Owns pure `AuthorshipLog` merge helpers.
  - Owns building a resolution `AuthorshipLog` from AI checkpoints in a base working log.
  - Does not write git notes and does not mutate working logs.

- Modify `src/authorship/mod.rs`
  - Exposes `pub mod conflict_resolution;`.

- Modify `src/authorship/post_commit.rs`
  - Detects merge commits after normal authorship log generation.
  - Calls a small helper that overlays AI conflict-resolution attribution when there is explicit AI checkpoint evidence.
  - Falls back to the normal authorship log if overlay cannot be built.

- Modify `tests/integration/merge_rebase.rs`
  - Adds a bubble-sort-shaped regression test.
  - Adds a human-resolution guard test if coverage is not already sufficient.

## Interfaces

### New public helper

```rust
pub fn merge_conflict_resolution_authorship(
    existing_shifted_log: Option<AuthorshipLog>,
    resolution_log: AuthorshipLog,
    commit_sha: &str,
) -> AuthorshipLog
```

Consumes:
- `existing_shifted_log`: the normal post-commit authorship log or another already-shifted parent/source log.
- `resolution_log`: authorship log built from AI checkpoint entries produced during conflict resolution.
- `commit_sha`: merge commit SHA.

Produces:
- A merged `AuthorshipLog` whose `metadata.base_commit_sha` is `commit_sha`.
- Existing coverage wins. Resolution attribution only fills uncovered lines.

### New internal helper

```rust
pub(crate) fn build_authorship_log_from_ai_checkpoints(
    repo: &Repository,
    base_commit: &str,
    commit_sha: &str,
    changed_files: &HashSet<String>,
) -> Option<AuthorshipLog>
```

Consumes:
- `repo`: current repository.
- `base_commit`: first parent / pre-merge HEAD used by the working log.
- `commit_sha`: final merge commit.
- `changed_files`: files changed between first parent and merge commit.

Produces:
- `Some(AuthorshipLog)` if the first-parent working log contains AI checkpoint line attributions for any changed files.
- `None` for human-only resolutions, missing working logs, missing agent IDs, or no relevant line attributions.

### New post-commit helper

```rust
fn maybe_overlay_merge_conflict_resolution_authorship(
    repo: &Repository,
    parent_sha: &str,
    commit_sha: &str,
    authorship_log: AuthorshipLog,
) -> AuthorshipLog
```

Consumes:
- The normal authorship log that `post_commit_with_final_state` already built.

Produces:
- The original log if this is not a merge commit or no AI resolution log exists.
- The merged log if an AI resolution log exists.

## Task 1: Add Failing Bubble Sort Regression Test

**Files:**
- Modify: `tests/integration/merge_rebase.rs`

**Interfaces:**
- Consumes existing `TestRepo`, `TestFile`, `lines!`, `.ai()`, `.human()` helpers.
- Produces failing test `test_merge_conflict_ai_resolution_combines_parent_arguments`.

- [ ] **Step 1: Add the failing test**

Append this test before `crate::reuse_tests_in_worktree!(...)` in `tests/integration/merge_rebase.rs`:

```rust
/// Regression: an AI merge-conflict resolution can synthesize a line that is
/// not identical to either parent. The synthesized function signature and
/// docstring line must be attributed to the AI checkpoint that wrote the
/// resolved file.
#[test]
fn test_merge_conflict_ai_resolution_combines_parent_arguments() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("bubble_sort.py");

    let base = "\
\"\"\"冒泡排序算法实现。\n\n\"\"\"\n+\n+\n+def bubble_sort(arr):\n+    \"\"\"对列表进行原地冒泡排序，并返回该列表。\n+\n+    Args:\n+        arr: 待排序的可变序列（list），元素需支持 ``>`` 比较。\n+\n+    Returns:\n+        排序后的同一个 list（原地修改）。\n+    \"\"\"\n+    n = len(arr)\n+    for i in range(n - 1):\n+        swapped = False\n+        for j in range(n - 1 - i):\n+            if arr[j] > arr[j + 1]:\n+                arr[j], arr[j + 1] = arr[j + 1], arr[j]\n+                swapped = True\n+        if not swapped:\n+            break\n+    return arr\n+";

    std::fs::write(&file_path, base).unwrap();
    repo.stage_all_and_commit("initial bubble sort").unwrap();
    let main_branch = repo.current_branch();

    repo.git(&["checkout", "-b", "git-ai-1"]).unwrap();
    let reverse_version = base
        .replace("def bubble_sort(arr):", "def bubble_sort(arr, *, reverse=False):")
        .replace(
            "对列表进行原地冒泡排序，并返回该列表。",
            "对列表进行原地冒泡排序，并返回该列表。",
        )
        .replace(
            "if arr[j] > arr[j + 1]:",
            "if (arr[j] < arr[j + 1]) if reverse else (arr[j] > arr[j + 1]):",
        );
    std::fs::write(&file_path, reverse_version).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "bubble_sort.py"])
        .unwrap();
    repo.stage_all_and_commit("feat: add reverse option").unwrap();

    repo.git(&["checkout", &main_branch]).unwrap();
    repo.git(&["checkout", "-b", "git-ai-2"]).unwrap();
    let inplace_version = base
        .replace("def bubble_sort(arr):", "def bubble_sort(arr, *, inplace=True):")
        .replace(
            "对列表进行原地冒泡排序，并返回该列表。",
            "对列表进行冒泡排序（升序），并返回排序结果。",
        )
        .replace(
            "    n = len(arr)",
            "    if not inplace:\n        arr = list(arr)\n    n = len(arr)",
        );
    std::fs::write(&file_path, inplace_version).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "bubble_sort.py"])
        .unwrap();
    repo.stage_all_and_commit("feat: add inplace option").unwrap();

    let merge_result = repo.git(&["merge", "git-ai-1"]);
    assert!(merge_result.is_err(), "merge should conflict on bubble_sort.py");

    let resolved = "\
\"\"\"冒泡排序算法实现。\n\n\"\"\"\n+\n+\n+def bubble_sort(arr, *, inplace=True, reverse=False):\n+    \"\"\"对列表进行冒泡排序，并返回排序结果。\n+\n+    Args:\n+        arr: 待排序的序列（list），元素需支持 ``>`` 比较。\n+        inplace: 若为 True（默认）则原地排序并返回原列表；\n+            若为 False 则对副本排序，不修改原列表。\n+        reverse: 若为 True 则按降序排序，默认升序。\n+\n+    Returns:\n+        排序后的 list。\n+    \"\"\"\n+    if not inplace:\n+        arr = list(arr)\n+    n = len(arr)\n+    for i in range(n - 1):\n+        swapped = False\n+        for j in range(n - 1 - i):\n+            if (arr[j] < arr[j + 1]) if reverse else (arr[j] > arr[j + 1]):\n+                arr[j], arr[j + 1] = arr[j + 1], arr[j]\n+                swapped = True\n+        if not swapped:\n+            break\n+    return arr\n+";
    std::fs::write(&file_path, resolved).unwrap();
    repo.git(&["add", "bubble_sort.py"]).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "bubble_sort.py"])
        .unwrap();
    repo.stage_all_and_commit("merge resolved by AI").unwrap();

    let mut file = repo.filename("bubble_sort.py");
    file.assert_lines_and_blame(crate::lines![
        "\"\"\"冒泡排序算法实现。".human(),
        "".human(),
        "\"\"\"".human(),
        "".human(),
        "".human(),
        "def bubble_sort(arr, *, inplace=True, reverse=False):".ai(),
        "    \"\"\"对列表进行冒泡排序，并返回排序结果。".ai(),
    ]);
}
```

- [ ] **Step 2: Add the test to worktree reuse macro**

Update the macro at the end of `tests/integration/merge_rebase.rs`:

```rust
crate::reuse_tests_in_worktree!(
    test_blame_after_merge_conflict_resolution,
    test_merge_conflict_ai_resolution_outside_session,
    test_merge_conflict_ai_resolution_combines_parent_arguments,
);
```

- [ ] **Step 3: Run the failing test**

Run:

```bash
task test TEST_FILTER=test_merge_conflict_ai_resolution_combines_parent_arguments
```

Expected before implementation: the test fails because the synthesized signature/docstring lines are not AI-attributed or the merge commit authorship note is empty.

## Task 2: Add Conflict Resolution Authorship Module

**Files:**
- Create: `src/authorship/conflict_resolution.rs`
- Modify: `src/authorship/mod.rs`

**Interfaces:**
- Produces `merge_conflict_resolution_authorship(...)` for `post_commit.rs`.
- Produces `build_authorship_log_from_ai_checkpoints(...)` for `post_commit.rs`.

- [ ] **Step 1: Create `src/authorship/conflict_resolution.rs`**

Create the file with this implementation:

```rust
use std::collections::{HashMap, HashSet};

use crate::authorship::authorship_log::LineRange;
use crate::authorship::authorship_log_serialization::{
    AttestationEntry, AuthorshipLog, FileAttestation, generate_short_hash,
};
use crate::authorship::working_log::CheckpointKind;
use crate::git::repository::Repository;

fn normalize_line_ranges(ranges: &[LineRange]) -> Vec<LineRange> {
    let mut lines: Vec<u32> = ranges.iter().flat_map(LineRange::expand).collect();
    lines.sort_unstable();
    lines.dedup();
    LineRange::compress_lines(&lines)
}

fn subtract_line_ranges(ranges: &[LineRange], covered: &[LineRange]) -> Vec<LineRange> {
    let mut remaining = ranges.to_vec();
    for covered_range in covered {
        remaining = remaining
            .iter()
            .flat_map(|range| range.remove(covered_range))
            .collect();
        if remaining.is_empty() {
            break;
        }
    }
    normalize_line_ranges(&remaining)
}

fn line_coverage_by_file(log: &AuthorshipLog) -> HashMap<String, Vec<LineRange>> {
    let mut coverage: HashMap<String, Vec<LineRange>> = HashMap::new();
    for attestation in &log.attestations {
        let file_coverage = coverage.entry(attestation.file_path.clone()).or_default();
        for entry in &attestation.entries {
            file_coverage.extend(entry.line_ranges.clone());
        }
    }
    for ranges in coverage.values_mut() {
        *ranges = normalize_line_ranges(ranges);
    }
    coverage
}

fn retain_referenced_metadata(log: &mut AuthorshipLog) {
    let mut prompt_keys = HashSet::new();
    for attestation in &log.attestations {
        for entry in &attestation.entries {
            prompt_keys.insert(entry.hash.clone());
        }
    }
    log.metadata
        .prompts
        .retain(|key, _| prompt_keys.contains(key));
}

fn filter_resolution_log_to_uncovered_lines(
    mut resolution_log: AuthorshipLog,
    shifted_log: &AuthorshipLog,
) -> AuthorshipLog {
    let shifted_coverage = line_coverage_by_file(shifted_log);

    for attestation in &mut resolution_log.attestations {
        let covered = shifted_coverage
            .get(&attestation.file_path)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for entry in &mut attestation.entries {
            entry.line_ranges = subtract_line_ranges(&entry.line_ranges, covered);
        }
        attestation
            .entries
            .retain(|entry| !entry.line_ranges.is_empty());
    }

    resolution_log
        .attestations
        .retain(|attestation| !attestation.entries.is_empty());
    retain_referenced_metadata(&mut resolution_log);
    resolution_log
}

fn merge_file_attestations(target: &mut AuthorshipLog, source: &AuthorshipLog) {
    for source_attestation in &source.attestations {
        let target_attestation = target.get_or_create_file(&source_attestation.file_path);
        for source_entry in &source_attestation.entries {
            if let Some(target_entry) = target_attestation
                .entries
                .iter_mut()
                .find(|entry| entry.hash == source_entry.hash)
            {
                target_entry
                    .line_ranges
                    .extend(source_entry.line_ranges.clone());
                target_entry.line_ranges = normalize_line_ranges(&target_entry.line_ranges);
            } else {
                let mut entry = source_entry.clone();
                entry.line_ranges = normalize_line_ranges(&entry.line_ranges);
                target_attestation.entries.push(entry);
            }
        }
    }
}

fn merge_authorship_metadata(target: &mut AuthorshipLog, source: &AuthorshipLog) {
    for (key, record) in &source.metadata.prompts {
        target
            .metadata
            .prompts
            .entry(key.clone())
            .or_insert_with(|| record.clone());
    }
}

pub fn merge_conflict_resolution_authorship(
    existing_shifted_log: Option<AuthorshipLog>,
    resolution_log: AuthorshipLog,
    commit_sha: &str,
) -> AuthorshipLog {
    let mut merged = existing_shifted_log.unwrap_or_default();
    let resolution_log = filter_resolution_log_to_uncovered_lines(resolution_log, &merged);

    merge_file_attestations(&mut merged, &resolution_log);
    merge_authorship_metadata(&mut merged, &resolution_log);
    merged.metadata.base_commit_sha = commit_sha.to_string();
    merged
}

pub(crate) fn build_authorship_log_from_ai_checkpoints(
    repo: &Repository,
    base_commit: &str,
    commit_sha: &str,
    changed_files: &HashSet<String>,
) -> Option<AuthorshipLog> {
    let working_log = repo.storage.working_log_for_base_commit(base_commit).ok()?;
    let checkpoints = working_log.read_all_checkpoints().ok()?;
    let mut authorship_log = AuthorshipLog::new();
    authorship_log.metadata.base_commit_sha = commit_sha.to_string();

    let mut file_author_lines: HashMap<String, HashMap<String, Vec<u32>>> = HashMap::new();

    for checkpoint in &checkpoints {
        if checkpoint.kind == CheckpointKind::Human {
            continue;
        }
        let agent_id = checkpoint.agent_id.as_ref()?;
        let author_id = generate_short_hash(&agent_id.id, &agent_id.tool);
        authorship_log
            .metadata
            .prompts
            .entry(author_id.clone())
            .or_insert_with(|| crate::authorship::authorship_log::PromptRecord {
                agent_id: agent_id.clone(),
                human_author: None,
                messages: Vec::new(),
                total_additions: checkpoint.line_stats.additions,
                total_deletions: checkpoint.line_stats.deletions,
                accepted_lines: 0,
                overriden_lines: 0,
                messages_url: None,
                custom_attributes: None,
            });

        for entry in &checkpoint.entries {
            if !changed_files.contains(&entry.file) {
                continue;
            }
            for line_attr in &entry.line_attributions {
                let lines = file_author_lines
                    .entry(entry.file.clone())
                    .or_default()
                    .entry(line_attr.author_id.clone())
                    .or_default();
                lines.extend(line_attr.start_line..=line_attr.end_line);
            }
        }
    }

    for (file_path, author_lines) in file_author_lines {
        let mut file_attestation = FileAttestation::new(file_path);
        for (author_id, mut lines) in author_lines {
            lines.sort_unstable();
            lines.dedup();
            if lines.is_empty() {
                continue;
            }
            let line_ranges = LineRange::compress_lines(&lines);
            let accepted_lines = lines.len() as u32;
            if let Some(record) = authorship_log.metadata.prompts.get_mut(&author_id) {
                record.accepted_lines = record.accepted_lines.saturating_add(accepted_lines);
            }
            file_attestation
                .entries
                .push(AttestationEntry::new(author_id, line_ranges));
        }
        if !file_attestation.entries.is_empty() {
            authorship_log.attestations.push(file_attestation);
        }
    }

    if authorship_log.attestations.is_empty() {
        None
    } else {
        Some(authorship_log)
    }
}
```

- [ ] **Step 2: Export the module**

Add this line to `src/authorship/mod.rs`:

```rust
pub mod conflict_resolution;
```

- [ ] **Step 3: Run diagnostics/build for the new module**

Run:

```bash
task build
```

Expected: compilation may fail only if API names need minor adjustment. Fix API mismatch in this task before continuing.

## Task 3: Wire Merge Overlay into Post-Commit

**Files:**
- Modify: `src/authorship/post_commit.rs`

**Interfaces:**
- Consumes `conflict_resolution::build_authorship_log_from_ai_checkpoints`.
- Consumes `conflict_resolution::merge_conflict_resolution_authorship`.
- Produces merge-aware overlay behavior for normal merge commits.

- [ ] **Step 1: Add imports**

At the top of `src/authorship/post_commit.rs`, add `HashSet` if it is not already imported:

```rust
use std::collections::{HashMap, HashSet};
```

- [ ] **Step 2: Add the post-commit overlay helper**

Add this helper near other private helpers in `src/authorship/post_commit.rs`:

```rust
fn maybe_overlay_merge_conflict_resolution_authorship(
    repo: &Repository,
    parent_sha: &str,
    commit_sha: &str,
    authorship_log: AuthorshipLog,
) -> AuthorshipLog {
    let is_merge_commit = repo
        .find_commit(commit_sha.to_string())
        .map(|commit| commit.parent_count().unwrap_or(0) > 1)
        .unwrap_or(false);
    if !is_merge_commit {
        return authorship_log;
    }

    let changed_files = match repo.diff_changed_files(parent_sha, commit_sha) {
        Ok(files) => files,
        Err(error) => {
            tracing::debug!(
                "Skipping merge conflict resolution authorship overlay for {}: failed to diff {}..{}: {}",
                commit_sha,
                parent_sha,
                commit_sha,
                error
            );
            return authorship_log;
        }
    };
    if changed_files.is_empty() {
        return authorship_log;
    }
    let changed_files: HashSet<String> = changed_files.into_iter().collect();

    let Some(resolution_log) =
        crate::authorship::conflict_resolution::build_authorship_log_from_ai_checkpoints(
            repo,
            parent_sha,
            commit_sha,
            &changed_files,
        )
    else {
        return authorship_log;
    };

    crate::authorship::conflict_resolution::merge_conflict_resolution_authorship(
        Some(authorship_log),
        resolution_log,
        commit_sha,
    )
}
```

- [ ] **Step 3: Call the helper before note serialization**

In `post_commit_with_final_state(...)`, after:

```rust
authorship_log.metadata.base_commit_sha = commit_sha.clone();
hydrate_missing_prompt_metadata(repo, &mut authorship_log);
```

insert:

```rust
authorship_log = maybe_overlay_merge_conflict_resolution_authorship(
    repo,
    &parent_sha,
    &commit_sha,
    authorship_log,
);
```

Then keep the existing config/custom attribute/prompt storage handling after this point so merged prompt records receive the same metadata treatment as normal post-commit records.

- [ ] **Step 4: Run the target test again**

Run:

```bash
task test TEST_FILTER=test_merge_conflict_ai_resolution_combines_parent_arguments
```

Expected: PASS after implementation.

## Task 4: Add Human-Resolution Guard Test

**Files:**
- Modify: `tests/integration/merge_rebase.rs`

**Interfaces:**
- Consumes same test helpers as Task 1.
- Produces guard test proving overlay does not synthesize AI without AI checkpoint evidence.

- [ ] **Step 1: Add guard test**

Add this test before `reuse_tests_in_worktree!`:

```rust
#[test]
fn test_merge_conflict_human_resolution_does_not_synthesize_ai() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("conflict.txt");
    std::fs::write(&file_path, "base\n").unwrap();
    repo.stage_all_and_commit("initial").unwrap();
    let main_branch = repo.current_branch();

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    std::fs::write(&file_path, "feature\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "conflict.txt"])
        .unwrap();
    repo.stage_all_and_commit("feature AI change").unwrap();

    repo.git(&["checkout", &main_branch]).unwrap();
    std::fs::write(&file_path, "main\n").unwrap();
    repo.stage_all_and_commit("main human change").unwrap();

    let merge_result = repo.git(&["merge", "feature"]);
    assert!(merge_result.is_err(), "merge should conflict");

    std::fs::write(&file_path, "human resolved\n").unwrap();
    repo.git(&["add", "conflict.txt"]).unwrap();
    repo.stage_all_and_commit("merge resolved by human").unwrap();

    let mut file = repo.filename("conflict.txt");
    file.assert_lines_and_blame(crate::lines!["human resolved".human()]);
}
```

- [ ] **Step 2: Add the test to worktree reuse macro**

Update the macro:

```rust
crate::reuse_tests_in_worktree!(
    test_blame_after_merge_conflict_resolution,
    test_merge_conflict_ai_resolution_outside_session,
    test_merge_conflict_ai_resolution_combines_parent_arguments,
    test_merge_conflict_human_resolution_does_not_synthesize_ai,
);
```

- [ ] **Step 3: Run guard test**

Run:

```bash
task test TEST_FILTER=test_merge_conflict_human_resolution_does_not_synthesize_ai
```

Expected: PASS.

## Task 5: Run Focused Regression Suite

**Files:**
- No source changes expected.

**Interfaces:**
- Verifies merge fix does not regress nearby rewrite paths.

- [ ] **Step 1: Run merge conflict tests**

Run:

```bash
task test TEST_FILTER=merge_conflict
```

Expected: PASS.

- [ ] **Step 2: Run merge/rebase integration tests**

Run:

```bash
task test TEST_FILTER=merge_rebase
```

Expected: PASS.

- [ ] **Step 3: Run rebase conflict tests**

Run:

```bash
task test TEST_FILTER=rebase_conflict
```

Expected: PASS.

- [ ] **Step 4: Run squash merge tests**

Run:

```bash
task test TEST_FILTER=squash_merge
```

Expected: PASS.

- [ ] **Step 5: Build**

Run:

```bash
task build
```

Expected: PASS.

## Task 6: Final Quality Gate

**Files:**
- No source changes expected unless formatting/lint fixes are required.

**Interfaces:**
- Confirms project-level checks for implementation branch.

- [ ] **Step 1: Format**

Run:

```bash
task fmt
```

Expected: PASS or formatting changes only.

- [ ] **Step 2: Lint**

Run:

```bash
task lint
```

Expected: PASS.

- [ ] **Step 3: Review diff**

Run:

```powershell
$env:GIT_MASTER='1'; git diff -- src/authorship/conflict_resolution.rs src/authorship/mod.rs src/authorship/post_commit.rs tests/integration/merge_rebase.rs
```

Expected: diff only contains the conflict-resolution helper, post-commit merge overlay wiring, and tests.

## Implementation Notes

- If `PromptRecord` or `AuthorshipMetadata` differs from the code assumed in Task 2, adapt field names to the current structs instead of adding compatibility fields.
- If `LineRange::expand()` is unavailable in the current checkout, use the existing pattern from `virtual_attribution.rs` or `authorship_log.rs` to expand ranges.
- If `repo.diff_changed_files(parent_sha, commit_sha)` returns paths that differ in normalization from checkpoint entries, normalize using the same POSIX/NFC path convention used elsewhere in authorship note generation before comparing.
- Keep the helper fallback behavior: a failure to build merge-resolution attribution must not abort `git commit`.
- Do not implement the optional unmerged-file checkpoint relaxation in this plan. That should be a separate plan and test suite.

## Self-Review Checklist

- Spec coverage: The plan covers the design doc goals, non-goals, post-commit overlay, helper module, tests, and verification commands.
- Placeholder scan: No `TBD`, `TODO`, or open-ended “write tests” placeholders remain.
- Type consistency: New helper signatures consistently use `AuthorshipLog`, `Repository`, `HashSet<String>`, `base_commit`, and `commit_sha` across tasks.
