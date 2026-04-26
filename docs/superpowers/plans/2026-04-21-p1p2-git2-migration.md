# P1/P2 git2 Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace remaining CLI subprocess calls in `src/git/repository.rs` with in-process `git2` / `gix` API calls for the P1/P2 functions identified in `docs/git2-gix-对照表.md`.

**Architecture:** Each task is a surgical replacement of one function's body. The public API signatures stay identical — callers are unaffected. All changes are inside `src/git/repository.rs` unless noted. The `git2::Repository` handle is obtained via the existing `self.open_git2()` helper.

**Tech Stack:** Rust 1.93.0 (edition 2024), `git2` crate (already a dependency), `gix_config` (already used for config reads).

---

## Files

- Modify: `src/git/repository.rs` — all function bodies below

---

### Task 1: `is_bare_repository()` — read cached field instead of spawning a process

**Context:** `find_repository()` already calls `rev-parse --is-bare-repository` once at startup and stores the result implicitly (it determines the `workdir` path). We can cache it in the `Repository` struct and read it directly.

**Files:**
- Modify: `src/git/repository.rs:1300-1318` (struct definition), `src/git/repository.rs:1445-1452` (method body), `src/git/repository.rs:2531-2686` (find_repository constructor)

- [ ] **Step 1: Add `is_bare: bool` field to `Repository` struct**

In the struct definition (around line 1300):
```rust
pub struct Repository {
    global_args: Vec<String>,
    git_dir: PathBuf,
    git_common_dir: PathBuf,
    pub storage: RepoStorage,
    pub pre_command_base_commit: Option<String>,
    pub pre_command_refname: Option<String>,
    pub pre_reset_target_commit: Option<String>,
    pub pre_update_ref_refname: Option<String>,
    pub pre_update_ref_old_target: Option<String>,
    pub pre_update_ref_affects_checked_out_branch: Option<bool>,
    workdir: PathBuf,
    canonical_workdir: PathBuf,
    cached_author_identity: std::sync::OnceLock<GitAuthorIdentity>,
    is_bare: bool,   // <-- add this
}
```

- [ ] **Step 2: Set `is_bare` in `find_repository()` constructor**

In the `Ok(Repository { ... })` block at the end of `find_repository()` (around line 2671), add:
```rust
Ok(Repository {
    global_args: normalized_global_args,
    storage,
    git_dir,
    git_common_dir,
    pre_command_base_commit: None,
    pre_command_refname: None,
    pre_reset_target_commit: None,
    pre_update_ref_refname: None,
    pre_update_ref_old_target: None,
    pre_update_ref_affects_checked_out_branch: None,
    workdir,
    canonical_workdir,
    cached_author_identity: std::sync::OnceLock::new(),
    is_bare,   // <-- add this (variable already exists in find_repository scope)
})
```

- [ ] **Step 3: Replace `is_bare_repository()` body**

Replace the current implementation (lines 1445-1452):
```rust
pub fn is_bare_repository(&self) -> Result<bool, GitAiError> {
    Ok(self.is_bare)
}
```

- [ ] **Step 4: Build and check**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 5: Run tests**

```bash
cargo test --package git-ai --test simple_additions -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: cache is_bare in Repository struct, remove subprocess in is_bare_repository()"
```

---

### Task 2: `upstream_remote()` — remove `branch --show-current` subprocess

**Context:** `head()` already uses git2 and returns a `Reference` whose `ref_name` is e.g. `refs/heads/main`. We can parse the branch name from that instead of spawning `git branch --show-current`. Config reading already uses `gix_config` via `self.config_get_str()`.

**Files:**
- Modify: `src/git/repository.rs:2056-2068`

- [ ] **Step 1: Replace `upstream_remote()` body**

Current code (lines 2056-2068):
```rust
pub fn upstream_remote(&self) -> Result<Option<String>, GitAiError> {
    let mut args = self.global_args_for_exec();
    args.push("branch".to_string());
    args.push("--show-current".to_string());
    let output = exec_git(&args)?;
    let branch = String::from_utf8(output.stdout)?.trim().to_string();
    if branch.is_empty() {
        return Ok(None);
    }
    let config_key = format!("branch.{}.remote", branch);
    self.config_get_str(&config_key)
}
```

Replace with:
```rust
pub fn upstream_remote(&self) -> Result<Option<String>, GitAiError> {
    let head = match self.head() {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    // ref_name is e.g. "refs/heads/main"; extract the short branch name
    let branch = match head.ref_name.strip_prefix("refs/heads/") {
        Some(b) => b.to_string(),
        None => return Ok(None), // detached HEAD
    };
    let config_key = format!("branch.{}.remote", branch);
    self.config_get_str(&config_key)
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 3: Run tests**

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: upstream_remote() uses git2 head() instead of branch --show-current subprocess"
```

---

### Task 3: `remotes()` — replace `git remote` subprocess with git2

**Context:** `git2::Repository::remotes()` returns a `StringArray` of remote names directly.

**Files:**
- Modify: `src/git/repository.rs:1524-1531`

- [ ] **Step 1: Replace `remotes()` body**

Current code:
```rust
pub fn remotes(&self) -> Result<Vec<String>, GitAiError> {
    let mut args = self.global_args_for_exec();
    args.push("remote".to_string());
    let output = exec_git(&args)?;
    let remotes = String::from_utf8(output.stdout)?;
    Ok(remotes.trim().split("\n").map(|s| s.to_string()).collect())
}
```

Replace with:
```rust
pub fn remotes(&self) -> Result<Vec<String>, GitAiError> {
    let g2repo = self.open_git2().map_err(|e| GitAiError::Generic(e.to_string()))?;
    let arr = g2repo.remotes().map_err(|e| GitAiError::Generic(e.to_string()))?;
    Ok(arr.iter().filter_map(|s| s.map(|n| n.to_string())).collect())
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 3: Run tests**

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: remotes() uses git2 instead of git remote subprocess"
```

---

### Task 4: `remotes_with_urls()` — replace `git remote -v` subprocess with git2

**Context:** `git2::Repository::remotes()` gives names; `find_remote(name)` gives fetch/push URLs. We only need the fetch URL (same dedup logic as before).

**Files:**
- Modify: `src/git/repository.rs:1534-1558`

- [ ] **Step 1: Replace `remotes_with_urls()` body**

Current code:
```rust
pub fn remotes_with_urls(&self) -> Result<Vec<(String, String)>, GitAiError> {
    let mut args = self.global_args_for_exec();
    args.push("remote".to_string());
    args.push("-v".to_string());
    let output = exec_git(&args)?;
    let remotes_output = String::from_utf8(output.stdout)?;
    let mut remotes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in remotes_output.trim().split("\n").filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let name = parts[0].to_string();
            let url = parts[1].to_string();
            if seen.insert(name.clone()) {
                remotes.push((name, url));
            }
        }
    }
    Ok(remotes)
}
```

Replace with:
```rust
pub fn remotes_with_urls(&self) -> Result<Vec<(String, String)>, GitAiError> {
    let g2repo = self.open_git2().map_err(|e| GitAiError::Generic(e.to_string()))?;
    let names = g2repo.remotes().map_err(|e| GitAiError::Generic(e.to_string()))?;
    let mut remotes = Vec::new();
    for name in names.iter().filter_map(|s| s) {
        if let Ok(remote) = g2repo.find_remote(name) {
            let url = remote.url().unwrap_or("").to_string();
            remotes.push((name.to_string(), url));
        }
    }
    Ok(remotes)
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 3: Run tests**

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: remotes_with_urls() uses git2 find_remote() instead of git remote -v subprocess"
```

---

### Task 5: `remote_head()` — replace `symbolic-ref` subprocess with git2

**Context:** `refs/remotes/<remote>/HEAD` is a symbolic ref. `git2::Repository::find_reference()` + `Reference::symbolic_target()` gives us the target. We then strip the remote prefix to get the short name (e.g. `origin/main` → `main`).

**Files:**
- Modify: `src/git/repository.rs:1745-1753`

- [ ] **Step 1: Replace `remote_head()` body**

Current code:
```rust
pub fn remote_head(&self, remote_name: &str) -> Result<String, GitAiError> {
    let mut args = self.global_args_for_exec();
    args.push("symbolic-ref".to_string());
    args.push(format!("refs/remotes/{}/HEAD", remote_name));
    args.push("--short".to_string());
    let output = exec_git(&args)?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
```

Replace with:
```rust
pub fn remote_head(&self, remote_name: &str) -> Result<String, GitAiError> {
    let g2repo = self.open_git2().map_err(|e| GitAiError::Generic(e.to_string()))?;
    let refname = format!("refs/remotes/{}/HEAD", remote_name);
    let reference = g2repo
        .find_reference(&refname)
        .map_err(|e| GitAiError::Generic(e.to_string()))?;
    let target = reference
        .symbolic_target()
        .ok_or_else(|| GitAiError::Generic(format!("refs/remotes/{}/HEAD is not a symbolic ref", remote_name)))?;
    // target is e.g. "refs/remotes/origin/main"; return short form "origin/main"
    let short = target
        .strip_prefix("refs/remotes/")
        .unwrap_or(target)
        .to_string();
    Ok(short)
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 3: Run tests**

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: remote_head() uses git2 symbolic_target() instead of symbolic-ref subprocess"
```

---

### Task 6: `get_all_staged_files_content()` — replace concurrent `git show :<path>` with git2 index blob reads

**Context:** This function currently spawns up to 30 concurrent `git show :<path>` subprocesses. The index is already open via `gix_index` in `get_all_staged_file_blob_oids()`. We can use `git2::Repository::index()` to get blob OIDs and then `find_blob()` to read content — all in-process, no concurrency needed.

**Files:**
- Modify: `src/git/repository.rs:2185-2229`

- [ ] **Step 1: Replace `get_all_staged_files_content()` body**

Current code (lines 2185-2229) uses `smol` async + semaphore + concurrent `exec_git`. Replace entirely:

```rust
pub fn get_all_staged_files_content(
    &self,
    file_paths: &[String],
) -> Result<HashMap<String, String>, GitAiError> {
    let g2repo = self.open_git2().map_err(|e| GitAiError::Generic(e.to_string()))?;
    let mut index = g2repo.index().map_err(|e| GitAiError::Generic(e.to_string()))?;
    index.read(true).map_err(|e| GitAiError::Generic(e.to_string()))?;

    let mut result = HashMap::new();
    for file_path in file_paths {
        let entry = match index.get_path(std::path::Path::new(file_path), 0) {
            Some(e) => e,
            None => continue,
        };
        let blob = match g2repo.find_blob(entry.id) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(content) = std::str::from_utf8(blob.content()) {
            result.insert(file_path.clone(), content.to_string());
        }
    }
    Ok(result)
}
```

- [ ] **Step 2: Check if `smol`/`futures` imports become unused**

```bash
cargo build 2>&1 | grep "unused import"
```

If `use futures::future::join_all` or `use std::sync::Arc` are now unused (they were only in this function body), they will be flagged. Remove them from the function — they were local `use` statements inside the function body, so they disappear automatically with the replacement.

- [ ] **Step 3: Build clean**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors, no warnings about unused imports.

- [ ] **Step 4: Run tests**

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -20
```
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: get_all_staged_files_content() reads blobs via git2 index instead of concurrent git show subprocesses"
```

---

### Task 7: `find_repository()` — replace two `rev-parse` subprocesses with `git2::Repository::discover()`

**Context:** `find_repository()` currently spawns two subprocesses at startup:
1. `git rev-parse --is-bare-repository --git-dir --git-common-dir`
2. `git rev-parse --show-toplevel` (non-bare repos only)

`git2::Repository::discover(path)` performs the same discovery in-process and exposes `is_bare()`, `path()` (git_dir), `commondir()` (git_common_dir), and `workdir()` (show-toplevel equivalent).

**Prerequisite:** Task 1 must be done first (adds the `is_bare` field to the `Repository` struct).

**Note on libgit2 trailing slashes:** `repo.path()` and `repo.commondir()` return paths with a trailing separator (e.g. `/path/to/.git/`). Use `.components().collect::<PathBuf>()` to normalize away the trailing separator.

**Files:**
- Modify: `src/git/repository.rs:2531-2686`

- [ ] **Step 1: Replace `find_repository()` body**

Replace the current implementation (lines 2531-2686) with:
```rust
pub fn find_repository(global_args: &[String]) -> Result<Repository, GitAiError> {
    let find_repository_start = Instant::now();
    let command_base_dir = resolve_command_base_dir(global_args)?;

    let g2repo = git2::Repository::discover(&command_base_dir)
        .map_err(|e| GitAiError::Generic(format!("git2 discover failed: {}", e)))?;

    let is_bare = g2repo.is_bare();

    // libgit2 appends a trailing path separator; normalize via .components().collect()
    let git_dir: PathBuf = g2repo.path().components().collect();
    let git_common_dir: PathBuf = g2repo.commondir().components().collect();

    if !git_dir.is_dir() {
        return Err(GitAiError::Generic(format!(
            "Git directory does not exist: {}",
            git_dir.display()
        )));
    }
    if !git_common_dir.is_dir() {
        return Err(GitAiError::Generic(format!(
            "Git common directory does not exist: {}",
            git_common_dir.display()
        )));
    }

    let workdir = if is_bare {
        git_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
            GitAiError::Generic(format!(
                "Git directory has no parent: {}",
                git_dir.display()
            ))
        })?
    } else {
        g2repo
            .workdir()
            .ok_or_else(|| GitAiError::Generic("Non-bare repository has no workdir".to_string()))?
            .components()
            .collect()
    };

    if !workdir.is_dir() {
        return Err(GitAiError::Generic(format!(
            "Work directory does not exist: {}",
            workdir.display()
        )));
    }

    // Ensure all internal git commands use a stable repository root consistently.
    let mut normalized_global_args = global_args.to_owned();
    let command_root = if is_bare {
        git_dir.display().to_string()
    } else {
        workdir.display().to_string()
    };

    if normalized_global_args.is_empty() {
        normalized_global_args = vec!["-C".to_string(), command_root];
    } else if normalized_global_args.len() == 2
        && normalized_global_args[0] == "-C"
        && normalized_global_args[1] != command_root
    {
        normalized_global_args[1] = command_root;
    }

    let canonical_workdir = workdir.canonicalize().map_err(|e| {
        GitAiError::Generic(format!(
            "Failed to canonicalize working directory {}: {}",
            workdir.display(),
            e
        ))
    })?;

    let worktree_ai_dir = worktree_storage_ai_dir(&git_dir, &git_common_dir);
    let storage = if worktree_ai_dir == git_dir.join("ai") {
        RepoStorage::for_repo_path(&git_dir, &workdir)?
    } else {
        RepoStorage::for_isolated_worktree_storage(&worktree_ai_dir, &workdir)?
    };

    tracing::debug!(
        "[find_repository] cost {}ms",
        find_repository_start.elapsed().as_millis()
    );

    Ok(Repository {
        global_args: normalized_global_args,
        storage,
        git_dir,
        git_common_dir,
        pre_command_base_commit: None,
        pre_command_refname: None,
        pre_reset_target_commit: None,
        pre_update_ref_refname: None,
        pre_update_ref_old_target: None,
        pre_update_ref_affects_checked_out_branch: None,
        workdir,
        canonical_workdir,
        cached_author_identity: std::sync::OnceLock::new(),
        is_bare,  // field added by Task 1
    })
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```
Expected: no errors.

- [ ] **Step 3: Run tests** (including worktree and bare-repo tests)

```bash
cargo test --package git-ai -- --nocapture 2>&1 | tail -30
```
Expected: all pass, especially `find_repository_in_path_supports_bare_repositories`, `find_repository_in_path_bare_repo_can_read_head_gitattributes`, and `find_repository_in_path_worktree_uses_common_dir_for_isolated_storage`.

- [ ] **Step 4: Commit**

```bash
git add src/git/repository.rs
git commit -m "perf: find_repository() uses git2::discover() instead of two rev-parse subprocesses"
```

---

## Self-Review

**Spec coverage:**
- `is_bare_repository()` ✅ Task 1
- `upstream_remote()` ✅ Task 2
- `remotes()` ✅ Task 3
- `remotes_with_urls()` ✅ Task 4
- `remote_head()` ✅ Task 5
- `get_all_staged_files_content()` ✅ Task 6
- `find_repository()` ✅ Task 7
- `resolve_author_spec()` — intentionally excluded (CLI one-liner covers complex logic; migration cost > benefit per 对照表)

**Placeholder scan:** None found. All steps contain complete replacement code.

**Type consistency:** All functions keep identical signatures. `open_git2()` returns `git2::Repository` — consistent across all tasks. `GitAiError::Generic(e.to_string())` error mapping pattern matches existing codebase style.

**Task 7 dependency:** Task 7 must run after Task 1 (which adds the `is_bare` struct field). Tasks 2-6 are independent of Task 7.
