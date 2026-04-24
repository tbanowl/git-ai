//! Comprehensive tests for non-`repository.rs` git2 migration targets.

use crate::repos::test_repo::TestRepo;
use crate::test_utils::fixture_path;
use git_ai::authorship::rebase_authorship::walk_commits_to_base;
use git_ai::commands::search::search_by_commit_range;
use git_ai::git::refs::{copy_ref, ref_exists};
use git_ai::git::repository::{Repository, find_repository_in_path};
use serde_json::json;
use std::fs;

fn open_repo(repo: &TestRepo) -> Repository {
    find_repository_in_path(repo.path().to_str().unwrap()).expect("should open repository")
}

fn git_rev_parse(repo: &TestRepo, rev: &str) -> String {
    repo.git(&["rev-parse", rev])
        .expect("rev-parse should succeed")
        .trim()
        .to_string()
}

fn write_file(repo: &TestRepo, path: &str, contents: &str) {
    let file_path = repo.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(file_path, contents).unwrap();
}

fn create_ai_commit(
    repo: &TestRepo,
    filename: &str,
    initial_content: &str,
    final_content: &str,
) -> String {
    let transcript_path = fixture_path("continue-cli-session-simple.json")
        .to_string_lossy()
        .to_string();
    let file_path = repo.path().join(filename);

    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&file_path, initial_content).unwrap();
    repo.stage_all_and_commit("Initial commit").unwrap();

    fs::write(&file_path, final_content).unwrap();

    let hook_input = json!({
        "session_id": format!("git2-migration-{}", filename.replace('/', "-")),
        "cwd": repo.canonical_path().to_string_lossy().to_string(),
        "hook_event_name": "PostToolUse",
        "model": "claude-3.5-sonnet",
        "tool_input": {
            "file_path": file_path.to_string_lossy().to_string()
        },
        "transcript_path": transcript_path
    })
    .to_string();

    repo.git_ai(&["checkpoint", "continue-cli", "--hook-input", &hook_input])
        .expect("checkpoint should succeed");

    repo.stage_all_and_commit("Add AI edits")
        .unwrap()
        .commit_sha
}

fn create_merge_heavy_history(repo: &TestRepo) -> (String, String, Vec<String>) {
    write_file(repo, "shared.txt", "base\n");
    let base = repo.stage_all_and_commit("base").unwrap().commit_sha;
    let trunk = repo.current_branch();

    write_file(repo, "trunk.txt", "trunk-1\n");
    let trunk_one = repo.stage_all_and_commit("trunk one").unwrap().commit_sha;

    repo.git(&["checkout", "-b", "feature", &base])
        .expect("create feature branch");
    write_file(repo, "feature.txt", "feature-1\n");
    let feature_one = repo.stage_all_and_commit("feature one").unwrap().commit_sha;
    write_file(repo, "feature.txt", "feature-1\nfeature-2\n");
    let feature_two = repo.stage_all_and_commit("feature two").unwrap().commit_sha;

    repo.git(&["checkout", &trunk]).expect("return to trunk");
    write_file(repo, "trunk.txt", "trunk-1\ntrunk-2\n");
    let trunk_two = repo.stage_all_and_commit("trunk two").unwrap().commit_sha;

    repo.git(&["merge", "--no-ff", "feature", "-m", "merge feature"])
        .expect("merge feature branch");
    let merge_commit = git_rev_parse(repo, "HEAD");

    (
        base,
        merge_commit,
        vec![trunk_one, feature_one, feature_two, trunk_two],
    )
}

fn extract_blame_hashes(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

#[test]
fn git2_migration_aux_comprehensive_walk_commits_to_base_matches_git_rev_list_for_merge_history() {
    let repo = TestRepo::new();
    let (base, head, _commits) = create_merge_heavy_history(&repo);
    let repository = open_repo(&repo);

    let expected: Vec<String> = repo
        .git(&[
            "rev-list",
            "--topo-order",
            "--ancestry-path",
            &format!("{}..{}", base, head),
        ])
        .expect("git rev-list should succeed")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();

    let actual = walk_commits_to_base(&repository, &head, &base).expect("walk should succeed");

    assert_eq!(actual, expected, "commit walk should preserve git CLI ordering and ancestry-path filtering");
}

#[test]
fn git2_migration_aux_comprehensive_walk_commits_to_base_returns_empty_for_same_commit() {
    let repo = TestRepo::new();
    write_file(&repo, "same.txt", "content\n");
    let commit = repo.stage_all_and_commit("single").unwrap().commit_sha;
    let repository = open_repo(&repo);

    let actual = walk_commits_to_base(&repository, &commit, &commit).expect("walk should succeed");

    assert!(actual.is_empty(), "same head/base should yield no intermediate commits");
}

#[test]
fn git2_migration_aux_comprehensive_walk_commits_to_base_rejects_missing_or_non_ancestor_base() {
    let repo = TestRepo::new();
    let (base, head, commits) = create_merge_heavy_history(&repo);
    let repository = open_repo(&repo);

    let missing = walk_commits_to_base(&repository, &head, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    assert!(missing.is_err(), "missing base commit should error");

    let non_ancestor = walk_commits_to_base(&repository, &commits[1], &commits[3]);
    let message = non_ancestor.expect_err("non-ancestor base should error").to_string();
    assert!(message.contains("not an ancestor") || message.contains(&base), "unexpected non-ancestor error: {message}");
}

#[test]
fn git2_migration_aux_comprehensive_search_by_commit_range_is_empty_for_empty_range() {
    let repo = TestRepo::new();
    let head = create_ai_commit(&repo, "search-empty.ts", "const a = 1;\n", "const a = 1;\nconst b = 2;\n");
    let repository = open_repo(&repo);

    let result = search_by_commit_range(&repository, &head, &head).expect("empty range search should succeed");

    assert!(result.is_empty(), "same start/end should not search any commits");
}

#[test]
fn git2_migration_aux_comprehensive_search_by_commit_range_returns_single_commit_results() {
    let repo = TestRepo::new();
    let commit = create_ai_commit(&repo, "search-single.ts", "const a = 1;\n", "const a = 1;\nconst b = 2;\n");
    let parent = git_rev_parse(&repo, "HEAD^");
    let repository = open_repo(&repo);

    let result = search_by_commit_range(&repository, &parent, &commit).expect("single commit range should succeed");

    assert!(!result.is_empty(), "single commit range should surface AI prompt metadata");
    assert!(
        result
            .prompt_commits
            .values()
            .flatten()
            .any(|sha| sha == &commit),
        "single commit range should attribute prompts to the ending commit"
    );
}

#[test]
fn git2_migration_aux_comprehensive_blame_abbrev_hashes_match_git_for_default_and_explicit_widths() {
    let repo = TestRepo::new();
    write_file(&repo, "blame.txt", "root\nshared\n");
    repo.stage_all_and_commit("root").unwrap();
    write_file(&repo, "blame.txt", "root\nchanged once\n");
    repo.stage_all_and_commit("change once").unwrap();
    write_file(&repo, "blame.txt", "root\nchanged once\nchanged twice\n");
    repo.stage_all_and_commit("change twice").unwrap();

    for extra in [None, Some("12"), Some("4")] {
        let git_output = match extra {
            Some(width) => repo
                .git(&["blame", &format!("--abbrev={width}"), "blame.txt"])
                .expect("git blame should succeed"),
            None => repo.git(&["blame", "blame.txt"]).expect("git blame should succeed"),
        };
        let git_ai_output = match extra {
            Some(width) => repo
                .git_ai(&["blame", "--abbrev", width, "blame.txt"])
                .expect("git-ai blame should succeed"),
            None => repo
                .git_ai(&["blame", "blame.txt"])
                .expect("git-ai blame should succeed"),
        };

        let git_hashes = extract_blame_hashes(&git_output);
        let git_ai_hashes = extract_blame_hashes(&git_ai_output);
        assert_eq!(git_ai_hashes, git_hashes, "abbreviated blame SHAs should match git for width {extra:?}");
    }
}

#[test]
fn git2_migration_aux_comprehensive_ref_exists_tracks_existing_and_missing_refs() {
    let repo = TestRepo::new();
    write_file(&repo, "refs.txt", "content\n");
    repo.stage_all_and_commit("refs").unwrap();
    let repository = open_repo(&repo);
    let branch_name = repo.current_branch();

    assert!(ref_exists(&repository, "HEAD"), "HEAD should always resolve");
    assert!(
        ref_exists(&repository, &format!("refs/heads/{branch_name}")),
        "current branch ref should exist"
    );
    assert!(
        !ref_exists(&repository, "refs/heads/does-not-exist"),
        "missing branch ref should report false"
    );
}

#[test]
fn git2_migration_aux_comprehensive_copy_ref_creates_missing_destination_and_preserves_target() {
    let repo = TestRepo::new();
    write_file(&repo, "copy-create.txt", "content\n");
    repo.stage_all_and_commit("copy create").unwrap();
    let repository = open_repo(&repo);

    assert!(!ref_exists(&repository, "refs/notes/ai-backup"));

    copy_ref(&repository, "HEAD", "refs/notes/ai-backup").expect("copy_ref should create destination");

    assert!(ref_exists(&repository, "refs/notes/ai-backup"));
    assert_eq!(git_rev_parse(&repo, "HEAD"), git_rev_parse(&repo, "refs/notes/ai-backup"));
}

#[test]
fn git2_migration_aux_comprehensive_copy_ref_overwrites_existing_destination() {
    let repo = TestRepo::new();
    write_file(&repo, "copy-overwrite.txt", "first\n");
    let first = repo.stage_all_and_commit("first").unwrap().commit_sha;
    write_file(&repo, "copy-overwrite.txt", "second\n");
    let second = repo.stage_all_and_commit("second").unwrap().commit_sha;
    let repository = open_repo(&repo);

    repo.git(&["update-ref", "refs/notes/overwrite-dest", &first])
        .expect("seed destination ref");
    assert_eq!(git_rev_parse(&repo, "refs/notes/overwrite-dest"), first);

    copy_ref(&repository, "HEAD", "refs/notes/overwrite-dest").expect("copy_ref should overwrite existing destination");

    assert_eq!(git_rev_parse(&repo, "refs/notes/overwrite-dest"), second);
}

#[test]
fn git2_migration_aux_comprehensive_copy_ref_errors_for_missing_source_without_creating_destination() {
    let repo = TestRepo::new();
    write_file(&repo, "copy-missing.txt", "content\n");
    repo.stage_all_and_commit("copy missing").unwrap();
    let repository = open_repo(&repo);

    let result = copy_ref(&repository, "refs/heads/does-not-exist", "refs/notes/missing-copy");

    assert!(result.is_err(), "missing source ref should fail copy_ref");
    assert!(
        !ref_exists(&repository, "refs/notes/missing-copy"),
        "failed copy_ref should not create the destination ref"
    );
}
