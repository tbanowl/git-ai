# Edge Whitespace AI Attribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve AI line attribution when a human only adds or removes spaces/tabs at the beginning or end of an AI-authored line.

**Architecture:** Add focused regression tests in `tests/integration/formatting_non_substantial_ai_attribution.rs`, then update the attribution diff path in `src/authorship/attribution_tracker.rs` so non-AI checkpoints treat edge-only space/tab edits as non-substantive and inherit the previous line attribution. Keep the rule narrow: trim only `' '` and `'\t'`, not interior whitespace or newline-driven reflows.

**Tech Stack:** Rust 2024, existing `TestRepo` integration harness, `AttributionTracker`, imara-diff wrapper, `task test`.

---

## File Structure

- Modify `tests/integration/formatting_non_substantial_ai_attribution.rs`: add three regression tests and include them in `reuse_tests_in_worktree!`.
- Modify `src/authorship/attribution_tracker.rs`: add an edge-space/tab equivalence helper and use it in human checkpoint attribution transfer without changing AI checkpoint force-split behavior.

## Task 1: Add regression tests

**Files:**
- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`

- [ ] **Step 1: Add tests for the confirmed behavior and negative case**

Append these tests before the existing `crate::reuse_tests_in_worktree!` block:

```rust
#[test]
fn test_human_trailing_space_on_uncommitted_ai_line_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("edge_uncommitted.rs");

    std::fs::write(&file_path, "let value = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "edge_uncommitted.rs"])
        .unwrap();

    std::fs::write(&file_path, "let value = compute();   \n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "edge_uncommitted.rs"])
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
    repo.git_ai(&["checkpoint", "mock_known_human", "edge_committed.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human adds edge whitespace")
        .unwrap();

    let mut file = repo.filename("edge_committed.rs");
    file.assert_lines_and_blame(crate::lines!["\tlet value = compute();   ".ai()]);
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
    repo.git_ai(&["checkpoint", "mock_known_human", "token_change.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human changes token content")
        .unwrap();

    let mut file = repo.filename("token_change.rs");
    file.assert_lines_and_blame(crate::lines!["let value = compute();".human()]);
}
```

- [ ] **Step 2: Add the tests to worktree reuse macro**

Add these names to the `crate::reuse_tests_in_worktree!` invocation:

```rust
    test_human_trailing_space_on_uncommitted_ai_line_keeps_ai_attribution,
    test_human_edge_spaces_on_committed_ai_line_keeps_ai_attribution,
    test_human_token_change_on_ai_line_reclaims_attribution,
```

- [ ] **Step 3: Run targeted tests to verify RED**

Run:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
```

Expected: at least one of the new positive tests fails because the current implementation does not fully preserve AI attribution for the requested edge-whitespace case. If all new tests pass, add a narrower unit test in `src/authorship/attribution_tracker.rs` around the helper introduced in Task 2 before changing production behavior.

## Task 2: Implement edge space/tab inheritance

**Files:**
- Modify: `src/authorship/attribution_tracker.rs`

- [ ] **Step 1: Add a helper that compares lines while ignoring only edge spaces/tabs**

Add this helper near other private helper functions in `src/authorship/attribution_tracker.rs`:

```rust
fn trim_edge_spaces_tabs(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '\t')
}

fn lines_equal_ignoring_edge_spaces_tabs(old_line: &str, new_line: &str) -> bool {
    trim_edge_spaces_tabs(old_line) == trim_edge_spaces_tabs(new_line)
}
```

- [ ] **Step 2: Use the helper only for non-AI checkpoint hunk processing**

In `AttributionTracker::process_changed_hunk`, before calling `append_range_diffs` / `build_token_aligned_diffs` for non-AI checkpoints, detect one-line-to-one-line replacements where old and new lines are equal after trimming only edge spaces/tabs. For such a hunk, emit the old/new range with non-substantive diff behavior so inserted edge whitespace inherits previous attribution instead of current human attribution.

The implementation must preserve these constraints:

```rust
// AI checkpoints keep existing force_split behavior.
if is_ai_checkpoint {
    append_range_diffs(
        &mut computation.diffs,
        old_content,
        new_content,
        (old_start, old_end),
        (new_start, new_end),
        true,
    );
    return Ok(());
}

// Only treat as edge-whitespace-equivalent when exactly one old line maps to one new line
// and trimming only spaces/tabs at the edges makes them identical.
```

- [ ] **Step 3: Keep token changes substantive**

Verify that `"let x = compute();"` vs `"let value = compute();"` continues through the existing substantive-change path and is attributed to the human editor.

- [ ] **Step 4: Run targeted tests to verify GREEN**

Run:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
```

Expected: all tests in `formatting_non_substantial_ai_attribution` pass.

## Task 3: Full verification

**Files:**
- Modified files from Tasks 1-2

- [ ] **Step 1: Run LSP diagnostics**

Run diagnostics on:

```text
src/authorship/attribution_tracker.rs
tests/integration/formatting_non_substantial_ai_attribution.rs
```

Expected: zero errors.

- [ ] **Step 2: Run related attribution tests**

Run:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
task test TEST_FILTER=attribution_tracker
```

Expected: both commands pass.

- [ ] **Step 3: Run formatting and lint verification**

Run:

```bash
task fmt
task lint
```

Expected: both commands exit 0. If `task fmt` modifies files, review the diff and rerun the targeted tests.

## Self-Review

- Spec coverage: Task 1 covers the two requested positive scenarios and a token-change negative case. Task 2 implements the narrow edge-only space/tab rule. Task 3 verifies modified files and related tests.
- Placeholder scan: no TBD/TODO placeholders remain.
- Type consistency: helper names and file paths are consistent across tasks.
