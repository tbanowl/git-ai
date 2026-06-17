//! Comprehensive tests for non-`repository.rs` git2 migration targets.

use crate::repos::test_repo::TestRepo;
use crate::test_utils::fixture_path;
use git_ai::authorship::rebase_authorship::walk_commits_to_base;
use git_ai::commands::diff::{DiffOptions, get_diff_json_filtered};
use git_ai::commands::hooks::rebase_hooks::build_rebase_commit_mappings;
use git_ai::commands::search::search_by_commit_range;
use git_ai::git::refs::{copy_ref, ref_exists};
use git_ai::git::repository::{Repository, find_repository_in_path};
use git_ai::git::rewrite_log::RewriteLogEvent;
use rusqlite::Connection;
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

fn default_branch(repo: &TestRepo) -> String {
    repo.git(&["branch", "--show-current"])
        .expect("branch --show-current should succeed")
        .trim()
        .to_string()
}

fn repo_rewrite_events(repo: &TestRepo) -> Vec<RewriteLogEvent> {
    repo.sync_daemon();
    open_repo(repo)
        .storage
        .read_rewrite_events()
        .expect("rewrite log should be readable")
}

fn commit_subject(repo: &TestRepo, rev: &str) -> String {
    repo.git(&["log", "--format=%s", "-1", rev])
        .expect("git log --format=%s should succeed")
        .trim()
        .to_string()
}

fn extract_blame_hashes(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

#[test]
fn git2_migration_aux_comprehensive_prompts_reachable_commit_rows_match_git_rev_list_all_for_noted_commits()
 {
    let mut repo = TestRepo::new_dedicated_daemon();
    repo.patch_git_ai_config(|patch| {
        patch.exclude_prompts_in_repositories = Some(vec![]);
        patch.prompt_storage = Some("notes".to_string());
    });

    let main_prompt = create_ai_commit(&repo, "main.ts", "const main = 1;\n", "const main = 2;\n");
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"])
        .expect("create feature branch");
    let feature_prompt = create_ai_commit(
        &repo,
        "feature.ts",
        "const feature = 1;\n",
        "const feature = 2;\n",
    );

    repo.git(&["checkout", &trunk]).expect("return to trunk");
    let orphan_prompt = create_ai_commit(
        &repo,
        "orphan.ts",
        "const orphan = 1;\n",
        "const orphan = 2;\n",
    );
    repo.git(&["reset", "--hard", "HEAD^"])
        .expect("drop orphan prompt commit from reachable history");

    let reachable_from_git: std::collections::HashSet<String> = repo
        .git(&["rev-list", "--all"])
        .expect("git rev-list --all should succeed")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();

    repo.git_ai(&["prompts"])
        .expect("prompts populate should succeed");

    let conn = Connection::open(repo.path().join("prompts.db")).expect("open prompts db");
    let actual: std::collections::HashSet<String> = conn
        .prepare("SELECT DISTINCT commit_sha FROM prompts WHERE commit_sha IS NOT NULL")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let expected: std::collections::HashSet<String> = [
        main_prompt.clone(),
        feature_prompt.clone(),
        orphan_prompt.clone(),
    ]
    .into_iter()
    .filter(|sha| reachable_from_git.contains(sha))
    .collect();

    assert_eq!(
        actual, expected,
        "prompt commit rows should match the git-reachable subset of noted commits"
    );
    assert!(
        !actual.contains(&orphan_prompt),
        "orphaned noted commit should be excluded by the production reachable_commits path"
    );
}

#[test]
fn git2_migration_aux_comprehensive_diff_single_commit_resolution_matches_git_rev_parse_for_head_sha_and_branch()
 {
    let repo = TestRepo::new();
    let commit = create_ai_commit(
        &repo,
        "resolve-commit.ts",
        "const a = 1;\n",
        "const a = 2;\n",
    );
    let repository = open_repo(&repo);
    let branch = repo.current_branch();

    for rev in ["HEAD", commit.as_str(), branch.as_str()] {
        let expected = git_rev_parse(&repo, rev);
        let diff_json = get_diff_json_filtered(&repository, rev, DiffOptions::default())
            .expect("single-commit diff should resolve");

        let actual: std::collections::HashSet<&str> = diff_json
            .hunks
            .iter()
            .map(|hunk| hunk.commit_sha.as_str())
            .collect();

        assert_eq!(
            actual.len(),
            1,
            "expected a single resolved commit for rev {rev}"
        );
        assert!(
            actual.contains(expected.as_str()),
            "resolved commit should match git rev-parse for rev {rev}"
        );
        assert!(
            diff_json.commits.contains_key(&expected),
            "commit metadata should be keyed by the git-resolved SHA for rev {rev}"
        );
    }
}

#[test]
fn git2_migration_aux_comprehensive_diff_single_commit_resolution_rejects_invalid_revspec() {
    let repo = TestRepo::new();
    let _commit = create_ai_commit(
        &repo,
        "resolve-invalid.ts",
        "const a = 1;\n",
        "const b = 2;\n",
    );
    let repository = open_repo(&repo);
    let invalid_rev = "definitely-not-a-real-revision";

    let git_error = repo
        .git(&["rev-parse", invalid_rev])
        .expect_err("git rev-parse should fail for invalid rev");

    let diff_error = get_diff_json_filtered(&repository, invalid_rev, DiffOptions::default())
        .expect_err("diff resolution should fail for invalid rev")
        .to_string();

    assert!(
        diff_error.contains("rev-parse") || diff_error.contains(invalid_rev),
        "diff error should surface rev resolution failure, got: {diff_error}"
    );
    assert!(
        git_error.contains(invalid_rev),
        "git rev-parse error should mention the invalid rev, got: {git_error}"
    );
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

    assert_eq!(
        actual, expected,
        "commit walk should preserve git CLI ordering and ancestry-path filtering"
    );
}

#[test]
fn git2_migration_aux_comprehensive_walk_commits_to_base_returns_empty_for_same_commit() {
    let repo = TestRepo::new();
    write_file(&repo, "same.txt", "content\n");
    let commit = repo.stage_all_and_commit("single").unwrap().commit_sha;
    let repository = open_repo(&repo);

    let actual = walk_commits_to_base(&repository, &commit, &commit).expect("walk should succeed");

    assert!(
        actual.is_empty(),
        "same head/base should yield no intermediate commits"
    );
}

#[test]
fn git2_migration_aux_comprehensive_walk_commits_to_base_rejects_missing_or_non_ancestor_base() {
    let repo = TestRepo::new();
    let (base, head, commits) = create_merge_heavy_history(&repo);
    let repository = open_repo(&repo);

    let missing = walk_commits_to_base(
        &repository,
        &head,
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
    );
    assert!(missing.is_err(), "missing base commit should error");

    let non_ancestor = walk_commits_to_base(&repository, &commits[1], &commits[3]);
    let message = non_ancestor
        .expect_err("non-ancestor base should error")
        .to_string();
    assert!(
        message.contains("not an ancestor") || message.contains(&base),
        "unexpected non-ancestor error: {message}"
    );
}

#[test]
fn git2_migration_aux_comprehensive_rebase_ancestor_family_covers_equal_ancestor_non_ancestor_and_missing()
 {
    let repo = TestRepo::new();
    write_file(&repo, "ancestor-family.txt", "base\n");
    let base = repo.stage_all_and_commit("base").unwrap().commit_sha;

    write_file(&repo, "ancestor-family.txt", "base\nchild\n");
    let child = repo.stage_all_and_commit("child").unwrap().commit_sha;

    repo.git(&["checkout", "-b", "side", &base])
        .expect("create side branch");
    write_file(&repo, "side-only.txt", "side\n");
    let side = repo.stage_all_and_commit("side").unwrap().commit_sha;

    let repository = open_repo(&repo);

    let equal = build_rebase_commit_mappings(&repository, &child, &child, Some(&child))
        .expect("equal heads should produce an empty mapping instead of erroring");
    assert!(
        equal.0.is_empty(),
        "equal case should have no original commits to rewrite"
    );
    assert!(
        equal.1.is_empty(),
        "equal case should have no new commits to rewrite"
    );

    let ancestor = build_rebase_commit_mappings(&repository, &child, &side, Some(&base))
        .expect("valid ancestor lower bound should succeed");
    assert_eq!(
        ancestor.0,
        vec![child.clone()],
        "ancestor case should keep the rewritten original commit"
    );
    assert_eq!(
        ancestor.1,
        vec![side.clone()],
        "ancestor case should map to the rewritten descendant commit"
    );

    let non_ancestor = build_rebase_commit_mappings(&repository, &child, &side, Some(&child))
        .expect("non-ancestor onto should be ignored rather than erroring");
    assert_eq!(
        non_ancestor.0,
        vec![child.clone()],
        "non-ancestor onto fallback should still keep the original commit lane"
    );
    assert_eq!(
        non_ancestor.1,
        vec![side.clone()],
        "non-ancestor onto fallback should use merge-base-derived rewritten commits"
    );

    let missing = build_rebase_commit_mappings(
        &repository,
        &child,
        &side,
        Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
    )
    .expect("syntactically valid but missing onto oid should be ignored like a non-ancestor lower bound");
    assert_eq!(
        missing.0,
        vec![child],
        "missing-object case should keep the original commit lane instead of failing"
    );
    assert_eq!(
        missing.1,
        vec![side],
        "missing-object case should fall back to merge-base-derived rewritten commits"
    );
}

#[test]
fn git2_migration_aux_comprehensive_rebase_commit_mappings_match_first_parent_git_rev_list() {
    let repo = TestRepo::new();
    write_file(&repo, "shared.txt", "base\n");
    repo.stage_all_and_commit("base").unwrap();
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "feature-only.txt", "feature-1\n");
    repo.stage_all_and_commit("feature commit 1").unwrap();
    write_file(&repo, "feature-only.txt", "feature-1\nfeature-2\n");
    let original_head = repo
        .stage_all_and_commit("feature commit 2")
        .unwrap()
        .commit_sha;

    repo.git(&["checkout", &trunk]).unwrap();
    write_file(&repo, "main-only.txt", "main\n");
    let onto_head = repo.stage_all_and_commit("main change").unwrap().commit_sha;

    repo.git(&["checkout", "feature"]).unwrap();
    repo.git(&["rebase", &trunk])
        .expect("rebase should succeed");
    let new_head = git_rev_parse(&repo, "HEAD");
    let repository = open_repo(&repo);

    let expected_new: Vec<String> = repo
        .git(&[
            "rev-list",
            "--first-parent",
            "--topo-order",
            "--max-count=2",
            &format!("{}..{}", onto_head, new_head),
        ])
        .expect("git rev-list --first-parent should succeed")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let (original_commits, new_commits) =
        build_rebase_commit_mappings(&repository, &original_head, &new_head, Some(&onto_head))
            .expect("rebase mappings should succeed");

    assert_eq!(
        new_commits, expected_new,
        "rebased commit mapping should follow git first-parent history in replay order (oldest to newest)"
    );
    assert_eq!(
        original_commits.len(),
        2,
        "original feature lane should contain both feature commits"
    );
    assert_eq!(
        new_commits.len(),
        2,
        "rebased lane should contain both rewritten commits"
    );
}

#[test]
fn git2_migration_aux_comprehensive_cherry_pick_range_rewrite_log_matches_git_rev_list_reverse_order()
 {
    let repo = TestRepo::new();
    write_file(&repo, "range-base.txt", "base\n");
    repo.stage_all_and_commit("base").unwrap();
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "one.txt", "one\n");
    let first = repo.stage_all_and_commit("feature one").unwrap().commit_sha;
    write_file(&repo, "two.txt", "two\n");
    let second = repo.stage_all_and_commit("feature two").unwrap().commit_sha;
    write_file(&repo, "three.txt", "three\n");
    let third = repo
        .stage_all_and_commit("feature three")
        .unwrap()
        .commit_sha;

    let expected = repo
        .git(&["rev-list", "--reverse", &format!("{}..{}", first, third)])
        .expect("git rev-list --reverse should succeed")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        expected,
        vec![second.clone(), third.clone()],
        "sanity check expected git range ordering"
    );

    repo.git(&["checkout", &trunk]).unwrap();
    repo.git(&["cherry-pick", &format!("{}..{}", first, third)])
        .expect("range cherry-pick should succeed");

    let start = repo_rewrite_events(&repo)
        .into_iter()
        .find_map(|event| match event {
            RewriteLogEvent::CherryPickStart { cherry_pick_start } => Some(cherry_pick_start),
            _ => None,
        })
        .expect("cherry-pick start event should exist");

    assert_eq!(
        start.source_commits, expected,
        "rewrite log source commits should follow git rev-list --reverse range expansion"
    );
}

#[test]
fn git2_migration_aux_comprehensive_cherry_pick_short_sha_rewrite_log_matches_git_rev_parse() {
    let repo = TestRepo::new();
    write_file(&repo, "short-sha.txt", "base\n");
    repo.stage_all_and_commit("base").unwrap();
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "short-sha.txt", "base\nfeature\n");
    let commit = repo
        .stage_all_and_commit("feature short sha")
        .unwrap()
        .commit_sha;
    let short = &commit[..8];
    let expected = git_rev_parse(&repo, short);

    repo.git(&["checkout", &trunk]).unwrap();
    repo.git(&["cherry-pick", short])
        .expect("short SHA cherry-pick should succeed");

    let start = repo_rewrite_events(&repo)
        .into_iter()
        .find_map(|event| match event {
            RewriteLogEvent::CherryPickStart { cherry_pick_start } => Some(cherry_pick_start),
            _ => None,
        })
        .expect("cherry-pick start event should exist");

    assert_eq!(
        start.source_commits,
        vec![expected],
        "single commit cherry-pick should resolve short SHA the same way as git rev-parse"
    );
}

#[test]
fn git2_migration_aux_comprehensive_cherry_pick_short_signoff_preserves_next_commit_argument() {
    let repo = TestRepo::new();
    write_file(&repo, "signoff-base.txt", "base\n");
    repo.stage_all_and_commit("base").unwrap();
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "signoff.txt", "feature\n");
    let commit = repo
        .stage_all_and_commit("feature with signoff")
        .unwrap()
        .commit_sha;
    let expected = git_rev_parse(&repo, &commit);

    repo.git(&["checkout", &trunk]).unwrap();
    repo.git(&["cherry-pick", "-s", &commit])
        .expect("short signoff cherry-pick should succeed");

    let start = repo_rewrite_events(&repo)
        .into_iter()
        .find_map(|event| match event {
            RewriteLogEvent::CherryPickStart { cherry_pick_start } => Some(cherry_pick_start),
            _ => None,
        })
        .expect("cherry-pick start event should exist");

    assert_eq!(
        start.source_commits,
        vec![expected],
        "-s/--signoff must not consume the next positional commit argument"
    );
}

#[test]
fn git2_migration_aux_comprehensive_rebase_skip_middle_commit_matches_subject_alignment_in_complete_event()
 {
    let repo = TestRepo::new();
    write_file(&repo, "base.txt", "base\n");
    repo.stage_all_and_commit("base").unwrap();
    let trunk = default_branch(&repo);

    repo.git(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "alpha.txt", "alpha\n");
    repo.stage_all_and_commit("subject alpha").unwrap();
    write_file(&repo, "beta.txt", "beta\n");
    let beta = repo
        .stage_all_and_commit("subject beta")
        .unwrap()
        .commit_sha;
    write_file(&repo, "gamma.txt", "gamma\n");
    let gamma = repo
        .stage_all_and_commit("subject gamma")
        .unwrap()
        .commit_sha;

    repo.git(&["checkout", &trunk]).unwrap();
    write_file(&repo, "beta.txt", "beta\n");
    repo.stage_all_and_commit("pre-apply beta on main").unwrap();

    repo.git(&["checkout", "feature"]).unwrap();
    repo.git(&["rebase", &trunk])
        .expect("rebase should succeed with skipped middle commit");

    let complete = repo_rewrite_events(&repo)
        .into_iter()
        .find_map(|event| match event {
            RewriteLogEvent::RebaseComplete { rebase_complete } => Some(rebase_complete),
            _ => None,
        })
        .expect("rebase complete event should exist");

    let original_subjects: Vec<String> = complete
        .original_commits
        .iter()
        .map(|sha| commit_subject(&repo, sha))
        .collect();
    let new_subjects: Vec<String> = complete
        .new_commits
        .iter()
        .map(|sha| commit_subject(&repo, sha))
        .collect();

    assert!(
        original_subjects.contains(&commit_subject(&repo, &beta)),
        "precondition: skipped source commit should be present in original mapping input"
    );
    assert!(
        original_subjects.contains(&commit_subject(&repo, &gamma)),
        "precondition: later source commit should be present in original mapping input"
    );
    assert_eq!(
        new_subjects,
        vec!["subject alpha".to_string(), "subject gamma".to_string()],
        "rebase completion should preserve the actually rewritten first-parent subjects"
    );
    assert!(
        !new_subjects.contains(&"subject beta".to_string()),
        "skipped middle commit should not appear in rewritten commit subjects"
    );
}

#[test]
fn git2_migration_aux_comprehensive_search_by_commit_range_is_empty_for_empty_range() {
    let repo = TestRepo::new();
    let head = create_ai_commit(
        &repo,
        "search-empty.ts",
        "const a = 1;\n",
        "const a = 1;\nconst b = 2;\n",
    );
    let repository = open_repo(&repo);

    let result = search_by_commit_range(&repository, &head, &head)
        .expect("empty range search should succeed");

    assert!(
        result.is_empty(),
        "same start/end should not search any commits"
    );
}

#[test]
fn git2_migration_aux_comprehensive_search_by_commit_range_returns_single_commit_results() {
    let repo = TestRepo::new();
    let commit = create_ai_commit(
        &repo,
        "search-single.ts",
        "const a = 1;\n",
        "const a = 1;\nconst b = 2;\n",
    );
    let parent = git_rev_parse(&repo, "HEAD^");
    let repository = open_repo(&repo);

    let result = search_by_commit_range(&repository, &parent, &commit)
        .expect("single commit range should succeed");

    assert!(
        !result.is_empty(),
        "single commit range should surface AI prompt metadata"
    );
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
fn git2_migration_aux_comprehensive_blame_abbrev_hashes_match_git_for_default_and_explicit_widths()
{
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
            None => repo
                .git(&["blame", "blame.txt"])
                .expect("git blame should succeed"),
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
        assert_eq!(
            git_ai_hashes, git_hashes,
            "abbreviated blame SHAs should match git for width {extra:?}"
        );
    }
}

#[test]
fn git2_migration_aux_comprehensive_ref_exists_tracks_existing_and_missing_refs() {
    let repo = TestRepo::new();
    write_file(&repo, "refs.txt", "content\n");
    repo.stage_all_and_commit("refs").unwrap();
    let repository = open_repo(&repo);
    let branch_name = repo.current_branch();

    assert!(
        ref_exists(&repository, "HEAD"),
        "HEAD should always resolve"
    );
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

    copy_ref(&repository, "HEAD", "refs/notes/ai-backup")
        .expect("copy_ref should create destination");

    assert!(ref_exists(&repository, "refs/notes/ai-backup"));
    assert_eq!(
        git_rev_parse(&repo, "HEAD"),
        git_rev_parse(&repo, "refs/notes/ai-backup")
    );
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

    copy_ref(&repository, "HEAD", "refs/notes/overwrite-dest")
        .expect("copy_ref should overwrite existing destination");

    assert_eq!(git_rev_parse(&repo, "refs/notes/overwrite-dest"), second);
}

#[test]
fn git2_migration_aux_comprehensive_copy_ref_errors_for_missing_source_without_creating_destination()
 {
    let repo = TestRepo::new();
    write_file(&repo, "copy-missing.txt", "content\n");
    repo.stage_all_and_commit("copy missing").unwrap();
    let repository = open_repo(&repo);

    let result = copy_ref(
        &repository,
        "refs/heads/does-not-exist",
        "refs/notes/missing-copy",
    );

    assert!(result.is_err(), "missing source ref should fail copy_ref");
    assert!(
        !ref_exists(&repository, "refs/notes/missing-copy"),
        "failed copy_ref should not create the destination ref"
    );
}
