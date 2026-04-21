//! Comprehensive tests for src/git/repository.rs
//!
//! This test suite covers the core git operations layer including:
//! - Repository initialization and discovery
//! - Git command execution and error handling
//! - HEAD operations and branch management
//! - Commit operations and traversal
//! - Config get/set operations
//! - Pathspec validation and filtering
//! - Rewrite log operations
//! - Error handling and edge cases
//! - Working directory operations
//! - Bare repository support

use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use git_ai::git::repository::{find_repository, find_repository_in_path};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

// ============================================================================
// Repository Discovery and Initialization Tests
// ============================================================================

#[test]
fn test_find_repository_in_valid_repo() {
    let repo = TestRepo::new();

    // Create a commit to ensure it's a valid repo
    let mut file = repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // Should successfully find repository
    let found_repo =
        find_repository(&["-C".to_string(), repo.path().to_str().unwrap().to_string()]);

    assert!(found_repo.is_ok(), "Should find valid repository");
}

#[test]
fn test_find_repository_in_subdirectory() {
    let repo = TestRepo::new();

    // Create subdirectory
    let subdir = repo.path().join("subdir");
    fs::create_dir(&subdir).unwrap();

    // Should find repository from subdirectory
    let found_repo = find_repository(&["-C".to_string(), subdir.to_str().unwrap().to_string()]);

    assert!(
        found_repo.is_ok(),
        "Should find repository from subdirectory"
    );
}

#[test]
fn test_find_repository_in_nested_subdirectory() {
    let repo = TestRepo::new();

    // Create nested subdirectories
    let nested = repo.path().join("a").join("b").join("c");
    fs::create_dir_all(&nested).unwrap();

    // Should find repository from deeply nested subdirectory
    let found_repo = find_repository(&["-C".to_string(), nested.to_str().unwrap().to_string()]);

    assert!(
        found_repo.is_ok(),
        "Should find repository from nested subdirectory"
    );
}

#[test]
fn test_find_repository_for_bare_repo() {
    let bare_repo = TestRepo::new_bare();

    let found_repo = find_repository(&[
        "-C".to_string(),
        bare_repo.path().to_str().unwrap().to_string(),
    ]);

    assert!(found_repo.is_ok(), "Should find bare repository");

    let repo = found_repo.unwrap();
    assert!(
        repo.is_bare_repository().unwrap(),
        "Should detect bare repository"
    );
}

#[test]
fn test_repository_path_methods() {
    let test_repo = TestRepo::new();
    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // path() should always point at a valid git directory.
    let git_path = repo.path();
    assert!(git_path.is_dir(), "path() should return a git directory");
    if git_path == repo.common_dir() {
        assert!(
            git_path.ends_with(".git"),
            "non-worktree path() should return .git directory"
        );
    } else {
        assert!(
            git_path.to_string_lossy().contains("/worktrees/")
                || git_path
                    .components()
                    .any(|c| c.as_os_str() == std::ffi::OsStr::new("worktrees")),
            "worktree path() should resolve to a linked worktree git dir"
        );
    }

    // Test workdir() returns repository root (use canonical paths for macOS /var vs /private/var)
    let workdir = repo.workdir().unwrap();
    let canonical_workdir = workdir.canonicalize().unwrap();
    let canonical_test_path = test_repo.path().canonicalize().unwrap();
    assert_eq!(
        canonical_workdir, canonical_test_path,
        "workdir() should return repository root"
    );
}

#[test]
fn test_canonical_workdir() {
    let test_repo = TestRepo::new();
    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let canonical = repo.canonical_workdir();
    assert!(
        canonical.is_absolute(),
        "Canonical workdir should be absolute"
    );
}

#[test]
fn test_path_is_in_workdir() {
    let test_repo = TestRepo::new();
    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Path inside workdir - create the file so it can be canonicalized
    let inside = test_repo.path().join("file.txt");
    fs::write(&inside, "test content").unwrap();
    assert!(
        repo.path_is_in_workdir(&inside),
        "File in workdir should return true"
    );

    // Path outside workdir
    let outside = Path::new("/tmp/outside.txt");
    assert!(
        !repo.path_is_in_workdir(outside),
        "File outside workdir should return false"
    );

    // Path inside a nested subrepo (has its own .git/ directory) should return false
    let nested_repo_dir = test_repo.path().join("nested-repo");
    fs::create_dir_all(nested_repo_dir.join("src")).unwrap();
    // Initialize a real git repo in the nested directory
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&nested_repo_dir)
        .output()
        .expect("failed to git init nested repo");
    let nested_file = nested_repo_dir.join("src").join("nested.txt");
    fs::write(&nested_file, "nested content").unwrap();
    assert!(
        !repo.path_is_in_workdir(&nested_file),
        "File inside a nested subrepo (with its own .git/ dir) should return false"
    );

    // Path directly in the nested repo root should also return false
    let nested_root_file = nested_repo_dir.join("root.txt");
    fs::write(&nested_root_file, "root content").unwrap();
    assert!(
        !repo.path_is_in_workdir(&nested_root_file),
        "File at root of nested subrepo should return false"
    );

    // Path in a subdirectory (no nested .git/) should still return true
    let subdir = test_repo.path().join("regular-subdir");
    fs::create_dir_all(&subdir).unwrap();
    let subdir_file = subdir.join("file.txt");
    fs::write(&subdir_file, "subdir content").unwrap();
    assert!(
        repo.path_is_in_workdir(&subdir_file),
        "File in a regular subdirectory (no .git/) should return true"
    );

    // Path inside a submodule (.git file, not directory) should return true
    // Submodules are transparent to the parent repo
    let submodule_dir = test_repo.path().join("my-submodule");
    fs::create_dir_all(submodule_dir.join("src")).unwrap();
    // Simulate a submodule by creating a .git *file* (not directory)
    fs::write(
        submodule_dir.join(".git"),
        "gitdir: ../.git/modules/my-submodule\n",
    )
    .unwrap();
    let submodule_file = submodule_dir.join("src").join("lib.rs");
    fs::write(&submodule_file, "submodule content").unwrap();
    assert!(
        repo.path_is_in_workdir(&submodule_file),
        "File inside a submodule (.git file, not directory) should return true"
    );

    // Non-existent file path inside a nested subrepo should return false
    // (exercises the normalized fallback path since canonicalize() will fail)
    let nonexistent = nested_repo_dir.join("does-not-exist").join("phantom.txt");
    assert!(
        !repo.path_is_in_workdir(&nonexistent),
        "Non-existent file inside a nested subrepo should return false (fallback path)"
    );

    // Non-existent file path in the repo (no nested .git) should return true
    let nonexistent_in_repo = test_repo.path().join("not-yet-created.txt");
    assert!(
        repo.path_is_in_workdir(&nonexistent_in_repo),
        "Non-existent file in the repo (no nested .git/) should return true (fallback path)"
    );
}

// ============================================================================
// HEAD and Reference Tests
// ============================================================================

#[test]
fn test_head_on_main_branch() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    let name = head.name().unwrap();

    // Should be on main or master
    assert!(
        name.contains("main") || name.contains("master"),
        "HEAD should be on main/master branch, got: {}",
        name
    );
}

#[test]
fn test_head_on_feature_branch() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    // Create and checkout feature branch
    test_repo.git(&["checkout", "-b", "feature"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    let shorthand = head.shorthand().unwrap();

    assert_eq!(shorthand, "feature", "HEAD should be on feature branch");
}

#[test]
fn test_head_target() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    let target = head.target().unwrap();

    assert_eq!(
        target, commit.commit_sha,
        "HEAD target should match commit SHA"
    );
}

#[test]
fn test_reference_is_branch() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    assert!(head.is_branch(), "HEAD should be a branch");
}

#[test]
fn test_find_reference() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Get full ref name from HEAD
    let head = repo.head().unwrap();
    let ref_name = head.name().unwrap();

    // Find reference by name
    let found_ref = repo.find_reference(ref_name);
    assert!(found_ref.is_ok(), "Should find reference by full name");
}

// ============================================================================
// Commit Operations and Traversal Tests
// ============================================================================

#[test]
fn test_find_commit() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha.clone());
    assert!(commit.is_ok(), "Should find commit by SHA");

    let commit = commit.unwrap();
    assert_eq!(
        commit.id(),
        commit_info.commit_sha,
        "Commit ID should match"
    );
}

#[test]
fn test_commit_summary() {
    let test_repo = TestRepo::new();

    // Create commit with message
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo
        .stage_all_and_commit("Test summary message")
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let summary = commit.summary().unwrap();

    assert_eq!(
        summary, "Test summary message",
        "Summary should match commit message"
    );
}

#[test]
fn test_commit_body() {
    let test_repo = TestRepo::new();

    // Create commit with multi-line message
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.git(&["add", "-A"]).unwrap();

    let message = "Summary line\n\nBody line 1\nBody line 2";
    test_repo.git(&["commit", "-m", message]).unwrap();

    let commit_sha = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_sha).unwrap();
    let body = commit.body().unwrap();

    assert!(
        body.contains("Body line 1"),
        "Body should contain first body line"
    );
    assert!(
        body.contains("Body line 2"),
        "Body should contain second body line"
    );
}

#[test]
fn test_commit_parent() {
    let test_repo = TestRepo::new();

    // Create two commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content1".human()]);
    let first = test_repo.stage_all_and_commit("First commit").unwrap();

    file.set_contents(crate::lines!["content2".human()]);
    let second = test_repo.stage_all_and_commit("Second commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(second.commit_sha).unwrap();
    let parent = commit.parent(0).unwrap();

    assert_eq!(
        parent.id(),
        first.commit_sha,
        "Parent should be first commit"
    );
}

#[test]
fn test_commit_parents_iterator() {
    let test_repo = TestRepo::new();

    // Create commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content1".human()]);
    test_repo.stage_all_and_commit("First commit").unwrap();

    file.set_contents(crate::lines!["content2".human()]);
    test_repo.stage_all_and_commit("Second commit").unwrap();

    let commit_sha = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_sha).unwrap();
    let parents: Vec<_> = commit.parents().collect();

    assert_eq!(parents.len(), 1, "Should have one parent");
}

#[test]
fn test_commit_parent_count() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let first = test_repo.stage_all_and_commit("First commit").unwrap();

    // Create second commit
    file.set_contents(crate::lines!["content2".human()]);
    test_repo.stage_all_and_commit("Second commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Initial commit has no parents
    let first_commit = repo.find_commit(first.commit_sha).unwrap();
    assert_eq!(
        first_commit.parent_count().unwrap(),
        0,
        "Initial commit should have no parents"
    );

    // Second commit has one parent
    let head_sha = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let second_commit = repo.find_commit(head_sha).unwrap();
    assert_eq!(
        second_commit.parent_count().unwrap(),
        1,
        "Second commit should have one parent"
    );
}

#[test]
fn test_commit_tree() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree();

    assert!(tree.is_ok(), "Should get tree from commit");
}

#[test]
fn test_revparse_single() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Revparse HEAD
    let obj = repo.revparse_single("HEAD");
    assert!(obj.is_ok(), "Should revparse HEAD");
}

#[test]
fn test_revparse_single_with_relative_ref() {
    let test_repo = TestRepo::new();

    // Create two commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content1".human()]);
    test_repo.stage_all_and_commit("First commit").unwrap();

    file.set_contents(crate::lines!["content2".human()]);
    test_repo.stage_all_and_commit("Second commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Revparse HEAD~1
    let obj = repo.revparse_single("HEAD~1");
    assert!(obj.is_ok(), "Should revparse HEAD~1");
}

#[test]
fn test_object_peel_to_commit() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let obj = repo.revparse_single("HEAD").unwrap();
    let commit = obj.peel_to_commit();

    assert!(commit.is_ok(), "Should peel object to commit");
}

// ============================================================================
// Tree and Blob Tests
// ============================================================================

#[test]
fn test_tree_get_path() {
    let test_repo = TestRepo::new();

    // Create file and commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("test.txt"));

    assert!(entry.is_ok(), "Should find file in tree");
}

#[test]
fn test_tree_get_path_nested() {
    let test_repo = TestRepo::new();

    // Create nested file
    fs::create_dir(test_repo.path().join("subdir")).unwrap();
    let mut file = test_repo.filename("subdir/nested.txt");
    file.set_contents(crate::lines!["nested content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("subdir/nested.txt"));

    assert!(entry.is_ok(), "Should find nested file in tree");
}

#[test]
fn test_tree_get_path_nonexistent() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("nonexistent.txt"));

    assert!(entry.is_err(), "Should not find nonexistent file in tree");
}

#[test]
fn test_find_blob() {
    let test_repo = TestRepo::new();

    // Create file and commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("test.txt")).unwrap();
    let blob = repo.find_blob(entry.id());

    assert!(blob.is_ok(), "Should find blob");
}

#[test]
fn test_blob_content() {
    let test_repo = TestRepo::new();

    // Create file and commit
    let mut file = test_repo.filename("test.txt");
    let content = "test content line";
    file.set_contents(crate::lines![content.human()]);
    let commit_info = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("test.txt")).unwrap();
    let blob = repo.find_blob(entry.id()).unwrap();
    let blob_content = blob.content().unwrap();

    let blob_str = String::from_utf8(blob_content).unwrap();
    assert!(
        blob_str.contains(content),
        "Blob content should match file content"
    );
}

// ============================================================================
// Config Operations Tests
// ============================================================================

#[test]
fn test_config_get_str() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Get user.name which is set in test repo
    let name = repo.config_get_str("user.name");
    assert!(name.is_ok(), "Should get config value");

    let name = name.unwrap();
    assert!(name.is_some(), "user.name should be set");
    assert_eq!(
        name.unwrap(),
        "Test User",
        "user.name should be 'Test User'"
    );
}

#[test]
fn test_config_get_str_nonexistent() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Get nonexistent config
    let result = repo.config_get_str("nonexistent.config.key");
    assert!(result.is_ok(), "Should not error on nonexistent key");

    let value = result.unwrap();
    assert!(value.is_none(), "Nonexistent key should return None");
}

#[test]
fn test_config_get_regexp() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Get all user.* configs
    let configs = repo.config_get_regexp("user\\..*");
    assert!(configs.is_ok(), "Should get matching configs");

    let configs = configs.unwrap();
    assert!(
        !configs.is_empty(),
        "Should have at least one user.* config"
    );
    assert!(
        configs.contains_key("user.name"),
        "Should contain user.name"
    );
}

#[test]
fn test_git_version() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let version = repo.git_version();
    assert!(version.is_some(), "Should get git version");

    let (major, _minor, _patch) = version.unwrap();
    assert!(major >= 2, "Git major version should be at least 2");
}

#[test]
fn test_git_supports_ignore_revs_file() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Most modern git versions support this (added in 2.23.0)
    let supports = repo.git_supports_ignore_revs_file();
    let expected = if let Some((major, minor, _)) = repo.git_version() {
        major > 2 || (major == 2 && minor >= 23)
    } else {
        true
    };
    assert_eq!(
        supports, expected,
        "ignore-revs-file support should match git version threshold"
    );
}

// ============================================================================
// Remote Operations Tests
// ============================================================================

#[test]
fn test_remotes_empty() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let remotes = repo.remotes().unwrap();
    assert!(
        remotes.is_empty() || remotes == vec!["".to_string()],
        "New repo should have no remotes"
    );
}

#[test]
fn test_remotes_with_origin() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    let repo = find_repository(&[
        "-C".to_string(),
        mirror.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let remotes = repo.remotes().unwrap();
    assert!(
        remotes.contains(&"origin".to_string()),
        "Cloned repo should have origin remote"
    );
}

#[test]
fn test_remotes_with_urls() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    let repo = find_repository(&[
        "-C".to_string(),
        mirror.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let remotes_with_urls = repo.remotes_with_urls().unwrap();
    assert!(
        !remotes_with_urls.is_empty(),
        "Should have remotes with URLs"
    );

    let has_origin = remotes_with_urls
        .iter()
        .any(|(name, _url)| name == "origin");
    assert!(has_origin, "Should have origin remote with URL");
}

#[test]
fn test_get_default_remote() {
    let (mirror, _upstream) = TestRepo::new_with_remote();

    let repo = find_repository(&[
        "-C".to_string(),
        mirror.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let default_remote = repo.get_default_remote().unwrap();
    assert!(default_remote.is_some(), "Should have default remote");
    assert_eq!(
        default_remote.unwrap(),
        "origin",
        "Default remote should be origin"
    );
}

#[test]
fn test_get_default_remote_no_remotes() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let default_remote = repo.get_default_remote().unwrap();
    // New repos might have an empty string as a remote or None
    assert!(
        default_remote.is_none() || default_remote == Some("".to_string()),
        "Repo without remotes should have no default or empty default"
    );
}

// ============================================================================
// Commit Range Tests
// ============================================================================

#[test]
fn test_commit_range_length() {
    let test_repo = TestRepo::new();

    // Create commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    test_repo.stage_all_and_commit("Second").unwrap();

    file.set_contents(crate::lines![
        "line1".human(),
        "line2".human(),
        "line3".human()
    ]);
    let third = test_repo.stage_all_and_commit("Third").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Create commit range
    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        first.commit_sha.clone(),
        third.commit_sha.clone(),
        "HEAD".to_string(),
    )
    .unwrap();

    let length = range.length();
    assert_eq!(
        length, 2,
        "Range should contain 2 commits (second and third)"
    );
}

#[test]
fn test_commit_range_iteration() {
    let test_repo = TestRepo::new();

    // Create commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    let second = test_repo.stage_all_and_commit("Second").unwrap();

    file.set_contents(crate::lines![
        "line1".human(),
        "line2".human(),
        "line3".human()
    ]);
    let third = test_repo.stage_all_and_commit("Third").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        first.commit_sha,
        third.commit_sha.clone(),
        "HEAD".to_string(),
    )
    .unwrap();

    let commits: Vec<_> = range.into_iter().collect();
    assert_eq!(commits.len(), 2, "Should iterate over 2 commits");

    // Commits should be in reverse chronological order (newest first)
    assert_eq!(
        commits[0].id(),
        third.commit_sha,
        "First commit should be newest"
    );
    assert_eq!(
        commits[1].id(),
        second.commit_sha,
        "Second commit should be middle"
    );
}

#[test]
fn test_commit_range_all_commits() {
    let test_repo = TestRepo::new();

    // Create commits
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    test_repo.stage_all_and_commit("Second").unwrap();

    file.set_contents(crate::lines![
        "line1".human(),
        "line2".human(),
        "line3".human()
    ]);
    let third = test_repo.stage_all_and_commit("Third").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        first.commit_sha,
        third.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    let all_commits = range.all_commits();
    assert_eq!(all_commits.len(), 2, "Should have 2 commits");
}

// ============================================================================
// Merge Base Tests
// ============================================================================

#[test]
fn test_merge_base_linear_history() {
    let test_repo = TestRepo::new();

    // Create linear history
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    let second = test_repo.stage_all_and_commit("Second").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_base = repo.merge_base(first.commit_sha.clone(), second.commit_sha);
    assert!(merge_base.is_ok(), "Should find merge base");

    let base = merge_base.unwrap();
    assert_eq!(base, first.commit_sha, "Merge base should be first commit");
}

#[test]
fn test_merge_base_with_branches() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let base = test_repo.stage_all_and_commit("Base").unwrap();

    // Capture the original branch name before creating feature branch
    let original_branch = test_repo.current_branch();

    // Create branch
    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    file.set_contents(crate::lines!["line1".human(), "feature".human()]);
    let feature = test_repo.stage_all_and_commit("Feature").unwrap();

    // Go back to original branch and make different commit
    test_repo.git(&["checkout", &original_branch]).unwrap();
    file.set_contents(crate::lines!["line1".human(), "main".human()]);
    let main = test_repo.stage_all_and_commit("Main").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_base = repo.merge_base(feature.commit_sha, main.commit_sha);
    assert!(merge_base.is_ok(), "Should find merge base");

    let merge_base_sha = merge_base.unwrap();
    assert_eq!(
        merge_base_sha, base.commit_sha,
        "Merge base should be base commit"
    );
}

// ============================================================================
// File Content Tests
// ============================================================================

#[test]
fn test_get_file_content() {
    let test_repo = TestRepo::new();

    // Create file and commit
    let mut file = test_repo.filename("test.txt");
    let content = "test file content";
    file.set_contents(crate::lines![content.human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let file_content = repo.get_file_content("test.txt", &commit.commit_sha);
    assert!(file_content.is_ok(), "Should get file content");

    let content_bytes = file_content.unwrap();
    let content_str = String::from_utf8(content_bytes).unwrap();
    assert!(content_str.contains(content), "Content should match");
}

#[test]
fn test_get_file_content_nonexistent() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let result = repo.get_file_content("nonexistent.txt", &commit.commit_sha);
    assert!(result.is_err(), "Should error on nonexistent file");
}

#[test]
fn test_list_commit_files() {
    let test_repo = TestRepo::new();

    // Create multiple files and commit
    let mut file1 = test_repo.filename("file1.txt");
    let mut file2 = test_repo.filename("file2.txt");
    file1.set_contents(crate::lines!["content1".human()]);
    file2.set_contents(crate::lines!["content2".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let files = repo.list_commit_files(&commit.commit_sha, None);
    assert!(files.is_ok(), "Should list commit files");

    let files = files.unwrap();
    assert!(files.contains("file1.txt"), "Should contain file1.txt");
    assert!(files.contains("file2.txt"), "Should contain file2.txt");
}

#[test]
fn test_list_commit_files_with_pathspec() {
    let test_repo = TestRepo::new();

    // Create multiple files and commit
    let mut file1 = test_repo.filename("file1.txt");
    let mut file2 = test_repo.filename("file2.txt");
    file1.set_contents(crate::lines!["content1".human()]);
    file2.set_contents(crate::lines!["content2".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Filter to only file1.txt
    let mut pathspec = HashSet::new();
    pathspec.insert("file1.txt".to_string());

    let files = repo.list_commit_files(&commit.commit_sha, Some(&pathspec));
    assert!(files.is_ok(), "Should list filtered commit files");

    let files = files.unwrap();
    assert!(files.contains("file1.txt"), "Should contain file1.txt");
    assert!(!files.contains("file2.txt"), "Should not contain file2.txt");
}

#[test]
fn test_diff_changed_files() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    // Modify file
    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    let second = test_repo.stage_all_and_commit("Second").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let changed = repo.diff_changed_files(&first.commit_sha, &second.commit_sha);
    assert!(changed.is_ok(), "Should get changed files");

    let files = changed.unwrap();
    assert!(
        files.contains(&"test.txt".to_string()),
        "Should contain changed file"
    );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_find_commit_invalid_sha() {
    let test_repo = TestRepo::new();

    // Create a valid repo
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let result = repo.find_commit("0000000000000000000000000000000000000000".to_string());
    assert!(result.is_err(), "Should error on invalid commit SHA");
}

#[test]
fn test_find_blob_with_commit_sha() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Try to find blob using commit SHA (should fail)
    let result = repo.find_blob(commit.commit_sha);
    assert!(
        result.is_err(),
        "Should error when finding blob with commit SHA"
    );
}

#[test]
fn test_find_tree_with_commit_sha() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Try to find tree using commit SHA (should fail)
    let result = repo.find_tree(commit.commit_sha);
    assert!(
        result.is_err(),
        "Should error when finding tree with commit SHA"
    );
}

#[test]
fn test_revparse_invalid_ref() {
    let test_repo = TestRepo::new();

    // Create valid repo
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let result = repo.revparse_single("invalid-ref-name-12345");
    assert!(result.is_err(), "Should error on invalid ref");
}

// ============================================================================
// Bare Repository Tests
// ============================================================================

#[test]
fn test_is_bare_repository() {
    let bare_repo = TestRepo::new_bare();

    let repo = find_repository(&[
        "-C".to_string(),
        bare_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let is_bare = repo.is_bare_repository();
    assert!(is_bare.is_ok(), "Should check if bare");
    assert!(is_bare.unwrap(), "Should be bare repository");
}

#[test]
fn test_is_not_bare_repository() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let is_bare = repo.is_bare_repository();
    assert!(is_bare.is_ok(), "Should check if bare");
    assert!(!is_bare.unwrap(), "Should not be bare repository");
}

// ============================================================================
// Author and Signature Tests
// ============================================================================

#[test]
fn test_commit_author() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();
    let author = commit_obj.author();

    assert!(author.is_ok(), "Should get commit author");

    let author = author.unwrap();
    assert_eq!(author.name(), Some("Test User"), "Author name should match");
    assert_eq!(
        author.email(),
        Some("test@example.com"),
        "Author email should match"
    );
}

#[test]
fn test_commit_committer() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();
    let committer = commit_obj.committer();

    assert!(committer.is_ok(), "Should get commit committer");

    let committer = committer.unwrap();
    assert_eq!(
        committer.name(),
        Some("Test User"),
        "Committer name should match"
    );
}

#[test]
fn test_commit_time() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();
    let time = commit_obj.time();

    assert!(time.is_ok(), "Should get commit time");

    let time = time.unwrap();
    assert!(time.seconds() > 0, "Commit time should be after epoch");
}

#[test]
fn test_signature_when() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();
    let author = commit_obj.author().unwrap();
    let time = author.when();

    assert!(time.seconds() > 0, "Author time should be after epoch");
}

// ============================================================================
// Working Directory Operations Tests
// ============================================================================

#[test]
fn test_find_repository_in_path() {
    let test_repo = TestRepo::new();

    // Create a commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let result = find_repository_in_path(test_repo.path().to_str().unwrap());
    assert!(result.is_ok(), "Should find repository in path");
}

#[test]
fn test_global_args_for_exec() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let args = repo.global_args_for_exec();

    // Should include --no-pager
    assert!(
        args.contains(&"--no-pager".to_string()),
        "Global args should include --no-pager"
    );
}

#[test]
fn test_git_command_execution() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Execute git command
    let result = repo.git(&["rev-parse", "HEAD"]);
    assert!(result.is_ok(), "Should execute git command");

    let output = result.unwrap();
    assert!(!output.is_empty(), "Output should not be empty");
}

// ============================================================================
// References Iterator Tests
// ============================================================================

#[test]
fn test_references_iterator() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let refs = repo.references();
    assert!(refs.is_ok(), "Should get references iterator");

    let refs = refs.unwrap();
    let ref_list: Vec<_> = refs.collect();

    assert!(!ref_list.is_empty(), "Should have at least one reference");
}

#[test]
fn test_resolve_author_spec() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Resolve author by name
    let result = repo.resolve_author_spec("Test User");
    assert!(result.is_ok(), "Should resolve author spec");

    let author = result.unwrap();
    assert!(author.is_some(), "Should find author");
}

#[test]
fn test_resolve_author_spec_not_found() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // Resolve nonexistent author
    let result = repo.resolve_author_spec("Nonexistent Author");
    assert!(result.is_ok(), "Should not error on nonexistent author");

    let author = result.unwrap();
    assert!(author.is_none(), "Should not find nonexistent author");
}

// ============================================================================
// Edge Cases and Special Scenarios
// ============================================================================

#[test]
fn test_empty_repository() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    // HEAD should exist even in empty repo
    let head = repo.head();
    assert!(head.is_ok(), "Should get HEAD in empty repository");
}

#[test]
fn test_initial_commit_has_no_parent() {
    let test_repo = TestRepo::new();

    // Create initial commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();

    // Should have no parents
    let parent_result = commit_obj.parent(0);
    assert!(
        parent_result.is_err(),
        "Initial commit should have no parent"
    );
}

#[test]
fn test_tree_clone() {
    let test_repo = TestRepo::new();

    // Create commit
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Test").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit_obj = repo.find_commit(commit.commit_sha).unwrap();
    let tree = commit_obj.tree().unwrap();
    let tree_clone = tree.clone();

    assert_eq!(
        tree.id(),
        tree_clone.id(),
        "Cloned tree should have same ID"
    );
}

#[test]
fn test_commit_with_unicode_message() {
    let test_repo = TestRepo::new();

    // Create commit with unicode message
    let mut file = test_repo.filename("test.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.git(&["add", "-A"]).unwrap();
    test_repo
        .git(&["commit", "-m", "Unicode message: 你好世界 🎉"])
        .unwrap();

    let commit_sha = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_sha).unwrap();
    let summary = commit.summary().unwrap();

    assert!(
        summary.contains("你好世界"),
        "Summary should contain unicode characters"
    );
}

#[test]
fn test_multiple_files_in_single_commit() {
    let test_repo = TestRepo::new();

    // Create multiple files
    let mut file1 = test_repo.filename("file1.txt");
    let mut file2 = test_repo.filename("file2.txt");
    let mut file3 = test_repo.filename("file3.txt");

    file1.set_contents(crate::lines!["content1".human()]);
    file2.set_contents(crate::lines!["content2".human()]);
    file3.set_contents(crate::lines!["content3".human()]);

    let commit = test_repo.stage_all_and_commit("Multiple files").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let files = repo.list_commit_files(&commit.commit_sha, None).unwrap();

    assert_eq!(files.len(), 3, "Should have 3 files in commit");
    assert!(files.contains("file1.txt"), "Should contain file1.txt");
    assert!(files.contains("file2.txt"), "Should contain file2.txt");
    assert!(files.contains("file3.txt"), "Should contain file3.txt");
}

// ============================================================================
// Commit Metadata Tests
// ============================================================================

#[test]
fn test_commit_summary_for_single_line_message() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("single_summary.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo
        .stage_all_and_commit("single line subject")
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit.commit_sha).unwrap();
    assert_eq!(commit.summary().unwrap(), "single line subject");
}

#[test]
fn test_commit_summary_and_body_for_multiline_message() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("msg.txt");
    file.set_contents(crate::lines!["content".human()]);
    file.stage();

    test_repo
        .git(&[
            "commit",
            "-m",
            "Title line",
            "-m",
            "Body line 1\n\nBody line 2",
        ])
        .unwrap();

    let oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid).unwrap();

    assert_eq!(commit.summary().unwrap(), "Title line");

    let body = commit.body().unwrap();
    assert!(body.contains("Body line 1"));
    assert!(body.contains("Body line 2"));
    assert!(!body.contains("Title line"));
}

#[test]
fn test_commit_body_is_empty_when_commit_has_no_body() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("no_body.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo
        .stage_all_and_commit("single line subject")
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid.commit_sha).unwrap();
    assert_eq!(commit.summary().unwrap(), "single line subject");
    assert_eq!(commit.body().unwrap(), "");
}

#[test]
fn test_commit_author_and_committer_match_default_identity() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("default_identity.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid.commit_sha).unwrap();
    let author = commit.author().unwrap();
    let committer = commit.committer().unwrap();

    assert!(author.name().is_some());
    assert!(author.email().is_some());
    assert!(committer.name().is_some());
    assert!(committer.email().is_some());
    assert_eq!(author.name(), committer.name());
    assert_eq!(author.email(), committer.email());
}

#[test]
fn test_commit_author_and_committer_can_differ() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("author.txt");
    file.set_contents(crate::lines!["content".human()]);
    file.stage();

    test_repo
        .git_with_env(
            &["commit", "-m", "author vs committer"],
            &[
                ("GIT_AUTHOR_NAME", "Author Name"),
                ("GIT_AUTHOR_EMAIL", "author@example.com"),
                ("GIT_COMMITTER_NAME", "Committer Name"),
                ("GIT_COMMITTER_EMAIL", "committer@example.com"),
            ],
            None,
        )
        .unwrap();

    let oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid).unwrap();
    let author = commit.author().unwrap();
    let committer = commit.committer().unwrap();

    assert_eq!(author.name().unwrap(), "Author Name");
    assert_eq!(author.email().unwrap(), "author@example.com");
    assert_eq!(committer.name().unwrap(), "Committer Name");
    assert_eq!(committer.email().unwrap(), "committer@example.com");
}

#[test]
fn test_commit_time_uses_committer_time() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("time.txt");
    file.set_contents(crate::lines!["content".human()]);
    file.stage();

    test_repo
        .git_with_env(
            &["commit", "-m", "time test"],
            &[
                ("GIT_AUTHOR_NAME", "Author"),
                ("GIT_AUTHOR_EMAIL", "author@example.com"),
                ("GIT_AUTHOR_DATE", "1700000000 +0000"),
                ("GIT_COMMITTER_NAME", "Committer"),
                ("GIT_COMMITTER_EMAIL", "committer@example.com"),
                ("GIT_COMMITTER_DATE", "1800000000 +0000"),
            ],
            None,
        )
        .unwrap();

    let oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid).unwrap();
    let time = commit.time().unwrap();
    assert_eq!(time.seconds(), 1_800_000_000);
}

#[test]
fn test_root_commit_has_no_parents() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("root.txt");
    file.set_contents(crate::lines!["root".human()]);
    let commit_info = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit_info.commit_sha).unwrap();

    assert_eq!(commit.parent_count().unwrap(), 0);
    assert_eq!(commit.parents().count(), 0);
    assert!(commit.parent(0).is_err());
}

#[test]
fn test_commit_parent_zero_returns_first_parent() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("parent_zero.txt");
    file.set_contents(crate::lines!["one".human()]);
    let first = test_repo.stage_all_and_commit("first").unwrap();

    file.set_contents(crate::lines!["one".human(), "two".human()]);
    let second = test_repo.stage_all_and_commit("second").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(second.commit_sha).unwrap();
    let parent = commit.parent(0).unwrap();
    assert_eq!(parent.id(), first.commit_sha);
}

#[test]
fn test_commit_parent_out_of_bounds_errors() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("parent_oob.txt");
    file.set_contents(crate::lines!["one".human()]);
    let commit = test_repo.stage_all_and_commit("first").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit.commit_sha).unwrap();
    assert!(commit.parent(1).is_err());
}

#[test]
fn test_merge_commit_parent_count_and_order_are_stable() {
    let test_repo = TestRepo::new();

    let mut base_file = test_repo.filename("base.txt");
    base_file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature_tip = test_repo.stage_all_and_commit("feature commit").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main commit").unwrap();

    test_repo
        .git(&["merge", "--no-ff", "feature", "-m", "merge feature"])
        .unwrap();

    let merge_oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(merge_oid).unwrap();
    assert_eq!(commit.parent_count().unwrap(), 2);

    let parents: Vec<String> = commit.parents().map(|p| p.id()).collect();
    assert_eq!(parents, vec![main_tip.commit_sha, feature_tip.commit_sha]);
}

#[test]
fn test_commit_tree_matches_head_tree_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("tree.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("tree commit").unwrap();

    let expected_tree_oid = test_repo
        .git(&["rev-parse", "HEAD^{tree}"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(commit.commit_sha).unwrap();
    let tree = commit.tree().unwrap();

    assert_eq!(tree.id(), expected_tree_oid);
}

#[test]
fn test_commit_metadata_supports_non_ascii_message_and_author() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("unicode_meta.txt");
    file.set_contents(crate::lines!["内容".human()]);
    file.stage();

    test_repo
        .git_with_env(
            &["commit", "-m", "你好世界", "-m", "正文内容"],
            &[
                ("GIT_AUTHOR_NAME", "测试作者"),
                ("GIT_AUTHOR_EMAIL", "author@example.com"),
                ("GIT_COMMITTER_NAME", "测试作者"),
                ("GIT_COMMITTER_EMAIL", "author@example.com"),
            ],
            None,
        )
        .unwrap();

    let oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid).unwrap();
    assert_eq!(commit.summary().unwrap(), "你好世界");
    assert!(commit.body().unwrap().contains("正文内容"));
    assert_eq!(commit.author().unwrap().name().unwrap(), "测试作者");
}

// ============================================================================
// Revparse and Reference Resolution Tests
// ============================================================================

#[test]
fn test_revparse_single_resolves_head() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("head.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let obj = repo.revparse_single("HEAD").unwrap();
    assert_eq!(obj.id(), oid.commit_sha);
}

#[test]
fn test_revparse_single_resolves_full_commit_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("full_oid.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let obj = repo.revparse_single(&oid.commit_sha).unwrap();
    assert_eq!(obj.id(), oid.commit_sha);
}

#[test]
fn test_revparse_single_resolves_branch_name() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("branch_resolve.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let branch_name = test_repo
        .git(&["branch", "--show-current"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let obj = repo.revparse_single(&branch_name).unwrap();
    assert_eq!(obj.id(), oid.commit_sha);
}

#[test]
fn test_revparse_single_resolves_fully_qualified_refname() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("fq_ref.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head_ref = repo.head().unwrap().name().unwrap().to_string();
    let obj = repo.revparse_single(&head_ref).unwrap();
    assert_eq!(obj.id(), oid.commit_sha);
}

#[test]
fn test_revparse_single_errors_for_invalid_spec() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert!(repo.revparse_single("definitely-not-a-real-ref").is_err());
}

#[test]
fn test_object_peel_to_commit_from_commit_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("peel_commit.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let obj = repo.revparse_single(&oid.commit_sha).unwrap();
    let commit = obj.peel_to_commit().unwrap();
    assert_eq!(commit.id(), oid.commit_sha);
}

#[test]
fn test_reference_peel_to_commit_from_annotated_tag() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("tag.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit_info = test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo
        .git(&["tag", "-a", "v1", "-m", "annotated tag"])
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/tags/v1").unwrap();
    let peeled = reference.peel_to_commit().unwrap();

    assert_eq!(peeled.id(), commit_info.commit_sha);
}

#[test]
fn test_reference_peel_to_commit_from_lightweight_tag() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("lw_tag.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["tag", "v1"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/tags/v1").unwrap();
    let peeled = reference.peel_to_commit().unwrap();

    assert_eq!(peeled.id(), commit.commit_sha);
}

#[test]
fn test_reference_peel_to_blob_from_blob_spec() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("blob_spec.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo
        .git(&["tag", "blob-tag", "HEAD:blob_spec.txt"])
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/tags/blob-tag").unwrap();
    let blob = reference.peel_to_blob().unwrap();

    let expected_blob_oid = test_repo
        .git(&["rev-parse", "HEAD:blob_spec.txt"])
        .unwrap()
        .trim()
        .to_string();

    assert_eq!(blob.id(), expected_blob_oid);
}

#[test]
fn test_reference_peel_to_commit_errors_for_non_commitish_reference() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("non_commitish.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo
        .git(&["tag", "blob-tag", "HEAD:non_commitish.txt"])
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/tags/blob-tag").unwrap();
    assert!(reference.peel_to_commit().is_err());
}

#[test]
fn test_reference_shorthand_matches_expected_branch_name() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("shorthand.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/heads/feature").unwrap();
    assert_eq!(reference.shorthand().unwrap(), "feature");
}

#[test]
fn test_reference_target_returns_expected_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("target.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head_name = repo.head().unwrap().name().unwrap().to_string();
    let reference = repo.find_reference(&head_name).unwrap();

    assert_eq!(reference.target().unwrap(), commit.commit_sha);
}

#[test]
fn test_head_returns_symbolic_branch_ref_when_attached() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("attached.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    let name = head.name().unwrap();
    assert!(name.starts_with("refs/heads/"));
}

#[test]
fn test_head_returns_head_when_detached() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("detached.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["checkout", &oid.commit_sha]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    assert_eq!(head.name().unwrap(), "HEAD");
}

#[test]
fn test_find_reference_finds_existing_branch() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("find_branch.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/heads/feature");
    assert!(reference.is_ok());
    assert_eq!(reference.unwrap().name().unwrap(), "refs/heads/feature");
}

#[test]
fn test_find_reference_finds_existing_tag() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("find_tag.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();
    test_repo.git(&["tag", "v1"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let reference = repo.find_reference("refs/tags/v1");
    assert!(reference.is_ok());
    assert_eq!(reference.unwrap().name().unwrap(), "refs/tags/v1");
}

#[test]
fn test_find_reference_errors_for_missing_ref() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert!(repo.find_reference("refs/heads/does-not-exist").is_err());
}

#[test]
fn test_references_lists_heads_and_tags() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("refs_list.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    test_repo.git(&["tag", "v1"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let refs: Vec<String> = repo
        .references()
        .unwrap()
        .map(|r| r.unwrap().name().unwrap().to_string())
        .collect();

    assert!(refs.iter().any(|r| r.starts_with("refs/heads/")));
    assert!(refs.iter().any(|r| r == "refs/heads/feature"));
    assert!(refs.iter().any(|r| r == "refs/tags/v1"));
}

#[test]
fn test_references_include_fully_qualified_refnames() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("refs_full.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();
    test_repo.git(&["tag", "v1"]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let refs: Vec<String> = repo
        .references()
        .unwrap()
        .map(|r| r.unwrap().name().unwrap().to_string())
        .collect();

    assert!(refs.iter().all(|r| r.starts_with("refs/")));
    assert!(refs.iter().any(|r| r == "refs/tags/v1"));
    assert!(refs.iter().any(|r| r.starts_with("refs/heads/")));
}

// ============================================================================
// Commit Graph and Range Tests
// ============================================================================

#[test]
fn test_merge_base_returns_common_ancestor_for_diverged_branches() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("graph.txt");
    file.set_contents(crate::lines!["base".human()]);
    let base = test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature_tip = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_base = repo
        .merge_base(main_tip.commit_sha, feature_tip.commit_sha)
        .unwrap();
    assert_eq!(merge_base, base.commit_sha);
}

#[test]
fn test_merge_base_errors_when_commits_are_invalid() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert!(
        repo.merge_base(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        )
        .is_err()
    );
}

#[test]
fn test_commit_range_length_for_linear_history() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("range_len.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let first = test_repo.stage_all_and_commit("First").unwrap();

    file.set_contents(crate::lines!["line1".human(), "line2".human()]);
    test_repo.stage_all_and_commit("Second").unwrap();

    file.set_contents(crate::lines![
        "line1".human(),
        "line2".human(),
        "line3".human()
    ]);
    let third = test_repo.stage_all_and_commit("Third").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        first.commit_sha,
        third.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert_eq!(range.length(), 2);
}

#[test]
fn test_commit_range_length_is_zero_for_adjacent_empty_range() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("range_zero.txt");
    file.set_contents(crate::lines!["line1".human()]);
    let commit = test_repo.stage_all_and_commit("only").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        commit.commit_sha.clone(),
        commit.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert_eq!(range.length(), 0);
}

#[test]
fn test_commit_range_into_iter_returns_expected_commits_in_current_order() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("range.txt");
    file.set_contents(crate::lines!["a".human()]);
    let a = test_repo.stage_all_and_commit("a").unwrap();

    file.set_contents(crate::lines!["b".human()]);
    let b = test_repo.stage_all_and_commit("b").unwrap();

    file.set_contents(crate::lines!["c".human()]);
    let c = test_repo.stage_all_and_commit("c").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        a.commit_sha,
        c.commit_sha.clone(),
        "HEAD".to_string(),
    )
    .unwrap();

    let commits: Vec<String> = range.into_iter().map(|commit| commit.id()).collect();

    assert_eq!(commits, vec![c.commit_sha, b.commit_sha]);
}

#[test]
fn test_commit_range_into_iter_handles_single_commit_range() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("single_range.txt");
    file.set_contents(crate::lines!["only".human()]);
    let oid = test_repo.stage_all_and_commit("only").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        oid.commit_sha.clone(),
        oid.commit_sha.clone(),
        "HEAD".to_string(),
    )
    .unwrap();

    let commits: Vec<String> = range.into_iter().map(|c| c.id()).collect();
    assert_eq!(commits, vec![oid.commit_sha]);
}

#[test]
fn test_commit_range_into_iter_returns_empty_for_empty_range() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::empty(&repo);
    let commits: Vec<String> = range.into_iter().map(|c| c.id()).collect();

    assert!(commits.is_empty());
}

#[test]
fn test_commit_range_is_valid_when_start_is_ancestor_of_end() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("valid_range.txt");
    file.set_contents(crate::lines!["a".human()]);
    let a = test_repo.stage_all_and_commit("a").unwrap();

    file.set_contents(crate::lines!["b".human()]);
    test_repo.stage_all_and_commit("b").unwrap();

    file.set_contents(crate::lines!["c".human()]);
    let c = test_repo.stage_all_and_commit("c").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        a.commit_sha,
        c.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert!(range.is_valid().is_ok());
}

#[test]
fn test_commit_range_is_invalid_when_start_is_not_ancestor_of_end() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("invalid_ancestor.txt");
    file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main = test_repo.stage_all_and_commit("main").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        feature.commit_sha,
        main.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert!(range.is_valid().is_err());
}

#[test]
fn test_commit_range_is_invalid_when_start_is_not_reachable_from_refname() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("unreachable_start.txt");
    file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main = test_repo.stage_all_and_commit("main").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        feature.commit_sha,
        main.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert!(range.is_valid().is_err());
}

#[test]
fn test_commit_range_is_invalid_when_end_is_not_reachable_from_refname() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("unreachable_end.txt");
    file.set_contents(crate::lines!["base".human()]);
    let base = test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        base.commit_sha,
        feature.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert!(range.is_valid().is_err());
}

#[test]
fn test_commit_range_allows_empty_tree_hash_as_start() {
    const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("empty_tree.txt");
    file.set_contents(crate::lines!["content".human()]);
    let end_oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        EMPTY_TREE_HASH.to_string(),
        end_oid.commit_sha,
        "HEAD".to_string(),
    )
    .unwrap();

    assert!(range.is_valid().is_ok());
}

#[test]
fn test_parent_on_refname_selects_parent_reachable_from_target_branch() {
    let test_repo = TestRepo::new();

    let mut base_file = test_repo.filename("base.txt");
    base_file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main").unwrap();

    test_repo
        .git(&["merge", "--no-ff", "feature", "-m", "merge"])
        .unwrap();
    let merge_oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_commit = repo.find_commit(merge_oid).unwrap();

    let selected = merge_commit
        .parent_on_refname("main")
        .or_else(|_| merge_commit.parent_on_refname("master"))
        .unwrap();

    assert_eq!(selected.id(), main_tip.commit_sha);
}

#[test]
fn test_parent_on_refname_accepts_short_branch_name() {
    let test_repo = TestRepo::new();

    let mut base_file = test_repo.filename("base.txt");
    base_file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main").unwrap();

    test_repo
        .git(&["merge", "--no-ff", "feature", "-m", "merge"])
        .unwrap();
    let merge_oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_commit = repo.find_commit(merge_oid).unwrap();
    let selected = merge_commit
        .parent_on_refname("main")
        .or_else(|_| merge_commit.parent_on_refname("master"))
        .unwrap();

    assert_eq!(selected.id(), main_tip.commit_sha);
}

#[test]
fn test_parent_on_refname_accepts_fully_qualified_refname() {
    let test_repo = TestRepo::new();

    let mut base_file = test_repo.filename("base.txt");
    base_file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    test_repo.stage_all_and_commit("feature").unwrap();

    let main_ref = if test_repo.git(&["checkout", "main"]).is_ok() {
        "refs/heads/main"
    } else {
        test_repo.git(&["checkout", "master"]).unwrap();
        "refs/heads/master"
    };

    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main").unwrap();

    test_repo
        .git(&["merge", "--no-ff", "feature", "-m", "merge"])
        .unwrap();
    let merge_oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let merge_commit = repo.find_commit(merge_oid).unwrap();
    let selected = merge_commit.parent_on_refname(main_ref).unwrap();

    assert_eq!(selected.id(), main_tip.commit_sha);
}

#[test]
fn test_parent_on_refname_errors_when_no_parent_is_reachable_from_ref() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("no_parent_ref.txt");
    file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    file.set_contents(crate::lines!["feature".human()]);
    let head = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo
        .git(&["checkout", "--orphan", "other-root"])
        .unwrap();
    fs::write(test_repo.path().join("other.txt"), "other root\n").unwrap();
    test_repo.git(&["add", "other.txt"]).unwrap();
    test_repo.git(&["commit", "-m", "other root"]).unwrap();

    test_repo.git(&["checkout", &head.commit_sha]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head.commit_sha).unwrap();
    assert!(commit.parent_on_refname("other-root").is_err());
}

// ============================================================================
// Object Access and Tree/Blob Content Tests
// ============================================================================

#[test]
fn test_object_type_reports_commit_blob_and_tree() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("types.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let blob_oid = test_repo
        .git(&["rev-parse", "HEAD:types.txt"])
        .unwrap()
        .trim()
        .to_string();

    let tree_oid = test_repo
        .git(&["rev-parse", "HEAD^{tree}"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert_eq!(repo.find_commit(commit.commit_sha).unwrap().id().len(), 40);
    assert_eq!(repo.find_blob(blob_oid).unwrap().id().len(), 40);
    assert_eq!(repo.find_tree(tree_oid).unwrap().id().len(), 40);
}

#[test]
fn test_object_type_errors_for_missing_oid() {
    let test_repo = TestRepo::new();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert!(
        repo.find_commit("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
            .is_err()
    );
    assert!(
        repo.find_blob("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
            .is_err()
    );
    assert!(
        repo.find_tree("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
            .is_err()
    );
}

#[test]
fn test_find_commit_returns_commit_for_commit_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("find_commit.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let found = repo.find_commit(commit.commit_sha.clone()).unwrap();
    assert_eq!(found.id(), commit.commit_sha);
}

#[test]
fn test_find_blob_returns_blob_for_blob_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("find_blob.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let blob_oid = test_repo
        .git(&["rev-parse", "HEAD:find_blob.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let found = repo.find_blob(blob_oid.clone()).unwrap();
    assert_eq!(found.id(), blob_oid);
}

#[test]
fn test_find_tree_returns_tree_for_tree_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("find_tree.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let tree_oid = test_repo
        .git(&["rev-parse", "HEAD^{tree}"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let found = repo.find_tree(tree_oid.clone()).unwrap();
    assert_eq!(found.id(), tree_oid);
}

#[test]
fn test_find_commit_errors_for_non_commit_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("blob_only.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let blob_oid = test_repo
        .git(&["rev-parse", "HEAD:blob_only.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let err = match repo.find_commit(blob_oid) {
        Ok(_) => panic!("expected non-commit lookup to fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("Object is not a commit"));
}

#[test]
fn test_find_blob_errors_for_non_blob_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("not_blob.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let err = match repo.find_blob(commit.commit_sha) {
        Ok(_) => panic!("expected non-blob lookup to fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("Object is not a blob"));
}

#[test]
fn test_find_tree_errors_for_non_tree_oid() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("not_tree.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let err = match repo.find_tree(commit.commit_sha) {
        Ok(_) => panic!("expected non-tree lookup to fail"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("Object is not a tree"));
}

#[test]
fn test_blob_content_returns_exact_text_bytes() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("blob.txt");
    file.set_contents(crate::lines!["hello".human(), "world".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let blob_oid = test_repo
        .git(&["rev-parse", "HEAD:blob.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let blob = repo.find_blob(blob_oid).unwrap();
    assert_eq!(blob.content().unwrap(), b"hello\nworld");
}

#[test]
fn test_blob_content_returns_exact_binary_bytes() {
    let test_repo = TestRepo::new();

    let bytes = vec![0, 1, 2, 3, 255, 10, 0, 42];
    fs::write(test_repo.path().join("bin.dat"), &bytes).unwrap();

    test_repo.git(&["add", "bin.dat"]).unwrap();
    test_repo.git(&["commit", "-m", "add binary"]).unwrap();

    let blob_oid = test_repo
        .git(&["rev-parse", "HEAD:bin.dat"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let blob = repo.find_blob(blob_oid).unwrap();
    assert_eq!(blob.content().unwrap(), bytes);
}

#[test]
fn test_get_file_content_reads_file_from_commit_root() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("root_file.txt");
    file.set_contents(crate::lines!["root content".human()]);
    test_repo.stage_all_and_commit("add root file").unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let content = repo.get_file_content("root_file.txt", &head).unwrap();
    assert_eq!(content, b"root content");
}

#[test]
fn test_get_file_content_reads_file_from_nested_path() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("nested/dir")).unwrap();
    fs::write(
        test_repo.path().join("nested/dir/file.txt"),
        "nested content\n",
    )
    .unwrap();

    test_repo.git(&["add", "nested/dir/file.txt"]).unwrap();
    test_repo.git(&["commit", "-m", "add nested file"]).unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let content = repo.get_file_content("nested/dir/file.txt", &head).unwrap();
    assert_eq!(content, b"nested content\n");
}

#[test]
fn test_get_file_content_errors_for_missing_path() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("exists.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("Initial commit").unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    assert!(repo.get_file_content("missing.txt", &head).is_err());
}

#[test]
fn test_get_file_content_errors_when_path_is_directory_like() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("dir/sub")).unwrap();
    fs::write(test_repo.path().join("dir/sub/file.txt"), "content\n").unwrap();

    test_repo.git(&["add", "dir/sub/file.txt"]).unwrap();
    test_repo.git(&["commit", "-m", "add nested file"]).unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let content = repo.get_file_content("dir", &head).unwrap();
    assert!(content.starts_with(b"tree "));
    assert!(content.ends_with(b"sub/\n"));
}

#[test]
fn test_tree_get_path_returns_expected_entry_for_root_file() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("root_entry.txt");
    file.set_contents(crate::lines!["content".human()]);
    test_repo.stage_all_and_commit("add root entry").unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let expected_blob_oid = test_repo
        .git(&["rev-parse", "HEAD:root_entry.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("root_entry.txt")).unwrap();

    assert_eq!(entry.id(), expected_blob_oid);
}

#[test]
fn test_tree_get_path_returns_expected_entry_for_nested_file() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("a/b")).unwrap();
    fs::write(test_repo.path().join("a/b/file.txt"), "x\n").unwrap();

    test_repo.git(&["add", "a/b/file.txt"]).unwrap();
    test_repo.git(&["commit", "-m", "add nested file"]).unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let expected_blob_oid = test_repo
        .git(&["rev-parse", "HEAD:a/b/file.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("a/b/file.txt")).unwrap();

    assert_eq!(entry.id(), expected_blob_oid);
}

#[test]
fn test_tree_get_path_errors_for_missing_path() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("exists.txt");
    file.set_contents(crate::lines!["content".human()]);
    let head = test_repo.stage_all_and_commit("Initial commit").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head.commit_sha).unwrap();
    let tree = commit.tree().unwrap();

    assert!(tree.get_path(Path::new("missing.txt")).is_err());
}

#[test]
fn test_get_file_content_supports_non_ascii_paths() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("目录")).unwrap();
    let rel_path = "目录/你好 世界.txt";
    fs::write(test_repo.path().join(rel_path), "hello unicode\n").unwrap();

    test_repo.git(&["add", rel_path]).unwrap();
    test_repo
        .git(&["commit", "-m", "add unicode path"])
        .unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let content = repo.get_file_content(rel_path, &head).unwrap();
    assert_eq!(content, b"hello unicode\n");
}

#[test]
fn test_tree_get_path_supports_paths_with_spaces() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("dir with spaces")).unwrap();
    let rel_path = "dir with spaces/file name.txt";
    fs::write(test_repo.path().join(rel_path), "space path\n").unwrap();

    test_repo.git(&["add", rel_path]).unwrap();
    test_repo.git(&["commit", "-m", "add spaced path"]).unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let expected_blob_oid = test_repo
        .git(&["rev-parse", &format!("HEAD:{}", rel_path)])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new(rel_path)).unwrap();

    assert_eq!(entry.id(), expected_blob_oid);
}

// ============================================================================
// Migration Guard Tests
// ============================================================================

#[test]
fn test_detached_head_behavior_matches_current_repository_contract() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("guard_detached.txt");
    file.set_contents(crate::lines!["content".human()]);
    let oid = test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo.git(&["checkout", &oid.commit_sha]).unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let head = repo.head().unwrap();
    assert_eq!(head.name().unwrap(), "HEAD");
}

#[test]
fn test_commit_range_iteration_order_matches_current_repository_contract() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("guard_range.txt");
    file.set_contents(crate::lines!["a".human()]);
    let a = test_repo.stage_all_and_commit("a").unwrap();

    file.set_contents(crate::lines!["b".human()]);
    let b = test_repo.stage_all_and_commit("b").unwrap();

    file.set_contents(crate::lines!["c".human()]);
    let c = test_repo.stage_all_and_commit("c").unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let range = git_ai::git::repository::CommitRange::new(
        &repo,
        a.commit_sha,
        c.commit_sha.clone(),
        "HEAD".to_string(),
    )
    .unwrap();
    let commits: Vec<String> = range.into_iter().map(|commit| commit.id()).collect();

    assert_eq!(commits, vec![c.commit_sha, b.commit_sha]);
}

#[test]
fn test_merge_commit_parent_order_matches_current_repository_contract() {
    let test_repo = TestRepo::new();

    let mut base_file = test_repo.filename("base.txt");
    base_file.set_contents(crate::lines!["base".human()]);
    test_repo.stage_all_and_commit("base").unwrap();

    test_repo.git(&["checkout", "-b", "feature"]).unwrap();
    let mut feature_file = test_repo.filename("feature.txt");
    feature_file.set_contents(crate::lines!["feature".human()]);
    let feature_tip = test_repo.stage_all_and_commit("feature").unwrap();

    test_repo.git(&["checkout", "main"]).unwrap_or_else(|_| {
        test_repo.git(&["checkout", "master"]).unwrap();
        String::new()
    });
    let mut main_file = test_repo.filename("main.txt");
    main_file.set_contents(crate::lines!["main".human()]);
    let main_tip = test_repo.stage_all_and_commit("main").unwrap();

    test_repo
        .git(&["merge", "--no-ff", "feature", "-m", "merge"])
        .unwrap();
    let merge_oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(merge_oid).unwrap();
    let parents: Vec<String> = commit.parents().map(|p| p.id()).collect();

    assert_eq!(parents, vec![main_tip.commit_sha, feature_tip.commit_sha]);
}

#[test]
fn test_annotated_tag_peeling_matches_current_repository_contract() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("guard_tag.txt");
    file.set_contents(crate::lines!["content".human()]);
    let commit = test_repo.stage_all_and_commit("Initial commit").unwrap();

    test_repo
        .git(&["tag", "-a", "v1", "-m", "annotated"])
        .unwrap();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let tag_ref = repo.find_reference("refs/tags/v1").unwrap();
    let peeled = tag_ref.peel_to_commit().unwrap();

    assert_eq!(peeled.id(), commit.commit_sha);
}

#[test]
fn test_summary_and_body_parsing_matches_current_repository_contract() {
    let test_repo = TestRepo::new();

    let mut file = test_repo.filename("guard_msg.txt");
    file.set_contents(crate::lines!["content".human()]);
    file.stage();

    test_repo
        .git(&[
            "commit",
            "-m",
            "Subject",
            "-m",
            "Body line 1\n\nBody line 2",
        ])
        .unwrap();

    let oid = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(oid).unwrap();
    assert_eq!(commit.summary().unwrap(), "Subject");

    let body = commit.body().unwrap();
    assert!(body.contains("Body line 1"));
    assert!(body.contains("Body line 2"));
    assert!(!body.contains("Subject"));
}

#[test]
fn test_tree_path_lookup_matches_current_repository_contract_for_nested_paths() {
    let test_repo = TestRepo::new();

    fs::create_dir_all(test_repo.path().join("nested/deep")).unwrap();
    fs::write(
        test_repo.path().join("nested/deep/file.txt"),
        "nested contract\n",
    )
    .unwrap();

    test_repo.git(&["add", "nested/deep/file.txt"]).unwrap();
    test_repo.git(&["commit", "-m", "add nested path"]).unwrap();

    let head = test_repo
        .git(&["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string();
    let expected_blob_oid = test_repo
        .git(&["rev-parse", "HEAD:nested/deep/file.txt"])
        .unwrap()
        .trim()
        .to_string();

    let repo = find_repository(&[
        "-C".to_string(),
        test_repo.path().to_str().unwrap().to_string(),
    ])
    .unwrap();

    let commit = repo.find_commit(head).unwrap();
    let tree = commit.tree().unwrap();
    let entry = tree.get_path(Path::new("nested/deep/file.txt")).unwrap();

    assert_eq!(entry.id(), expected_blob_oid);
}

crate::reuse_tests_in_worktree!(
    test_find_repository_in_valid_repo,
    test_find_repository_in_subdirectory,
    test_find_repository_in_nested_subdirectory,
    test_find_repository_for_bare_repo,
    test_repository_path_methods,
    test_canonical_workdir,
    test_path_is_in_workdir,
    test_head_on_main_branch,
    test_head_on_feature_branch,
    test_head_target,
    test_reference_is_branch,
    test_find_reference,
    test_find_commit,
    test_commit_summary,
    test_commit_body,
    test_commit_parent,
    test_commit_parents_iterator,
    test_commit_parent_count,
    test_commit_tree,
    test_revparse_single,
    test_revparse_single_with_relative_ref,
    test_object_peel_to_commit,
    test_tree_get_path,
    test_tree_get_path_nested,
    test_tree_get_path_nonexistent,
    test_find_blob,
    test_blob_content,
    test_config_get_str,
    test_config_get_str_nonexistent,
    test_config_get_regexp,
    test_git_version,
    test_git_supports_ignore_revs_file,
    test_remotes_empty,
    test_remotes_with_origin,
    test_remotes_with_urls,
    test_get_default_remote,
    test_get_default_remote_no_remotes,
    test_commit_range_length,
    test_commit_range_iteration,
    test_commit_range_all_commits,
    test_merge_base_linear_history,
    test_merge_base_with_branches,
    test_get_file_content,
    test_get_file_content_nonexistent,
    test_list_commit_files,
    test_list_commit_files_with_pathspec,
    test_diff_changed_files,
    test_find_commit_invalid_sha,
    test_find_blob_with_commit_sha,
    test_find_tree_with_commit_sha,
    test_revparse_invalid_ref,
    test_is_bare_repository,
    test_is_not_bare_repository,
    test_commit_author,
    test_commit_committer,
    test_commit_time,
    test_signature_when,
    test_find_repository_in_path,
    test_global_args_for_exec,
    test_git_command_execution,
    test_references_iterator,
    test_resolve_author_spec,
    test_resolve_author_spec_not_found,
    test_empty_repository,
    test_initial_commit_has_no_parent,
    test_tree_clone,
    test_commit_with_unicode_message,
    test_multiple_files_in_single_commit,
);
