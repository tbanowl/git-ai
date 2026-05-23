use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::git::find_repository_in_path;

#[test]
fn test_ai_reflow_human_single_line_call_is_fully_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("call.rs");

    file.set_contents(crate::lines!["call(foo, bar, baz)"]);
    repo.stage_all_and_commit("Initial compact call").unwrap();

    file.set_contents(crate::lines![
        "call(".ai(),
        "  foo,".ai(),
        "  bar,".ai(),
        "  baz".ai(),
        ")".ai(),
    ]);
    repo.stage_all_and_commit("AI reflows call").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "call(".ai(),
        "  foo,".ai(),
        "  bar,".ai(),
        "  baz".ai(),
        ")".ai(),
    ]);
}

#[test]
fn test_ai_indentation_only_change_on_human_block_attributes_touched_line_to_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("indent.rs");

    file.set_contents(crate::lines!["fn wrapper() {", "do_work();", "}"]);
    repo.stage_all_and_commit("Initial human block").unwrap();

    file.replace_at(1, "    do_work();".ai());
    repo.stage_all_and_commit("AI reindents body line").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "fn wrapper() {".human(),
        "    do_work();".ai(),
        "}".human()
    ]);
}

#[test]
fn test_ai_wraps_mixed_human_ai_human_block_all_reformatted_lines_become_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("wrapped.rs");

    file.set_contents(crate::lines!["if (ready) {", "do_work();".ai(), "}"]);
    repo.stage_all_and_commit("Initial mixed block").unwrap();

    file.set_contents(crate::lines![
        "fn run() {".ai(),
        "    if (ready) {".ai(),
        "        do_work();".ai(),
        "    }".ai(),
        "}".ai(),
    ]);
    repo.stage_all_and_commit("AI wraps and reformats block")
        .unwrap();

    file.assert_lines_and_blame(crate::lines![
        "fn run() {".ai(),
        "    if (ready) {".ai(),
        "        do_work();".ai(),
        "    }".ai(),
        "}".human(),
    ]);
}

#[test]
fn test_ai_non_substantial_reflow_with_blank_lines_attributes_blank_and_reflowed_lines_to_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("main.rs");

    file.set_contents(crate::lines!["fn main(){println!(\"x\");}"]);
    repo.stage_all_and_commit("Initial compact function")
        .unwrap();

    file.set_contents(crate::lines![
        "fn main() {".ai(),
        "    println!(\"x\");".ai(),
        "".ai(),
        "}".ai(),
    ]);
    repo.stage_all_and_commit("AI reformats function").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "fn main() {".ai(),
        "    println!(\"x\");".ai(),
        "".ai(),
        "}".ai(),
    ]);
}

#[test]
fn test_human_on_ai_after_ai_formatting_reclaims_only_human_edited_line() {
    let repo = TestRepo::new();
    let mut file = repo.filename("pipeline.rs");

    file.set_contents(crate::lines!["call(foo, bar, baz)"]);
    repo.stage_all_and_commit("Initial compact call").unwrap();

    file.set_contents(crate::lines![
        "call(".ai(),
        "  foo,".ai(),
        "  bar,".ai(),
        "  baz".ai(),
        ")".ai(),
    ]);
    repo.stage_all_and_commit("AI reflows call").unwrap();

    file.replace_at(2, "  bar_renamed,".human());
    repo.stage_all_and_commit("Human edits one line").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "call(".ai(),
        "  foo,".ai(),
        "  bar_renamed,".human(),
        "  baz".ai(),
        ")".ai(),
    ]);
}

#[test]
fn test_ai_on_human_after_human_edit_reformats_and_takes_line_ownership() {
    let repo = TestRepo::new();
    let mut file = repo.filename("control.rs");

    file.set_contents(crate::lines!["if (enabled) { do_work(); }"]);
    repo.stage_all_and_commit("Initial control flow").unwrap();

    file.replace_at(0, "if (enabled) { run_work(); }".human());
    repo.stage_all_and_commit("Human changes callee").unwrap();

    file.set_contents(crate::lines![
        "if (enabled) {".ai(),
        "    run_work();".ai(),
        "}".ai(),
    ]);
    repo.stage_all_and_commit("AI reformats human change")
        .unwrap();

    file.assert_lines_and_blame(crate::lines![
        "if (enabled) {".ai(),
        "    run_work();".ai(),
        "}".ai(),
    ]);
}

#[test]
fn test_ai_on_ai_second_formatting_pass_keeps_reformatted_region_ai_and_preserves_untouched_human()
{
    let repo = TestRepo::new();
    let mut file = repo.filename("mixed.rs");

    file.set_contents(crate::lines![
        "// header",
        "call(foo,bar,baz)".ai(),
        "// footer"
    ]);
    repo.stage_all_and_commit("Initial mixed ownership")
        .unwrap();

    file.set_contents(crate::lines![
        "// header",
        "call(".ai(),
        "  foo,".ai(),
        "  bar,".ai(),
        "  baz".ai(),
        ")".ai(),
        "// footer",
    ]);
    repo.stage_all_and_commit("AI reflows AI region").unwrap();

    file.assert_lines_and_blame(crate::lines![
        "// header".human(),
        "call(".ai(),
        "  foo,".ai(),
        "  bar,".ai(),
        "  baz".ai(),
        ")".ai(),
        "// footer".human(),
    ]);
}

#[test]
fn test_iterative_human_ai_human_ai_series_assert_each_commit_state() {
    let repo = TestRepo::new();
    let mut file = repo.filename("iterative.py");

    file.set_contents(crate::lines!["items=[1,2,3]", "total=sum(items)"]);
    repo.stage_all_and_commit("Initial script").unwrap();
    file.assert_lines_and_blame(crate::lines![
        "items=[1,2,3]".human(),
        "total=sum(items)".human()
    ]);

    file.replace_at(0, "items = [1, 2, 3]".ai());
    repo.stage_all_and_commit("AI formats list literal")
        .unwrap();
    file.assert_lines_and_blame(crate::lines![
        "items = [1, 2, 3]".ai(),
        "total=sum(items)".human(),
    ]);

    file.replace_at(1, "total = sum(items) + 1".human());
    repo.stage_all_and_commit("Human adjusts total").unwrap();
    file.assert_lines_and_blame(crate::lines![
        "items = [1, 2, 3]".ai(),
        "total = sum(items) + 1".human(),
    ]);

    file.set_contents(crate::lines![
        "items = [".ai(),
        "    1,".ai(),
        "    2,".ai(),
        "    3,".ai(),
        "]".ai(),
        "total = sum(items) + 1".human(),
    ]);
    repo.stage_all_and_commit("AI reflows list vertically")
        .unwrap();
    file.assert_lines_and_blame(crate::lines![
        "items = [".ai(),
        "    1,".ai(),
        "    2,".ai(),
        "    3,".ai(),
        "]".ai(),
        "total = sum(items) + 1".human(),
    ]);
}

#[test]
fn test_multi_file_ai_formatting_commit_tracks_exact_line_blame_in_each_file() {
    let repo = TestRepo::new();
    let mut file_a = repo.filename("a.py");
    let mut file_b = repo.filename("b.toml");

    file_a.set_contents(crate::lines!["result=compute(x,y)"]);
    file_b.set_contents(crate::lines!["[server]", "port=8080"]);
    repo.stage_all_and_commit("Initial multi-file content")
        .unwrap();

    file_a.set_contents(crate::lines![
        "result = compute(".ai(),
        "    x,".ai(),
        "    y,".ai(),
        ")".ai(),
    ]);
    file_b.set_contents(crate::lines!["[server]", "port = 8080".ai()]);
    repo.stage_all_and_commit("AI reformats both files")
        .unwrap();

    file_a.assert_lines_and_blame(crate::lines![
        "result = compute(".ai(),
        "    x,".ai(),
        "    y,".ai(),
        ")".ai(),
    ]);
    file_b.assert_lines_and_blame(crate::lines!["[server]".human(), "port = 8080".ai()]);
}

#[test]
fn test_complex_sectioned_file_ai_formats_only_selected_sections() {
    let repo = TestRepo::new();
    let mut file = repo.filename("settings.ini");

    file.set_contents(crate::lines![
        "[A]", "alpha=1", "", "[B]", "beta=2", "", "[C]", "gamma=3",
    ]);
    repo.stage_all_and_commit("Initial sectioned config")
        .unwrap();

    file.replace_at(1, "alpha = 1".ai());
    file.replace_at(4, "beta = 2".ai());
    repo.stage_all_and_commit("AI formats selected sections")
        .unwrap();

    file.assert_lines_and_blame(crate::lines![
        "[A]".human(),
        "alpha = 1".ai(),
        "".human(),
        "[B]".human(),
        "beta = 2".ai(),
        "".human(),
        "[C]".human(),
        "gamma=3".human(),
    ]);
}

#[test]
fn test_ai_rewrites_markdown_table_byte_identical_separator_attributed_to_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("table.md");

    // Human creates a markdown table
    file.set_contents(crate::lines![
        "# Data",
        "",
        "| Name | Value |",
        "| --- | --- |",
        "| alpha | 1 |",
        "| beta | 2 |",
    ]);
    repo.stage_all_and_commit("Initial table").unwrap();

    // AI rewrites the table with different data but byte-identical separator
    file.set_contents(crate::lines![
        "# Data",
        "",
        "| Name | Score |".ai(),
        "| --- | --- |".ai(),
        "| gamma | 100 |".ai(),
        "| delta | 200 |".ai(),
    ]);
    repo.stage_all_and_commit("AI rewrites table").unwrap();

    // Changed lines should be AI; byte-identical separator stays human (git didn't see it change)
    file.assert_lines_and_blame(crate::lines![
        "# Data".human(),
        "".human(),
        "| Name | Score |".ai(),
        "| --- | --- |".human(),
        "| gamma | 100 |".ai(),
        "| delta | 200 |".ai(),
    ]);
}

#[test]
fn test_ai_rewrites_table_reformatted_lines_all_attributed_to_ai() {
    let repo = TestRepo::new();
    let mut file = repo.filename("table2.md");

    // Human creates a markdown table with tight formatting
    file.set_contents(crate::lines![
        "# Results",
        "",
        "|Name|Value|",
        "|---|---|",
        "|alpha|1|",
        "|beta|2|",
    ]);
    repo.stage_all_and_commit("Initial tight table").unwrap();

    // AI reformats and rewrites the table with different data + spacing
    file.set_contents(crate::lines![
        "# Results",
        "",
        "| Name  | Score |".ai(),
        "| ----- | ----- |".ai(),
        "| gamma | 100   |".ai(),
        "| delta | 200   |".ai(),
    ]);
    repo.stage_all_and_commit("AI reformats table").unwrap();

    // All reformatted lines should be AI
    file.assert_lines_and_blame(crate::lines![
        "# Results".human(),
        "".human(),
        "| Name  | Score |".ai(),
        "| ----- | ----- |".ai(),
        "| gamma | 100   |".ai(),
        "| delta | 200   |".ai(),
    ]);
}

#[test]
fn test_ai_rewrite_with_byte_identical_line_in_gap() {
    let repo = TestRepo::new();
    let mut file = repo.filename("config.yaml");

    // Human creates a config with a separator
    file.set_contents(crate::lines!["key1: alpha", "---", "key2: beta",]);
    repo.stage_all_and_commit("Initial config").unwrap();

    // AI rewrites the values and the separator (even though it's byte-identical)
    file.set_contents(crate::lines![
        "key1: gamma".ai(),
        "---".ai(),
        "key2: delta".ai(),
    ]);
    repo.stage_all_and_commit("AI updates config values")
        .unwrap();

    // Changed lines are AI; byte-identical separator stays human (git didn't see it change)
    file.assert_lines_and_blame(crate::lines![
        "key1: gamma".ai(),
        "---".human(),
        "key2: delta".ai(),
    ]);
}

#[test]
fn test_ai_edits_around_large_human_section_preserves_human_attribution() {
    let repo = TestRepo::new();
    let mut file = repo.filename("mixed.py");

    // Human creates a file with multiple sections
    file.set_contents(crate::lines![
        "# header",
        "line1 = 1",
        "line2 = 2",
        "line3 = 3",
        "line4 = 4",
        "line5 = 5",
        "# footer",
    ]);
    repo.stage_all_and_commit("Initial file").unwrap();

    // AI changes just the header and footer, leaving 5 human lines in between
    file.set_contents(crate::lines![
        "# new header".ai(),
        "line1 = 1",
        "line2 = 2",
        "line3 = 3",
        "line4 = 4",
        "line5 = 5",
        "# new footer".ai(),
    ]);
    repo.stage_all_and_commit("AI updates header and footer")
        .unwrap();

    // Human lines between AI edits should stay human
    file.assert_lines_and_blame(crate::lines![
        "# new header".ai(),
        "line1 = 1".human(),
        "line2 = 2".human(),
        "line3 = 3".human(),
        "line4 = 4".human(),
        "line5 = 5".human(),
        "# new footer".ai(),
    ]);
}

#[test]
fn test_human_trailing_space_on_uncommitted_ai_line_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_uncommitted.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_uncommitted.rs"])
        .unwrap();

    std::fs::write(&file_path, "let value = compute();   \n").unwrap();
    repo.git_ai(&["checkpoint", "--", "edge_uncommitted.rs"])
        .unwrap();

    repo.stage_all_and_commit("Commit AI line with human trailing whitespace")
        .unwrap();

    let mut file = repo.filename("edge_uncommitted.rs");
    file.assert_lines_and_blame(crate::lines!["let value = compute();   ".ai()]);
}

#[test]
fn test_human_edge_spaces_on_committed_ai_line_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_committed.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_committed.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "\tlet value = compute();   \n").unwrap();
    repo.git_ai(&["checkpoint", "--", "edge_committed.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human adds edge whitespace")
        .unwrap();

    let mut file = repo.filename("edge_committed.rs");
    file.assert_lines_and_blame(crate::lines!["\tlet value = compute();   ".ai()]);
}

#[test]
fn test_human_edge_spaces_in_mixed_hunk_keep_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_mixed_hunk.rs");

    std::fs::write(
        &file_path,
        "let first = compute();\nlet second = compute();\nlet third = compute();\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_mixed_hunk.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI lines").unwrap();

    std::fs::write(
        &file_path,
        "\tlet first = compute();\nlet second = recompute();\nlet third = compute();   \n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "--", "edge_mixed_hunk.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human changes token and edge whitespace")
        .unwrap();

    let mut file = repo.filename("edge_mixed_hunk.rs");
    file.assert_lines_and_blame(crate::lines![
        "\tlet first = compute();".ai(),
        "let second = recompute();".human(),
        "let third = compute();   ".ai(),
    ]);
}

#[test]
fn test_legacy_human_edge_spaces_on_committed_ai_line_keep_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_legacy_human.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_legacy_human.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "\tlet value = compute();   \n").unwrap();
    repo.git_ai(&["checkpoint", "human", "edge_legacy_human.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human adds edge whitespace")
        .unwrap();

    let mut file = repo.filename("edge_legacy_human.rs");
    file.assert_lines_and_blame(crate::lines!["\tlet value = compute();   ".ai()]);
}

#[test]
fn test_uncheckpointed_human_edge_spaces_on_committed_ai_line_keep_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_uncheckpointed.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_uncheckpointed.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "\tlet value = compute();   \n").unwrap();
    repo.stage_all_and_commit("Human adds edge whitespace without checkpoint")
        .unwrap();

    let mut file = repo.filename("edge_uncheckpointed.rs");
    file.assert_lines_and_blame(crate::lines!["\tlet value = compute();   ".ai()]);
}

#[test]
fn test_uncheckpointed_edge_spaces_commit_writes_ai_note_and_clears_parent_working_log() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_note.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_note.rs"])
        .unwrap();
    let first_commit = repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "\tlet value = compute();   \n").unwrap();
    let second_commit = repo
        .stage_all_and_commit("Human adds edge whitespace without checkpoint")
        .unwrap();

    let prompt_ids: std::collections::HashSet<&String> = second_commit
        .authorship_log
        .metadata
        .prompts
        .keys()
        .collect();
    assert!(
        second_commit
            .authorship_log
            .attestations
            .iter()
            .any(|file| {
                file.file_path == "edge_note.rs"
                    && file.entries.iter().any(|entry| {
                        prompt_ids.contains(&entry.hash)
                            && entry.line_ranges.iter().any(|range| range.contains(1))
                    })
            }),
        "expected whitespace-only change to write an AI attestation for the changed line"
    );

    let git_ai_repo = find_repository_in_path(repo.path().to_str().unwrap()).unwrap();
    assert!(
        !git_ai_repo
            .storage
            .has_working_log(&first_commit.commit_sha),
        "expected parent working log to be cleared after commit"
    );
}

#[test]
fn test_mixed_checkpointed_ai_and_uncheckpointed_edge_spaces_write_both_ai_notes() {
    let repo = TestRepo::new();
    let edge_path = repo.path().join("mixed_edge.rs");
    let ai_path = repo.path().join("mixed_ai.rs");

    std::fs::write(&edge_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "mixed_edge.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit initial AI edge file")
        .unwrap();

    std::fs::write(&edge_path, "\tlet value = compute();   \n").unwrap();
    std::fs::write(&ai_path, "let generated = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "mixed_ai.rs"])
        .unwrap();
    let mixed_commit = repo
        .stage_all_and_commit("Commit checkpointed AI and uncheckpointed edge whitespace")
        .unwrap();

    let prompt_ids: std::collections::HashSet<&String> = mixed_commit
        .authorship_log
        .metadata
        .prompts
        .keys()
        .collect();
    for expected_file in ["mixed_edge.rs", "mixed_ai.rs"] {
        let note = mixed_commit.authorship_log.serialize_to_string().unwrap();
        assert!(
            mixed_commit.authorship_log.attestations.iter().any(|file| {
                file.file_path == expected_file
                    && file.entries.iter().any(|entry| {
                        prompt_ids.contains(&entry.hash)
                            && entry.line_ranges.iter().any(|range| range.contains(1))
                    })
            }),
            "expected {expected_file} to have an AI prompt-backed note entry:\n{note}"
        );
    }
}

#[test]
fn test_uncheckpointed_human_token_change_on_ai_line_reclaims_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("token_uncheckpointed.rs");

    std::fs::write(&file_path, "let x = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "token_uncheckpointed.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.stage_all_and_commit("Human changes token without checkpoint")
        .unwrap();

    let mut file = repo.filename("token_uncheckpointed.rs");
    file.assert_lines_and_blame(crate::lines!["let value = compute();".human()]);
}

#[test]
fn test_human_token_change_on_ai_line_reclaims_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("token_change.rs");

    std::fs::write(&file_path, "let x = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "token_change.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "--", "token_change.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human changes token content")
        .unwrap();

    let mut file = repo.filename("token_change.rs");
    file.assert_lines_and_blame(crate::lines!["let value = compute();".human()]);
}

#[test]
fn test_known_human_aliases_serialize_as_canonical_human_checkpoint() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("known_human_alias.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "known_human_alias.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(&file_path, "let renamed = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "known_human_alias.rs"])
        .unwrap();

    std::fs::write(&file_path, "let renamed_again = compute();\n").unwrap();
    repo.git_ai_with_stdin(
        &[
            "checkpoint",
            "known_human",
            "--hook-input",
            "stdin",
            "known_human_alias.rs",
        ],
        br#"{"edited_filepaths":["known_human_alias.rs"]}"#,
    )
    .unwrap();

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("working log checkpoints should be readable");
    let checkpoints_jsonl =
        std::fs::read_to_string(repo.current_working_logs().dir.join("checkpoints.jsonl"))
            .expect("working log JSONL should be readable");
    for line in checkpoints_jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let checkpoint_json: serde_json::Value =
            serde_json::from_str(line).expect("checkpoint JSONL line should parse");
        assert_ne!(
            checkpoint_json.get("kind").and_then(|kind| kind.as_str()),
            Some("known_human"),
            "compatibility aliases must not be serialized as a checkpoint kind: {checkpoint_json}"
        );
    }
    let human_checkpoints: Vec<_> = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.kind == CheckpointKind::Human)
        .collect();

    assert_eq!(
        human_checkpoints.len(),
        2,
        "both known_human aliases should normalize to canonical human checkpoints: {checkpoints:#?}"
    );
    repo.stage_all_and_commit("Human alias edits AI line")
        .unwrap();

    let mut file = repo.filename("known_human_alias.rs");
    file.assert_lines_and_blame(crate::lines!["let renamed_again = compute();".human()]);
}

crate::reuse_tests_in_worktree!(
    test_ai_reflow_human_single_line_call_is_fully_ai,
    test_ai_indentation_only_change_on_human_block_attributes_touched_line_to_ai,
    test_ai_wraps_mixed_human_ai_human_block_all_reformatted_lines_become_ai,
    test_ai_non_substantial_reflow_with_blank_lines_attributes_blank_and_reflowed_lines_to_ai,
    test_human_on_ai_after_ai_formatting_reclaims_only_human_edited_line,
    test_ai_on_human_after_human_edit_reformats_and_takes_line_ownership,
    test_ai_on_ai_second_formatting_pass_keeps_reformatted_region_ai_and_preserves_untouched_human,
    test_iterative_human_ai_human_ai_series_assert_each_commit_state,
    test_multi_file_ai_formatting_commit_tracks_exact_line_blame_in_each_file,
    test_complex_sectioned_file_ai_formats_only_selected_sections,
    test_ai_rewrites_markdown_table_byte_identical_separator_attributed_to_ai,
    test_ai_rewrites_table_reformatted_lines_all_attributed_to_ai,
    test_ai_rewrite_with_byte_identical_line_in_gap,
    test_ai_edits_around_large_human_section_preserves_human_attribution,
    test_human_trailing_space_on_uncommitted_ai_line_keeps_ai_attribution,
    test_human_edge_spaces_on_committed_ai_line_keeps_ai_attribution,
    test_human_edge_spaces_in_mixed_hunk_keep_ai_attribution,
    test_legacy_human_edge_spaces_on_committed_ai_line_keep_ai_attribution,
    test_uncheckpointed_human_edge_spaces_on_committed_ai_line_keep_ai_attribution,
    test_uncheckpointed_edge_spaces_commit_writes_ai_note_and_clears_parent_working_log,
    test_mixed_checkpointed_ai_and_uncheckpointed_edge_spaces_write_both_ai_notes,
    test_uncheckpointed_human_token_change_on_ai_line_reclaims_attribution,
    test_human_token_change_on_ai_line_reclaims_attribution,
    test_known_human_aliases_serialize_as_canonical_human_checkpoint,
);
