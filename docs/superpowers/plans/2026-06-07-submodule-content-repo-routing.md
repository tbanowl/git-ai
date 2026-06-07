# Submodule Content Repository Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route AI checkpoints and authorship notes for files inside initialized Git submodules to the submodule repository, even when the agent is launched from the parent repository.

**Architecture:** Keep command handling on the existing file-based repository grouping path. Change low-level repository ownership helpers so `.git` files are content repository boundaries, then add regression coverage for repository discovery, `path_is_in_workdir`, and the parent-CWD Claude submodule flow.

**Tech Stack:** Rust 2024, Git CLI-backed repository discovery, existing integration test harness in `tests/integration/repos`, `task` test/build/lint/format commands.

---

## File Structure

- Modify `tests/integration/git_repository_comprehensive.rs`: flip the existing submodule `.git` file expectation in `test_path_is_in_workdir()` from parent-owned to child-owned for content paths.
- Modify `tests/integration/multi_repo_workspace.rs`: update the module comment and add repository discovery/grouping tests for a real submodule layout.
- Modify `tests/integration/cross_repo_cwd_attribution.rs`: add the end-to-end parent-CWD, initialized-submodule, Claude checkpoint regression.
- Modify `src/git/repository.rs`: update repository boundary detection and file repository discovery. No new public API is required.

## Task 1: Add Repository Boundary Regression Tests

**Files:**
- Modify: `tests/integration/git_repository_comprehensive.rs`
- Modify: `tests/integration/multi_repo_workspace.rs`

- [ ] **Step 1: Update the existing `path_is_in_workdir` submodule expectation**

In `tests/integration/git_repository_comprehensive.rs`, find the block in `test_path_is_in_workdir()` that currently says submodule `.git` files are transparent and asserts `repo.path_is_in_workdir(&submodule_file)`.

Replace that block with:

```rust
    // Path inside a submodule (.git file, not directory) should return false
    // for content ownership. Submodule file contents belong to the submodule
    // repository, while the parent owns .gitmodules and the gitlink pointer.
    let submodule_dir = test_repo.path().join("my-submodule");
    fs::create_dir_all(submodule_dir.join("src")).unwrap();
    fs::write(
        submodule_dir.join(".git"),
        "gitdir: ../.git/modules/my-submodule\n",
    )
    .unwrap();
    let submodule_file = submodule_dir.join("src").join("lib.rs");
    fs::write(&submodule_file, "submodule content").unwrap();
    assert!(
        !repo.path_is_in_workdir(&submodule_file),
        "File inside a submodule (.git file, not directory) should return false for content ownership"
    );
```

- [ ] **Step 2: Update the `multi_repo_workspace` module comment**

In `tests/integration/multi_repo_workspace.rs`, replace the scenario comment line:

```rust
//! 3. Handling submodules correctly (should be ignored in favor of parent repo)
```

with:

```rust
//! 3. Handling submodules correctly (content files route to the submodule repo)
```

- [ ] **Step 3: Add a helper for raw git commands used by the submodule tests**

At the top of `tests/integration/multi_repo_workspace.rs`, replace:

```rust
use std::path::PathBuf;
```

with:

```rust
use std::path::{Path, PathBuf};
```

In `tests/integration/multi_repo_workspace.rs`, below `create_file()`, add:

```rust
fn run_git(path: &Path, args: &[&str]) -> Result<String, GitAiError> {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .map_err(|e| GitAiError::Generic(format!("Failed to run git {:?}: {}", args, e)))?;

    if !output.status.success() {
        return Err(GitAiError::Generic(format!(
            "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
            args,
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

- [ ] **Step 4: Add a helper that creates a real initialized submodule**

In `tests/integration/multi_repo_workspace.rs`, below `run_git()`, add:

```rust
fn create_parent_with_submodule(
    workspace: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf), GitAiError> {
    let remote_submodule = workspace.join("remote-submodule");
    init_git_repo(&remote_submodule)?;
    create_file(&remote_submodule.join("README.md"), "# Remote Submodule\n")?;
    run_git(&remote_submodule, &["add", "-A"])?;
    run_git(&remote_submodule, &["commit", "-m", "initial submodule"])?;

    let parent_repo = workspace.join("parent");
    init_git_repo(&parent_repo)?;
    create_file(&parent_repo.join("README.md"), "# Parent\n")?;
    run_git(&parent_repo, &["add", "-A"])?;
    run_git(&parent_repo, &["commit", "-m", "initial parent"])?;

    run_git(
        &parent_repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            remote_submodule.to_str().unwrap(),
            "vendor/submodule",
        ],
    )?;
    run_git(&parent_repo, &["commit", "-m", "add submodule"])?;

    let submodule_path = parent_repo.join("vendor").join("submodule");
    Ok((parent_repo, submodule_path, remote_submodule))
}
```

- [ ] **Step 5: Add `find_repository_for_file` coverage for real submodules**

In `tests/integration/multi_repo_workspace.rs`, after `test_find_repository_for_file_nested_repos()`, add:

```rust
#[test]
fn test_find_repository_for_file_real_submodule_prefers_submodule() {
    let workspace = create_unique_tmp_dir("git-ai-real-submodule-detect-test").unwrap();
    let (parent_repo, submodule_path, _remote_submodule) =
        create_parent_with_submodule(&workspace).unwrap();

    let submodule_file = submodule_path.join("src").join("lib.rs");
    create_file(&submodule_file, "pub fn generated() {}\n").unwrap();

    let result = find_repository_for_file(
        submodule_file.to_str().unwrap(),
        Some(workspace.to_str().unwrap()),
    );

    assert!(
        result.is_ok(),
        "Should find a repository for a file inside an initialized submodule"
    );
    let repo = result.unwrap();
    let workdir = repo.workdir().unwrap().canonicalize().unwrap();
    assert_eq!(
        workdir,
        submodule_path.canonicalize().unwrap(),
        "Submodule content file should resolve to the submodule repository"
    );

    let parent_file = parent_repo.join(".gitmodules");
    let parent_result = find_repository_for_file(
        parent_file.to_str().unwrap(),
        Some(workspace.to_str().unwrap()),
    )
    .expect(".gitmodules should resolve to the parent repository");
    assert_eq!(
        parent_result.workdir().unwrap().canonicalize().unwrap(),
        parent_repo.canonicalize().unwrap(),
        ".gitmodules should remain owned by the parent repository"
    );

    cleanup_tmp_dir(&workspace);
}
```

- [ ] **Step 6: Add `group_files_by_repository` coverage for mixed parent/submodule files**

In `tests/integration/multi_repo_workspace.rs`, after the test from Step 5, add:

```rust
#[test]
fn test_group_files_by_repository_splits_parent_and_real_submodule() {
    let workspace = create_unique_tmp_dir("git-ai-real-submodule-group-test").unwrap();
    let (parent_repo, submodule_path, _remote_submodule) =
        create_parent_with_submodule(&workspace).unwrap();

    let parent_file = parent_repo.join("parent.txt");
    let submodule_file = submodule_path.join("src").join("lib.rs");
    create_file(&parent_file, "parent content\n").unwrap();
    create_file(&submodule_file, "submodule content\n").unwrap();

    let paths = vec![
        parent_file.to_str().unwrap().to_string(),
        submodule_file.to_str().unwrap().to_string(),
    ];
    let (repo_files, orphan_files) =
        group_files_by_repository(&paths, Some(workspace.to_str().unwrap()));

    assert!(
        orphan_files.is_empty(),
        "Parent and submodule files should both be associated with repositories: {:?}",
        orphan_files
    );
    assert_eq!(
        repo_files.len(),
        2,
        "Parent and initialized submodule files should be grouped into separate repositories"
    );

    let parent_key = parent_repo.canonicalize().unwrap();
    let submodule_key = submodule_path.canonicalize().unwrap();
    let parent_files = repo_files
        .iter()
        .find(|(workdir, _)| workdir.canonicalize().unwrap() == parent_key)
        .map(|(_, (_repo, files))| files)
        .expect("Expected parent repo group");
    let submodule_files = repo_files
        .iter()
        .find(|(workdir, _)| workdir.canonicalize().unwrap() == submodule_key)
        .map(|(_, (_repo, files))| files)
        .expect("Expected submodule repo group");
    assert_eq!(parent_files.len(), 1);
    assert_eq!(submodule_files.len(), 1);

    cleanup_tmp_dir(&workspace);
}
```

- [ ] **Step 7: Run focused tests and confirm they fail for the intended reason**

Run:

```bash
task test TEST_FILTER=test_path_is_in_workdir
task test TEST_FILTER=test_find_repository_for_file_real_submodule_prefers_submodule
task test TEST_FILTER=test_group_files_by_repository_splits_parent_and_real_submodule
```

Expected before implementation:

- `test_path_is_in_workdir` fails because `repo.path_is_in_workdir(&submodule_file)` still returns `true`.
- `test_find_repository_for_file_real_submodule_prefers_submodule` fails because the returned workdir is the parent repo instead of the submodule.
- `test_group_files_by_repository_splits_parent_and_real_submodule` fails because parent and submodule files are grouped together.

- [ ] **Step 8: Commit the failing boundary tests**

```bash
git add tests/integration/git_repository_comprehensive.rs tests/integration/multi_repo_workspace.rs
git commit -m "test: cover submodule content repository routing"
```

## Task 2: Add Parent-CWD Claude Submodule Regression Test

**Files:**
- Modify: `tests/integration/cross_repo_cwd_attribution.rs`

- [ ] **Step 1: Add imports needed for raw Git command helpers**

At the top of `tests/integration/cross_repo_cwd_attribution.rs`, replace:

```rust
use std::path::PathBuf;
```

with:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;
```

- [ ] **Step 2: Add raw Git helpers for real submodule setup**

Below `create_unique_workspace()`, add:

```rust
fn run_git_checked(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?} in {}: {}", args, path.display(), e));

    if !output.status.success() {
        panic!(
            "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
            args,
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout).to_string()
}

fn init_plain_git_repo(path: &Path) {
    fs::create_dir_all(path).expect("failed to create git repo dir");
    run_git_checked(path, &["init"]);
    run_git_checked(path, &["config", "user.name", "Test User"]);
    run_git_checked(path, &["config", "user.email", "test@example.com"]);
}
```

- [ ] **Step 3: Add the end-to-end Claude submodule test**

After `test_claude_preset_nested_subrepo_pre_post_cycle()`, add:

```rust
/// Claude Code running in a parent repo and editing an initialized Git submodule.
/// The submodule has a `.git` file pointing into the parent's `.git/modules`,
/// so this specifically covers true submodule routing rather than an ordinary
/// nested repo with a `.git/` directory.
#[test]
fn test_claude_preset_parent_cwd_real_submodule_records_prompts() {
    let workspace = create_unique_workspace("git-ai-claude-real-submodule-test");

    let remote_submodule_path = workspace.join("remote-submodule");
    init_plain_git_repo(&remote_submodule_path);
    fs::write(remote_submodule_path.join("README.md"), "# Remote Submodule\n").unwrap();
    run_git_checked(&remote_submodule_path, &["add", "-A"]);
    run_git_checked(&remote_submodule_path, &["commit", "-m", "initial submodule"]);

    let parent_path = workspace.join("parent-repo");
    let parent = TestRepo::new_at_path(&parent_path);
    fs::write(parent_path.join("README.md"), "# Parent Repo\n").unwrap();
    parent.stage_all_and_commit("initial parent").unwrap();

    parent
        .git(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            remote_submodule_path.to_str().unwrap(),
            "vendor/submodule",
        ])
        .expect("submodule add should succeed");
    parent
        .stage_all_and_commit("add submodule")
        .expect("parent submodule pointer commit should succeed");

    let submodule_path = parent_path.join("vendor").join("submodule");
    let mut submodule = TestRepo::new_at_path(&submodule_path);
    submodule.patch_git_ai_config(|patch| {
        patch.exclude_prompts_in_repositories = Some(vec![]);
    });

    let transcript_path = submodule.canonical_path().join("claude-session.jsonl");
    let fixture = fixture_path("example-claude-code.jsonl");
    fs::copy(&fixture, &transcript_path).unwrap();

    let target_file = submodule.canonical_path().join("src").join("generated.ts");
    fs::create_dir_all(target_file.parent().unwrap()).unwrap();
    fs::write(
        &target_file,
        "export const generated = true;\n",
    )
    .unwrap();

    let hook_input = json!({
        "cwd": parent.canonical_path().to_string_lossy().to_string(),
        "hook_event_name": "PostToolUse",
        "transcript_path": transcript_path.to_string_lossy().to_string(),
        "tool_input": {
            "file_path": target_file.to_string_lossy().to_string()
        }
    })
    .to_string();

    submodule
        .git_ai_from_working_dir(
            &parent.canonical_path(),
            &["checkpoint", "claude", "--hook-input", &hook_input],
        )
        .expect("Claude checkpoint from parent CWD for real submodule should succeed");

    let submodule_working_log = submodule.current_working_logs();
    let submodule_ai_files = submodule_working_log
        .all_ai_touched_files()
        .unwrap_or_default();
    assert!(
        !submodule_ai_files.is_empty(),
        "Working log entries should exist in the real submodule when Claude runs from the parent CWD"
    );

    let parent_working_log = parent.current_working_logs();
    let parent_ai_files = parent_working_log.all_ai_touched_files().unwrap_or_default();
    assert!(
        parent_ai_files.is_empty(),
        "Parent working log should not claim submodule content edits: {:?}",
        parent_ai_files
    );

    let commit = submodule
        .stage_all_and_commit("add generated submodule code")
        .expect("submodule commit should succeed");
    assert!(
        !commit.authorship_log.metadata.prompts.is_empty(),
        "Submodule commit note should contain Claude prompt metadata"
    );
    assert!(
        !commit.authorship_log.attestations.is_empty(),
        "Submodule commit note should contain AI attestations"
    );

    let mut file = submodule.filename("src/generated.ts");
    file.assert_lines_and_blame(crate::lines![
        "export const generated = true;".ai()
    ]);

    let _ = fs::remove_dir_all(&workspace);
}
```

- [ ] **Step 4: Run the new end-to-end test and confirm it fails for the intended reason**

Run:

```bash
task test TEST_FILTER=test_claude_preset_parent_cwd_real_submodule_records_prompts
```

Expected before implementation:

- The submodule working log assertion fails because the checkpoint is routed to the parent repo or dropped.
- If the working log assertion is bypassed, the submodule commit note lacks prompts or attestations.

- [ ] **Step 5: Commit the failing end-to-end test**

```bash
git add tests/integration/cross_repo_cwd_attribution.rs
git commit -m "test: cover parent cwd claude edits in submodule"
```

## Task 3: Implement Nearest-Repository Content Routing

**Files:**
- Modify: `src/git/repository.rs`

- [ ] **Step 1: Update `path_is_in_workdir` documentation**

In `src/git/repository.rs`, replace the doc comment above `Repository::path_is_in_workdir()` with:

```rust
    /// Check if a content path is within the repository's working directory.
    ///
    /// Returns `false` for paths inside nested independent git repositories,
    /// including submodules represented by `.git` files, because those file
    /// contents belong to the nearest nested repository. Parent repository
    /// metadata such as `.gitmodules` and gitlink pointer changes are handled
    /// by Git index operations, not by this content-path predicate.
```

- [ ] **Step 2: Replace the boundary helper comments and implementation**

In `src/git/repository.rs`, replace the comment and body of `has_intervening_git_dir()` with:

```rust
/// Check if any directory between `workdir` and `file_path` contains a `.git`
/// entry that represents a separate git repository boundary.
///
/// Both `.git` directories and `.git` files are boundaries for content paths.
/// A `.git` file can represent a linked worktree or an initialized submodule;
/// in both cases, file contents below that directory belong to the nested
/// repository rather than the parent repository.
fn has_intervening_git_dir(file_path: &Path, workdir: &Path) -> bool {
    let Ok(relative) = file_path.strip_prefix(workdir) else {
        return false;
    };

    // Walk parent directories of the relative path (excluding the file itself
    // and the empty path). For "subrepo/src/file.ts" we check:
    //   workdir/subrepo/src/.git
    //   workdir/subrepo/.git
    let mut current = relative;
    loop {
        let Some(parent) = current.parent() else {
            break;
        };
        if parent.as_os_str().is_empty() {
            break;
        }
        let potential_git = workdir.join(parent).join(".git");
        if potential_git.is_dir() || potential_git.is_file() {
            return true;
        }
        current = parent;
    }
    false
}
```

- [ ] **Step 3: Remove the now-unused linked-worktree-only helper**

Delete the entire `is_linked_worktree_git_file()` function:

```rust
fn is_linked_worktree_git_file(git_file: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(git_file) else {
        return false;
    };
    // Format: "gitdir: <path>\n"
    let Some(gitdir) = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:").map(str::trim))
    else {
        return false;
    };
    // A linked worktree's gitdir resolves to something like
    // `/repo/.git/worktrees/<name>`.  A submodule's gitdir looks like
    // `../.git/modules/<name>`.
    gitdir.contains("/.git/worktrees/")
}
```

If the compiler reports the helper is still used elsewhere, keep it and remove only the call from `has_intervening_git_dir()`. Otherwise delete it to avoid dead code.

- [ ] **Step 4: Remove submodule skipping from `find_repository_for_file()`**

In `src/git/repository.rs`, replace this block:

```rust
        // Check for .git directory or file (file for submodules/worktrees)
        let git_path = dir.join(".git");
        if git_path.exists() {
            // Found a .git - but we need to check if this is a submodule
            // Submodules have a .git file (not directory) that points to the parent's .git/modules
            if git_path.is_file() {
                // This is a submodule - read the file to check if it points to modules/
                if let Ok(content) = std::fs::read_to_string(&git_path)
                    && content.contains("gitdir:")
                    && content.contains("/modules/")
                {
                    // This is a submodule, skip it and continue searching up
                    current_dir = dir.parent();
                    continue;
                }
            }

            // Found a real git repository, use find_repository_in_path
            return find_repository_in_path(&dir.to_string_lossy());
        }
```

with:

```rust
        // Check for .git directory or file. A .git file can be a linked worktree
        // or an initialized submodule; both are repositories for content paths.
        let git_path = dir.join(".git");
        if git_path.exists() {
            return find_repository_in_path(&dir.to_string_lossy());
        }
```

- [ ] **Step 5: Run the Task 1 focused tests**

Run:

```bash
task test TEST_FILTER=test_path_is_in_workdir
task test TEST_FILTER=test_find_repository_for_file_real_submodule_prefers_submodule
task test TEST_FILTER=test_group_files_by_repository_splits_parent_and_real_submodule
```

Expected after implementation: all three commands pass.

- [ ] **Step 6: Run the Task 2 end-to-end test**

Run:

```bash
task test TEST_FILTER=test_claude_preset_parent_cwd_real_submodule_records_prompts
```

Expected after implementation: the test passes, including non-empty submodule working log, empty parent working log, non-empty submodule prompt metadata, non-empty submodule attestations, and line-level blame.

- [ ] **Step 7: Commit the implementation**

```bash
git add src/git/repository.rs
git commit -m "fix: route submodule content to nearest repository"
```

## Task 4: Verify Nearby Behavior and Finish

**Files:**
- No code changes expected unless verification exposes failures.

- [ ] **Step 1: Run focused integration suites named in the spec**

Run:

```bash
task test TEST_FILTER=multi_repo_workspace
task test TEST_FILTER=cross_repo_cwd_attribution
task test TEST_FILTER=git_repository_comprehensive
```

Expected: all pass.

- [ ] **Step 2: Run build**

Run:

```bash
task build
```

Expected: build completes successfully.

- [ ] **Step 3: Run formatter**

Run:

```bash
task fmt
```

Expected: formatter exits successfully. If it changes files, inspect `git status --short`, then include those formatting changes in the final commit for Task 4.

- [ ] **Step 4: Run lint**

Run:

```bash
task lint
```

Expected: lint completes successfully.

- [ ] **Step 5: Check final worktree state**

Run:

```bash
git status --short
```

Expected: only intentional files are modified. If `task fmt` changed files, they should be the files already touched by this plan.

- [ ] **Step 6: Commit verification cleanup if needed**

If formatting or test adjustments changed files after Task 3, commit them:

```bash
git add tests/integration/git_repository_comprehensive.rs tests/integration/multi_repo_workspace.rs tests/integration/cross_repo_cwd_attribution.rs src/git/repository.rs
git commit -m "test: verify submodule content routing"
```

If Step 5 shows a clean worktree, skip this commit.

- [ ] **Step 7: Record final verification evidence**

In the final handoff, report the exact commands run and whether they passed:

```text
task test TEST_FILTER=multi_repo_workspace
task test TEST_FILTER=cross_repo_cwd_attribution
task test TEST_FILTER=git_repository_comprehensive
task build
task fmt
task lint
```

Also mention the commits produced by Tasks 1, 2, and 3.
