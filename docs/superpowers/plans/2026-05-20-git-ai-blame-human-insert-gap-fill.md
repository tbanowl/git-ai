# Git AI Blame Human Insert Gap-Fill Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `git-ai blame` so that a human line inserted into the middle of an AI-generated block, and committed once, stays human instead of being rewritten as AI.

**Architecture:** Keep the existing checkpoint → working log → authorship note → blame pipeline. The change is intentionally conservative: preserve current AI attribution for normal contiguous AI blocks, but tighten the authorship-note gap-fill logic in `src/authorship/virtual_attribution.rs` so it does not fill holes that are more consistent with human insertion or replacement. The regression coverage lives in `tests/integration/simple_additions.rs` and should prove both the original bug and the adjacent scenarios.

**Tech Stack:** Rust 2024, the existing integration test harness (`TestRepo`, `TestFile`, `assert_lines_and_blame`), the current working-log/authorship note pipeline, and `task test` / `task build` / `task lint` / `task fmt`.

---

## File Structure

- Modify: `tests/integration/simple_additions.rs`
  - Keep the current reproducer for the human-insert bug.
  - Add adjacent regression tests for human replacement of an AI line and human insertion at the edge of an AI block.
  - Keep the tests explicit with `fs::write` + `checkpoint` calls so the checkpoint order is controlled.
- Modify: `src/authorship/virtual_attribution.rs`
  - Tighten the committed-hunk gap-fill logic around lines `1436-1488`.
  - Add a small helper if needed to decide whether a gap is safe to fill as AI.
  - Preserve the existing behavior for normal contiguous AI blocks and for blame/authorship note generation outside the gap-fill path.

---

### Task 1: Keep the reproducer as the failing regression test and add adjacent test coverage

**Files:**
- Modify: `tests/integration/simple_additions.rs`

- [ ] **Step 1: Keep the existing failing reproducer focused on the single-commit bug**

Use the current test shape as the canonical bug reproducer:

```rust
#[test]
fn test_human_insert_between_ai_lines_before_first_commit_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("test.txt");

    fs::write(&file_path, "Header\nFooter\n").unwrap();
    repo.stage_all_and_commit("Base commit").unwrap();

    fs::write(
        &file_path,
        "Header\nAI line 1\nAI line 2\nAI line 3\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "test.txt"]).unwrap();

    fs::write(
        &file_path,
        "Header\nAI line 1\nHuman inserted line\nAI line 2\nAI line 3\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "--", "test.txt"]).unwrap();

    repo.stage_all_and_commit("AI block with human insertion").unwrap();

    let mut file = repo.filename("test.txt");
    file.assert_lines_and_blame(crate::lines![
        "Header".human(),
        "AI line 1".ai(),
        "Human inserted line".human(),
        "AI line 2".ai(),
        "AI line 3".ai(),
        "Footer".human(),
    ]);
}
```

- [ ] **Step 2: Run only this test and confirm it fails for the expected reason**

Run:

```bash
task test TEST_FILTER=test_human_insert_between_ai_lines_before_first_commit_stays_human
```

Expected: FAIL. The failure should show `Human inserted line` being blamed as `mock_ai`, which confirms the bug is real and the test is reproducing the correct path.

- [ ] **Step 3: Add the adjacent “human replaces AI line” regression test**

Add a second test in the same file that uses the same pattern but replaces one AI line instead of inserting a new one:

```rust
#[test]
fn test_human_replaces_ai_line_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("test_replace.txt");

    fs::write(&file_path, "Header\nFooter\n").unwrap();
    repo.stage_all_and_commit("Base commit").unwrap();

    fs::write(
        &file_path,
        "Header\nAI line 1\nAI line 2\nAI line 3\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "test_replace.txt"]).unwrap();

    fs::write(
        &file_path,
        "Header\nAI line 1\nHuman replaced line\nAI line 3\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "--", "test_replace.txt"]).unwrap();

    repo.stage_all_and_commit("Human replaces AI line").unwrap();

    let mut file = repo.filename("test_replace.txt");
    file.assert_lines_and_blame(crate::lines![
        "Header".human(),
        "AI line 1".ai(),
        "Human replaced line".human(),
        "AI line 3".ai(),
        "Footer".human(),
    ]);
}
```

- [ ] **Step 4: Add the adjacent “human inserts at AI block edge” regression test**

Add a third test for insertion at the beginning or end of an AI block. Use the same explicit checkpoint flow and assert the inserted edge line remains human:

```rust
#[test]
fn test_human_insert_at_ai_block_edge_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("test_edge.txt");

    fs::write(&file_path, "Header\nFooter\n").unwrap();
    repo.stage_all_and_commit("Base commit").unwrap();

    fs::write(
        &file_path,
        "Header\nAI line 1\nAI line 2\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "test_edge.txt"]).unwrap();

    fs::write(
        &file_path,
        "Header\nHuman edge insert\nAI line 1\nAI line 2\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "--", "test_edge.txt"]).unwrap();

    repo.stage_all_and_commit("Human inserts at AI edge").unwrap();

    let mut file = repo.filename("test_edge.txt");
    file.assert_lines_and_blame(crate::lines![
        "Header".human(),
        "Human edge insert".human(),
        "AI line 1".ai(),
        "AI line 2".ai(),
        "Footer".human(),
    ]);
}
```

- [ ] **Step 5: Run the new test subset and confirm the failure family is captured**

Run:

```bash
task test TEST_FILTER="test_human_insert_between_ai_lines_before_first_commit_stays_human|test_human_replaces_ai_line_stays_human|test_human_insert_at_ai_block_edge_stays_human"
```

Expected: at least the original insert-middle test fails before code changes. The new tests should compile and exercise the same path family, even if they are not all red for the same reason before the code change.

---

### Task 2: Tighten the authorship-note gap-fill logic so human insertions do not become AI

**Files:**
- Modify: `src/authorship/virtual_attribution.rs`

- [ ] **Step 1: Locate the gap-fill block that currently fills missing committed lines from same-AI neighbors**

Focus on the committed-hunk gap-fill path around the existing comment that says it fills gaps when both neighbors have the same AI author. That is the behavior that should be made more conservative.

- [ ] **Step 2: Add a helper that decides whether a gap is safe to fill as AI**

Implement a small, local helper in `virtual_attribution.rs` rather than spreading the rule across the file. The helper should be conservative and should return `false` for ambiguous cases.

Suggested shape:

```rust
fn should_fill_committed_gap_as_ai(
    prev_author: &str,
    next_author: &str,
    gap_line: u32,
    committed_lines_map: &std::collections::HashMap<String, Vec<u32>>,
) -> bool {
    prev_author == next_author && !prev_author.starts_with("h_")
}
```

Then evolve it carefully so that it refuses to fill gaps when the surrounding evidence looks like a human insertion or replacement. Keep the logic local to the gap-fill block.

- [ ] **Step 3: Change the gap-fill loop to use the helper before pushing `(author_id, line)` into `gap_fills`**

The existing loop should keep the same data structures, but only append a gap fill when the helper returns `true`.

The intended behavior after the change:

```rust
if let (Some((_, prev_author)), Some((_, next_author))) = (prev, next)
    && should_fill_committed_gap_as_ai(prev_author, next_author, line, &committed_lines_map)
{
    gap_fills.push((prev_author.to_string(), line));
}
```

- [ ] **Step 4: Make sure human-attributed lines are still not written into AI attestations**

Keep the existing `author_id == CheckpointKind::Human.to_str()` filtering behavior intact. The goal is to stop the AI gap-fill from reintroducing human lines as AI, not to change the broader committed/uncommitted split.

- [ ] **Step 5: Run the targeted regression test and verify the inserted human line now stays human**

Run:

```bash
task test TEST_FILTER=test_human_insert_between_ai_lines_before_first_commit_stays_human
```

Expected: PASS.

- [ ] **Step 6: Run the adjacent regression tests for replacement and edge insertion**

Run:

```bash
task test TEST_FILTER="test_human_insert_between_ai_lines_before_first_commit_stays_human|test_human_replaces_ai_line_stays_human|test_human_insert_at_ai_block_edge_stays_human"
```

Expected: PASS.

- [ ] **Step 7: Run the broader blame and attribution suite to catch regressions**

Run the related tests that already cover AI attribution, mixed edits, and blame output:

```bash
task test TEST_FILTER="test_simple_additions_on_top_of_ai_contributions|test_ai_human_interleaved_line_attribution|test_human_token_change_on_ai_line_reclaims_attribution"
```

Expected: PASS.

---

### Task 3: Verify the whole change set and clean up the regression test shape

**Files:**
- Modify: `tests/integration/simple_additions.rs`
- Modify: `src/authorship/virtual_attribution.rs`

- [ ] **Step 1: Run formatting and lint checks**

Run:

```bash
task fmt
task lint
```

Expected: PASS.

- [ ] **Step 2: Run the full project build**

Run:

```bash
task build
```

Expected: PASS.

- [ ] **Step 3: Run the focused integration suite one more time**

Run:

```bash
task test TEST_FILTER="test_human_insert_between_ai_lines_before_first_commit_stays_human|test_human_replaces_ai_line_stays_human|test_human_insert_at_ai_block_edge_stays_human|test_simple_additions_on_top_of_ai_contributions|test_ai_human_interleaved_line_attribution|test_human_token_change_on_ai_line_reclaims_attribution"
```

Expected: PASS.

- [ ] **Step 4: Remove any debugging-only output and keep the tests focused on behavior**

If any temporary logging or diagnostic code was added during debugging, remove it now so the final test file only asserts behavior.

- [ ] **Step 5: Final review before handoff**

Confirm the final state matches the conservative design:

- the gap-fill rule is stricter for ambiguous human insertions
- the blame output still works the same way
- the new regression test and adjacent tests all pass

---

## Verification Checklist

- [ ] `task test TEST_FILTER=test_human_insert_between_ai_lines_before_first_commit_stays_human`
- [ ] `task test TEST_FILTER="test_human_insert_between_ai_lines_before_first_commit_stays_human|test_human_replaces_ai_line_stays_human|test_human_insert_at_ai_block_edge_stays_human"`
- [ ] `task test TEST_FILTER="test_simple_additions_on_top_of_ai_contributions|test_ai_human_interleaved_line_attribution|test_human_token_change_on_ai_line_reclaims_attribution"`
- [ ] `task fmt`
- [ ] `task lint`
- [ ] `task build`

## Expected Outcome

After this plan is executed, a single commit that mixes AI-generated lines and a human insertion inside the same block should keep the human insertion as human in `git-ai blame`, while the rest of the AI block remains attributed to AI.
