# resolve_git_var_identity Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `Repository::resolve_git_var_identity()` with an in-process env/config resolver while preserving the current identity shape and env-over-config precedence.

**Architecture:** Keep the change local to `src/git/repository.rs`. Resolve `GIT_COMMITTER_IDENT` and `GIT_AUTHOR_IDENT` by reading their corresponding environment variables first, then fall back to the existing `config_get_str("user.name")` / `config_get_str("user.email")` path. Reuse the existing public API surface (`git_author_identity()`, `git_commit_author_identity()`) and tighten coverage with one integration test extension plus parser-focused unit tests.

**Tech Stack:** Rust 2024, `cargo test` via `task test`, existing `TestRepo` integration harness, existing `ApiContext` identity path, existing `gix-config`-backed `config_get_str()` helper.

---

## File Structure

- `src/git/repository.rs`
  - owns `GitAuthorIdentity`, `parse_git_var_identity()`, `git_author_identity()`, `git_commit_author_identity()`, and `resolve_git_var_identity()`
  - will receive the in-process resolver and new parser unit tests
- `tests/integration/low_difficulty_git2_gix_task1.rs`
  - already covers config identity formatting and env-over-config precedence through `ApiContext`
  - will be extended to exercise the `GIT_AUTHOR_*` path explicitly so both public entry points remain covered

### Task 1: Lock in parser and author/committer behavior with failing tests

**Files:**
- Modify: `src/git/repository.rs`
- Modify: `tests/integration/low_difficulty_git2_gix_task1.rs`
- Test: `src/git/repository.rs`
- Test: `tests/integration/low_difficulty_git2_gix_task1.rs`

- [ ] **Step 1: Add parser unit tests to `src/git/repository.rs`**

Insert new tests near the existing `test_parse_git_version_*` block:

```rust
#[test]
fn test_parse_git_var_identity_standard_format() {
    let parsed = parse_git_var_identity("Taylor Dev <taylor@example.com> 1714118400 +0800\n");
    assert_eq!(parsed.name.as_deref(), Some("Taylor Dev"));
    assert_eq!(parsed.email.as_deref(), Some("taylor@example.com"));
}

#[test]
fn test_parse_git_var_identity_name_only() {
    let parsed = parse_git_var_identity("Taylor Dev\n");
    assert_eq!(parsed.name.as_deref(), Some("Taylor Dev"));
    assert_eq!(parsed.email, None);
}

#[test]
fn test_parse_git_var_identity_empty() {
    let parsed = parse_git_var_identity("   \n");
    assert_eq!(parsed.name, None);
    assert_eq!(parsed.email, None);
}
```

- [ ] **Step 2: Extend the existing integration test driver with an author-specific case**

Update the case switch in `tests/integration/low_difficulty_git2_gix_task1.rs` so it can dispatch a third isolated test case:

```rust
fn run_identity_case(case: &str) {
    let test_name = match case {
        "config" => "low_difficulty_task1_resolve_git_identity_matches_git_var_config_identity_format",
        "env-overrides" => "low_difficulty_task1_resolve_git_identity_prefers_env_over_repo_config",
        "author-env-overrides" => "low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config",
        other => panic!("unknown identity case: {other}"),
    };
    // existing child-process launch stays the same
}
```

- [ ] **Step 3: Add a failing integration test for `git_commit_author_identity()`**

Append this new test and helper to `tests/integration/low_difficulty_git2_gix_task1.rs`:

```rust
#[test]
fn low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("author-env-overrides") {
        run_commit_author_env_override_case();
        return;
    }
    run_identity_case("author-env-overrides");
}

fn run_commit_author_env_override_case() {
    let repo = TestRepo::new();
    write_file(&repo, "author.txt", "content\n");
    repo.stage_all_and_commit("initial").unwrap();
    repo.git(&["config", "user.name", "Repo User"]).unwrap();
    repo.git(&["config", "user.email", "repo@example.com"]).unwrap();

    let workdir = repo.path();
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();

    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Author Env User");
        std::env::set_var("GIT_AUTHOR_EMAIL", "author-env@example.com");
        std::env::remove_var("GIT_COMMITTER_NAME");
        std::env::remove_var("GIT_COMMITTER_EMAIL");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let repository = git_ai::git::repository::find_repository(&[])
        .expect("repository lookup should succeed");
    let actual = repository.git_commit_author_identity();

    assert_eq!(actual.formatted().as_deref(), Some("Author Env User <author-env@example.com>"));
}
```

- [ ] **Step 4: Run the targeted tests and verify the new author-path test fails for the expected reason**

Run:

```bash
task test TEST_FILTER=low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config
```

Expected: FAIL because the current implementation still shells out through `git var` and the new test has not yet been reconciled with the in-process path or imports.

- [ ] **Step 5: Run the parser unit tests and verify they fail only if not yet added/compiled correctly**

Run:

```bash
task test TEST_FILTER=test_parse_git_var_identity_
```

Expected: FAIL before implementation is complete if there are compile/test additions still pending; after the tests are correctly added, they should be ready to pass once compilation succeeds.

- [ ] **Step 6: Commit the red tests**

```bash
git add src/git/repository.rs tests/integration/low_difficulty_git2_gix_task1.rs
git commit -m "test: cover in-process git identity resolution"
```

### Task 2: Replace `git var` subprocess resolution with env/config lookup

**Files:**
- Modify: `src/git/repository.rs`
- Test: `tests/integration/low_difficulty_git2_gix_task1.rs`

- [ ] **Step 1: Add a tiny helper that maps `GIT_*_IDENT` to env variable names**

Add this helper near `resolve_git_var_identity()`:

```rust
fn git_identity_env_keys(git_var: &str) -> Option<(&'static str, &'static str)> {
    match git_var {
        "GIT_COMMITTER_IDENT" => Some(("GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL")),
        "GIT_AUTHOR_IDENT" => Some(("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL")),
        _ => None,
    }
}
```

- [ ] **Step 2: Replace the CLI branch inside `resolve_git_var_identity()` with direct env/config resolution**

Update the function to this shape:

```rust
fn resolve_git_var_identity(&self, git_var: &str) -> GitAuthorIdentity {
    let (name_env, email_env) = match git_identity_env_keys(git_var) {
        Some(keys) => keys,
        None => return GitAuthorIdentity::default(),
    };

    let name = std::env::var(name_env)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            self.config_get_str("user.name")
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
        });

    let email = std::env::var(email_env)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            self.config_get_str("user.email")
                .ok()
                .flatten()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
        });

    GitAuthorIdentity { name, email }
}
```

Also update the surrounding doc comments so they no longer claim the function “uses `git var`”, and add a migration note comment:

```rust
// Migrated from:
// - git var GIT_COMMITTER_IDENT
// - git var GIT_AUTHOR_IDENT
```
```

- [ ] **Step 3: Run the new author-specific test and verify it passes**

Run:

```bash
task test TEST_FILTER=low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config
```

Expected: PASS.

- [ ] **Step 4: Run the existing identity integration tests and verify no regression**

Run:

```bash
task test TEST_FILTER=low_difficulty_task1_resolve_git_identity_
```

Expected: PASS for both existing tests plus the new author-path test.

- [ ] **Step 5: Run the parser unit tests and verify green**

Run:

```bash
task test TEST_FILTER=test_parse_git_var_identity_
```

Expected: PASS.

- [ ] **Step 6: Commit the minimal implementation**

```bash
git add src/git/repository.rs tests/integration/low_difficulty_git2_gix_task1.rs
git commit -m "refactor: resolve git identity without git var subprocess"
```

### Task 3: Final targeted verification

**Files:**
- Modify: none expected
- Test: `src/git/repository.rs`
- Test: `tests/integration/low_difficulty_git2_gix_task1.rs`

- [ ] **Step 1: Run all low-difficulty identity tests together**

Run:

```bash
task test TEST_FILTER=low_difficulty_task1_
```

Expected: PASS.

- [ ] **Step 2: Run the broader repository-focused integration file slice**

Run:

```bash
task test TEST_FILTER=git_repository_comprehensive
```

Expected: PASS.

- [ ] **Step 3: Run a build to ensure repository.rs still compiles cleanly across the crate**

Run:

```bash
task build
```

Expected: build succeeds with exit code 0.

- [ ] **Step 4: Review diff for scope discipline**

Run:

```bash
git diff -- src/git/repository.rs tests/integration/low_difficulty_git2_gix_task1.rs
```

Expected: only the identity resolver, its comments, and directly related tests changed.

- [ ] **Step 5: Commit the final verification checkpoint**

```bash
git add src/git/repository.rs tests/integration/low_difficulty_git2_gix_task1.rs
git commit -m "test: verify repository identity migration"
```
