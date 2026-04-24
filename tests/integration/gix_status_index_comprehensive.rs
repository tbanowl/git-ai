//! Comprehensive tests for status/index/staged-content paths that should migrate via `gix`.

use crate::repos::test_repo::TestRepo;
use git_ai::git::repo_state::read_head_state_for_worktree;
use git_ai::git::repository::{Repository, find_repository_in_path};
use git_ai::git::status::{EntryKind, StatusCode, StatusEntry};
use std::collections::{HashMap, HashSet};
use std::fs;

fn open_repo(repo: &TestRepo) -> Repository {
    find_repository_in_path(repo.path().to_str().unwrap()).expect("should open repository")
}

fn write_file(repo: &TestRepo, path: &str, contents: impl AsRef<[u8]>) {
    let file_path = repo.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("should create parent directories");
    }
    fs::write(file_path, contents).expect("should write file");
}

fn pathset(paths: &[&str]) -> HashSet<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}

fn status_entry_by_path<'a>(entries: &'a [StatusEntry], path: &str) -> &'a StatusEntry {
    entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("missing status entry for {path}"))
}

fn sorted_paths(set: HashSet<String>) -> Vec<String> {
    let mut paths: Vec<String> = set.into_iter().collect();
    paths.sort();
    paths
}

#[test]
fn gix_status_index_comprehensive_get_staged_filenames_only_reports_staged_paths_in_mixed_worktree() {
    let repo = TestRepo::new();

    write_file(&repo, "staged.txt", "base staged\n");
    write_file(&repo, "unstaged.txt", "base unstaged\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "staged.txt", "base staged\nstaged change\n");
    repo.git_og(&["add", "staged.txt"]).unwrap();

    write_file(&repo, "unstaged.txt", "base unstaged\nunstaged change\n");
    write_file(&repo, "untracked.txt", "untracked\n");

    let repository = open_repo(&repo);
    let staged = sorted_paths(repository.get_staged_filenames().unwrap());

    assert_eq!(staged, vec!["staged.txt".to_string()]);
}

#[test]
fn gix_status_index_comprehensive_get_staged_and_unstaged_filenames_includes_untracked_but_not_ignored() {
    let repo = TestRepo::new();

    write_file(&repo, ".gitignore", "ignored.log\n");
    write_file(&repo, "tracked.txt", "one\n");
    write_file(&repo, "staged.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "tracked.txt", "one\ntwo\n");
    write_file(&repo, "staged.txt", "seed\nstaged\n");
    repo.git_og(&["add", "staged.txt"]).unwrap();
    write_file(&repo, "new file.txt", "hello\n");
    write_file(&repo, "ignored.log", "ignore me\n");

    let repository = open_repo(&repo);
    let filenames = sorted_paths(repository.get_staged_and_unstaged_filenames().unwrap());

    assert_eq!(
        filenames,
        vec![
            "new file.txt".to_string(),
            "staged.txt".to_string(),
            "tracked.txt".to_string(),
        ]
    );
}

#[test]
fn gix_status_index_comprehensive_get_staged_and_unstaged_filenames_recurses_untracked_directories() {
    let repo = TestRepo::new();

    write_file(&repo, ".gitignore", "ignored-dir/\n");
    write_file(&repo, "tracked.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "newdir/nested/file.txt", "hello\n");
    write_file(&repo, "ignored-dir/skip.txt", "ignore me\n");

    let repository = open_repo(&repo);
    let filenames = sorted_paths(repository.get_staged_and_unstaged_filenames().unwrap());

    assert_eq!(
        filenames,
        vec!["newdir/".to_string()],
        "current status scan reports the top-level untracked directory path"
    );
}

#[test]
fn gix_status_index_comprehensive_status_toggles_untracked_entries_with_skip_untracked() {
    let repo = TestRepo::new();

    write_file(&repo, "staged.txt", "seed\n");
    write_file(&repo, "unstaged.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "staged.txt", "seed\nstaged\n");
    repo.git_og(&["add", "staged.txt"]).unwrap();
    write_file(&repo, "unstaged.txt", "seed\nunstaged\n");
    write_file(&repo, "untracked.txt", "brand new\n");

    let repository = open_repo(&repo);
    let pathspecs = pathset(&["unstaged.txt", "untracked.txt"]);
    let without_untracked = repository.status(Some(&pathspecs), true).unwrap();
    let with_untracked = repository.status(Some(&pathspecs), false).unwrap();

    let staged = status_entry_by_path(&without_untracked, "staged.txt");
    assert_eq!(staged.staged, StatusCode::Modified);
    assert_eq!(staged.unstaged, StatusCode::Unmodified);

    let unstaged = status_entry_by_path(&without_untracked, "unstaged.txt");
    assert_eq!(unstaged.staged, StatusCode::Unmodified);
    assert_eq!(unstaged.unstaged, StatusCode::Modified);

    assert!(without_untracked.iter().all(|entry| entry.path != "untracked.txt"));

    let untracked = status_entry_by_path(&with_untracked, "untracked.txt");
    assert_eq!(untracked.kind, EntryKind::Untracked);
    assert_eq!(untracked.unstaged, StatusCode::Untracked);
}

#[test]
fn gix_status_index_comprehensive_status_pathspec_union_keeps_staged_paths_and_filters_other_unstaged_paths() {
    let repo = TestRepo::new();

    write_file(&repo, "staged.txt", "seed\n");
    write_file(&repo, "selected.txt", "seed\n");
    write_file(&repo, "other.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "staged.txt", "seed\nstaged\n");
    repo.git_og(&["add", "staged.txt"]).unwrap();
    write_file(&repo, "selected.txt", "seed\nselected\n");
    write_file(&repo, "other.txt", "seed\nother\n");

    let repository = open_repo(&repo);
    let pathspecs = pathset(&["selected.txt"]);
    let mut entries = repository.status(Some(&pathspecs), true).unwrap();
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    let paths: Vec<String> = entries.into_iter().map(|entry| entry.path).collect();
    assert_eq!(paths, vec!["selected.txt".to_string(), "staged.txt".to_string()]);
}

#[test]
fn gix_status_index_comprehensive_status_post_filter_handles_non_ascii_and_space_paths() {
    let repo = TestRepo::new();

    write_file(&repo, "unicodé/文件.txt", "base\n");
    write_file(&repo, "space dir/hello world.txt", "base\n");
    write_file(&repo, "other.txt", "base\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "unicodé/文件.txt", "base\nchanged\n");
    write_file(&repo, "space dir/hello world.txt", "base\nchanged\n");
    write_file(&repo, "other.txt", "base\nchanged\n");

    let repository = open_repo(&repo);
    let pathspecs = pathset(&["unicodé/文件.txt", "space dir/hello world.txt"]);
    let mut entries = repository.status(Some(&pathspecs), true).unwrap();
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    let paths: Vec<String> = entries.into_iter().map(|entry| entry.path).collect();
    assert_eq!(
        paths,
        vec![
            "space dir/hello world.txt".to_string(),
            "unicodé/文件.txt".to_string(),
        ]
    );
}

#[test]
fn gix_status_index_comprehensive_status_without_pathspecs_still_reports_pure_unstaged_changes() {
    let repo = TestRepo::new();

    write_file(&repo, "tracked.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "tracked.txt", "seed\nunstaged\n");

    let repository = open_repo(&repo);
    let entries = repository.status(None, true).unwrap();

    let tracked = status_entry_by_path(&entries, "tracked.txt");
    assert_eq!(tracked.kind, EntryKind::Ordinary);
    assert_eq!(tracked.staged, StatusCode::Unmodified);
    assert_eq!(tracked.unstaged, StatusCode::Modified);
}

#[test]
fn gix_status_index_comprehensive_status_reports_staged_deletions() {
    let repo = TestRepo::new();

    write_file(&repo, "gone.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    std::fs::remove_file(repo.path().join("gone.txt")).unwrap();
    repo.git_og(&["add", "gone.txt"]).unwrap();

    let repository = open_repo(&repo);
    let entries = repository.status(None, true).unwrap();

    let gone = status_entry_by_path(&entries, "gone.txt");
    assert_eq!(gone.kind, EntryKind::Ordinary);
    assert_eq!(gone.staged, StatusCode::Deleted);
    assert_eq!(gone.unstaged, StatusCode::Unmodified);
}

#[test]
fn gix_status_index_comprehensive_branch_metadata_tracks_attached_and_detached_head_contract() {
    let repo = TestRepo::new();

    write_file(&repo, "head.txt", "base\n");
    let commit = repo.stage_all_and_commit("initial").unwrap().commit_sha;
    let branch_name = repo.current_branch();

    let attached = read_head_state_for_worktree(repo.path()).expect("attached head state");
    assert_eq!(attached.branch.as_deref(), Some(branch_name.as_str()));
    assert_eq!(attached.head.as_deref(), Some(commit.as_str()));
    assert!(!attached.detached, "branch checkout should not be detached");

    repo.git_og(&["checkout", "--detach", &commit]).unwrap();

    let detached = read_head_state_for_worktree(repo.path()).expect("detached head state");
    assert_eq!(detached.head.as_deref(), Some(commit.as_str()));
    assert_eq!(detached.branch, None);
    assert!(detached.detached, "detached checkout should mark detached=true");
}

#[test]
fn gix_status_index_comprehensive_conflicted_index_paths_diverge_from_stage0_status_entries() {
    let repo = TestRepo::new();

    write_file(&repo, "conflicted.txt", "base\n");
    write_file(&repo, "clean.txt", "base\n");
    repo.stage_all_and_commit("initial").unwrap();

    repo.git_og(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "conflicted.txt", "feature\n");
    repo.stage_all_and_commit("feature change").unwrap();

    let trunk = repo.git_og(&["rev-parse", "--abbrev-ref", "feature@{upstream}"]);
    assert!(trunk.is_err(), "feature should not have an upstream in test repo");
    repo.git_og(&["checkout", "-"]).unwrap();

    write_file(&repo, "conflicted.txt", "main\n");
    repo.stage_all_and_commit("main change").unwrap();

    let merge = repo.git_og(&["merge", "feature"]);
    assert!(merge.is_err(), "merge should leave conflicted index entries");

    write_file(&repo, "clean.txt", "base\nclean staged\n");
    repo.git_og(&["add", "clean.txt"]).unwrap();

    let repository = open_repo(&repo);
    let staged = repository.get_staged_filenames().unwrap();
    assert!(staged.contains("clean.txt"));
    assert!(
        staged.contains("conflicted.txt"),
        "current staged filename scan should surface conflicted paths present in the index"
    );

    let pathspecs = pathset(&["conflicted.txt", "clean.txt"]);
    let entries = repository.status(Some(&pathspecs), true).unwrap();
    assert!(
        entries.iter().all(|entry| entry.path != "conflicted.txt"),
        "current status() filtering should not surface the unresolved path here"
    );

    let clean = status_entry_by_path(&entries, "clean.txt");
    assert_eq!(clean.kind, EntryKind::Ordinary);
    assert_eq!(clean.staged, StatusCode::Modified);
}

#[test]
fn gix_status_index_comprehensive_get_all_staged_files_content_reads_text_and_skips_binary() {
    let repo = TestRepo::new();

    write_file(&repo, "text.txt", "base\n");
    write_file(&repo, "binary.bin", b"\x00\x01seed");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "text.txt", "base\nhello\n");
    write_file(&repo, "binary.bin", [0xff, 0xfe, 0xfd, 0x00]);
    repo.git_og(&["add", "text.txt", "binary.bin"]).unwrap();

    let repository = open_repo(&repo);
    let contents = repository
        .get_all_staged_files_content(&["text.txt".to_string(), "binary.bin".to_string()])
        .unwrap();

    assert_eq!(contents.get("text.txt"), Some(&"base\nhello\n".to_string()));
    assert!(
        !contents.contains_key("binary.bin"),
        "binary staged blobs should be skipped when not valid UTF-8"
    );
}

#[test]
fn gix_status_index_comprehensive_get_all_staged_files_content_skips_conflicted_paths_but_keeps_stage0_entries() {
    let repo = TestRepo::new();

    write_file(&repo, "conflicted.txt", "base\n");
    write_file(&repo, "clean.txt", "base\n");
    repo.stage_all_and_commit("initial").unwrap();

    repo.git_og(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "conflicted.txt", "feature\n");
    repo.stage_all_and_commit("feature change").unwrap();
    repo.git_og(&["checkout", "-"]).unwrap();

    write_file(&repo, "conflicted.txt", "main\n");
    repo.stage_all_and_commit("main change").unwrap();

    let merge = repo.git_og(&["merge", "feature"]);
    assert!(merge.is_err(), "merge should produce unresolved conflict");

    write_file(&repo, "clean.txt", "base\nclean staged\n");
    repo.git_og(&["add", "clean.txt"]).unwrap();

    let repository = open_repo(&repo);
    let contents: HashMap<String, String> = repository
        .get_all_staged_files_content(&[
            "conflicted.txt".to_string(),
            "clean.txt".to_string(),
        ])
        .unwrap();

    assert_eq!(contents.get("clean.txt"), Some(&"base\nclean staged\n".to_string()));
    assert!(
        !contents.contains_key("conflicted.txt"),
        "unmerged index entries should not be surfaced as stage-0 content"
    );
}

#[test]
fn gix_status_index_comprehensive_new_status_matches_cli_for_mixed_repo() {
    let repo = TestRepo::new();

    write_file(&repo, "rename-me.txt", "seed\n");
    write_file(&repo, "tracked.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    repo.git_og(&["mv", "rename-me.txt", "renamed.txt"]).unwrap();
    write_file(&repo, "tracked.txt", "seed\nunstaged\n");
    write_file(&repo, "untracked.txt", "brand new\n");

    let repository = open_repo(&repo);
    let mut new_entries = repository.status(None, false).unwrap();
    new_entries.sort_by(|left, right| left.path.cmp(&right.path));

    let mut old_entries = git_ai::git::status::old_cli_status_entries_for_test(&repository, None, false)
        .expect("old cli-backed status helper should succeed");
    old_entries.sort_by(|left, right| left.path.cmp(&right.path));

    assert_eq!(new_entries, old_entries);
}
