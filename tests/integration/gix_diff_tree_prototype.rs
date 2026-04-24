//! Prototype tests for evaluating whether `gix-diff` can preserve the current raw tree-diff contract.

use crate::repos::test_repo::TestRepo;
use git_ai::git::diff_tree_to_tree::DiffStatus;
use git_ai::git::repository::{Repository, find_repository_in_path};
use std::fs;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

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

fn rev_parse(repo: &TestRepo, spec: &str) -> String {
    repo.git_og(&["rev-parse", spec])
        .expect("rev-parse should succeed")
        .trim()
        .to_string()
}

fn tree_for_rev<'a>(repository: &'a Repository, rev: &str) -> git_ai::git::repository::Tree<'a> {
    let oid = repository
        .revparse_single(rev)
        .expect("revision should resolve")
        .id();
    repository
        .find_commit(oid)
        .expect("commit should load")
        .tree()
        .expect("tree should load")
}

#[test]
fn gix_diff_tree_prototype_preserves_current_delta_order_status_letters_oids_and_modes() {
    let repo = TestRepo::new();

    write_file(&repo, "delete.txt", "delete me\n");
    write_file(&repo, "modify.txt", "before\n");
    let base_commit = repo.stage_all_and_commit("base").unwrap().commit_sha;

    fs::remove_file(repo.path().join("delete.txt")).expect("delete should succeed");
    write_file(&repo, "modify.txt", "after\n");
    write_file(&repo, "zzz_added.txt", "brand new\n");
    let head_commit = repo.stage_all_and_commit("change set").unwrap().commit_sha;

    let repository = open_repo(&repo);
    let base_tree = tree_for_rev(&repository, &base_commit);
    let head_tree = tree_for_rev(&repository, &head_commit);

    let deltas: Vec<_> = repository
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None, None)
        .expect("diff should succeed")
        .deltas()
        .map(|delta| {
            (
                delta.status(),
                delta.old_file().path().map(|p| p.to_string_lossy().to_string()),
                delta.new_file().path().map(|p| p.to_string_lossy().to_string()),
                delta.old_file().mode().to_string(),
                delta.new_file().mode().to_string(),
                delta.old_file().id().to_string(),
                delta.new_file().id().to_string(),
            )
        })
        .collect();

    assert_eq!(
        deltas,
        vec![
            (
                DiffStatus::Deleted,
                Some("delete.txt".to_string()),
                Some("delete.txt".to_string()),
                "100644".to_string(),
                "000000".to_string(),
                rev_parse(&repo, &format!("{}:delete.txt", base_commit)),
                ZERO_OID.to_string(),
            ),
            (
                DiffStatus::Modified,
                Some("modify.txt".to_string()),
                Some("modify.txt".to_string()),
                "100644".to_string(),
                "100644".to_string(),
                rev_parse(&repo, &format!("{}:modify.txt", base_commit)),
                rev_parse(&repo, &format!("{}:modify.txt", head_commit)),
            ),
            (
                DiffStatus::Added,
                Some("zzz_added.txt".to_string()),
                Some("zzz_added.txt".to_string()),
                "000000".to_string(),
                "100644".to_string(),
                ZERO_OID.to_string(),
                rev_parse(&repo, &format!("{}:zzz_added.txt", head_commit)),
            ),
        ],
        "prototype should lock the current raw CLI-backed delta sequence and metadata"
    );
}

#[test]
fn gix_diff_tree_prototype_preserves_current_rename_path_shape_and_metadata() {
    let repo = TestRepo::new();

    write_file(&repo, "before name.txt", "rename me\n");
    let base_commit = repo.stage_all_and_commit("base").unwrap().commit_sha;

    fs::create_dir_all(repo.path().join("renamed dir")).expect("should create rename directory");
    repo.git_og(&["mv", "before name.txt", "renamed dir/after name.txt"])
        .expect("rename should succeed");
    let head_commit = repo.stage_all_and_commit("rename only").unwrap().commit_sha;

    let repository = open_repo(&repo);
    let base_tree = tree_for_rev(&repository, &base_commit);
    let head_tree = tree_for_rev(&repository, &head_commit);

    let deltas: Vec<_> = repository
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None, None)
        .expect("diff should succeed")
        .deltas()
        .map(|delta| {
            (
                delta.status(),
                delta.old_file().path().map(|p| p.to_string_lossy().to_string()),
                delta.new_file().path().map(|p| p.to_string_lossy().to_string()),
                delta.old_file().mode().to_string(),
                delta.new_file().mode().to_string(),
                delta.old_file().id().to_string(),
                delta.new_file().id().to_string(),
                delta.similarity(),
            )
        })
        .collect();

    assert_eq!(
        deltas,
        vec![(
            DiffStatus::Renamed,
            Some("renamed dir/after name.txt".to_string()),
            Some("before name.txt".to_string()),
            "100644".to_string(),
            "100644".to_string(),
            rev_parse(&repo, &format!("{}:before name.txt", base_commit)),
            rev_parse(&repo, &format!("{}:renamed dir/after name.txt", head_commit)),
            100,
        )],
        "prototype should capture the current raw parser rename path orientation"
    );
}

#[test]
fn gix_diff_tree_prototype_keeps_copy_like_changes_as_added_entries() {
    let repo = TestRepo::new();

    write_file(&repo, "source.txt", "same blob\n");
    let base_commit = repo.stage_all_and_commit("base").unwrap().commit_sha;

    repo.git_og(&["config", "diff.renames", "copies"])
        .expect("should enable copy-friendly config");
    write_file(&repo, "copy target.txt", "same blob\n");
    let head_commit = repo.stage_all_and_commit("copy-like add").unwrap().commit_sha;

    let repository = open_repo(&repo);
    let base_tree = tree_for_rev(&repository, &base_commit);
    let head_tree = tree_for_rev(&repository, &head_commit);

    let deltas: Vec<_> = repository
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None, None)
        .expect("diff should succeed")
        .deltas()
        .map(|delta| {
            (
                delta.status(),
                delta.old_file().path().map(|p| p.to_string_lossy().to_string()),
                delta.new_file().path().map(|p| p.to_string_lossy().to_string()),
            )
        })
        .collect();

    assert_eq!(
        deltas,
        vec![(
            DiffStatus::Added,
            Some("copy target.txt".to_string()),
            Some("copy target.txt".to_string()),
        )],
        "current CLI-backed tree diff does not surface copy deltas here; keep that prototype contract explicit"
    );

    assert_eq!(
        rev_parse(&repo, &format!("{}:source.txt", base_commit)),
        rev_parse(&repo, &format!("{}:copy target.txt", head_commit)),
        "fixture should remain a pure blob copy so the prototype documents current non-copy behavior"
    );
}

#[test]
fn gix_diff_tree_prototype_preserves_raw_paths_with_spaces_and_non_ascii() {
    let repo = TestRepo::new();

    write_file(&repo, "space dir/hello world.txt", "before\n");
    write_file(&repo, "unicodé/文件.txt", "before\n");
    repo.stage_all_and_commit("base").unwrap();

    write_file(&repo, "space dir/hello world.txt", "after\n");
    write_file(&repo, "unicodé/文件.txt", "after\n");
    repo.stage_all_and_commit("update paths").unwrap();

    let repository = open_repo(&repo);
    let base_tree = tree_for_rev(&repository, "HEAD~1");
    let head_tree = tree_for_rev(&repository, "HEAD");

    let paths: Vec<_> = repository
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None, None)
        .expect("diff should succeed")
        .deltas()
        .map(|delta| {
            (
                delta.status(),
                delta.new_file()
                    .path()
                    .map(|path| path.to_string_lossy().to_string())
                    .expect("path should be utf-8"),
            )
        })
        .collect();

    assert_eq!(
        paths,
        vec![
            (DiffStatus::Modified, "space dir/hello world.txt".to_string()),
            (DiffStatus::Modified, "unicodé/文件.txt".to_string()),
        ],
        "prototype should lock the current raw parser path strings for spaces and non-ASCII paths"
    );
}
