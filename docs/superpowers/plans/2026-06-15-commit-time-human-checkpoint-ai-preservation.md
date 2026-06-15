# Commit-time Human Checkpoint AI Preservation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 防止 commit-time / synthetic `Human` checkpoint 把已有 line-only AI attribution 整文件覆盖成 human，确保新增文件在提交 notes 中保留未被真实 human 重写的 AI 行。

**Architecture:** 修复点放在 checkpoint 生成阶段：`src/commands/checkpoint.rs` 读取 previous checkpoint state 时同时保留 char-level 与 line-level attribution；当 previous state 只有 `line_attributions` 时，用 previous blob 内容转换为 char attribution，再交给 `make_entry_for_file()` 做正常 diff tracking。post-commit notes 逻辑保持不变，只消费 checkpoint 的正确归属结果。

**Tech Stack:** Rust 2024, git-ai integration test framework (`TestRepo`, `ExpectedLineExt`, `lines!`), checkpoint working log JSONL, Git notes `refs/notes/ai`.

---

## Source Spec

Read first:

- `docs/superpowers/specs/2026-06-15-commit-time-human-checkpoint-ai-preservation-design.md`
- `docs/superpowers/specs/2026-06-15-checkpoint-line-attribution-fallback-design.md`
- `tests/integration/simple_additions.rs`
- `src/commands/checkpoint.rs`

## Constraints

- Follow TDD: add failing tests before production code.
- Main implementation target is `src/commands/checkpoint.rs`.
- Do not make `src/authorship/post_commit.rs` ignore all-human Human checkpoints as the primary fix.
- Do not make `src/authorship/virtual_attribution.rs` preserve stale AI attribution by skipping Human checkpoints unless a later test proves a separate consumer-side bug.
- Do not change authorship note schema.
- Do not add old-data repair tooling in this plan.
- Do not commit changes unless the user explicitly requests a commit.

## File Structure

- Modify: `tests/integration/simple_additions.rs`
  - Add the positive regression test for line-only AI attribution surviving a synthetic Human checkpoint before commit.
  - Keep the existing full human rewrite regression test as the reverse safety check.
  - Extend `reuse_tests_in_worktree!` to run the new regression in worktree mode.
- Modify: `src/commands/checkpoint.rs`
  - Ensure `PreviousFileState` carries `line_attributions`.
  - Ensure `build_previous_file_state_maps()` stores `entry.line_attributions` alongside `entry.attributions`.
  - Ensure `get_checkpoint_entry_for_file()` reconstructs char attribution from previous line attribution when previous char attribution is empty.
- No changes expected: `src/authorship/post_commit.rs`, `src/authorship/virtual_attribution.rs`, authorship note schema.

---

### Task 1: Add Failing Positive Regression Test

**Files:**
- Modify: `tests/integration/simple_additions.rs`

- [ ] **Step 1: Insert the failing test before `test_ai_generated_file_then_human_full_rewrite`**

Add this test near the existing legacy line-only tests, before the reverse full-rewrite regression at the end of `tests/integration/simple_additions.rs`:

```rust
#[test]
fn test_commit_time_human_checkpoint_preserves_line_only_ai_for_new_file() {
    use sha2::{Digest, Sha256};

    let repo = TestRepo::new();
    let file_path = repo.path().join("foo.cpp");

    let ai_content = "\
int main() {
    int value = 1;
    value += 2;
    value += 3;
    return value;
}
";
    fs::write(&file_path, ai_content).unwrap();

    let ai_sha = format!(
        "{:x}",
        Sha256::new_with_prefix(ai_content.as_bytes()).finalize()
    );
    let agent_author_id = "3bd30911a58cb074".to_string();

    let working_log = repo.current_working_logs();
    working_log
        .persist_file_version(ai_content)
        .expect("persist line-only AI blob");
    working_log
        .append_checkpoint(&Checkpoint {
            kind: CheckpointKind::AiAgent,
            diff: "line-only-ai-before-commit".to_string(),
            author: "Test User".to_string(),
            entries: vec![WorkingLogEntry::new(
                "foo.cpp".to_string(),
                ai_sha,
                Vec::new(),
                vec![LineAttribution {
                    start_line: 1,
                    end_line: 6,
                    author_id: agent_author_id.clone(),
                    overrode: None,
                }],
            )],
            timestamp: 1000,
            transcript: None,
            agent_id: Some(AgentId {
                tool: "mock_ai".to_string(),
                id: "test_session".to_string(),
                model: "test".to_string(),
            }),
            agent_metadata: None,
            line_stats: CheckpointLineStats {
                additions: 6,
                deletions: 0,
                additions_sloc: 6,
                deletions_sloc: 0,
            },
            api_version: "checkpoint/1.0.0".to_string(),
            git_ai_version: Some("development:test".to_string()),
        })
        .expect("append line-only AI checkpoint");

    let final_content = "\
int main() {
    int value = 1;
    value += 3;
    return value;
}
";
    fs::write(&file_path, final_content).unwrap();

    // Simulates the Human checkpoint appended during the commit-time capture path.
    // The remaining lines are inherited from the previous AI checkpoint; there is no
    // human-authored replacement text in the final file.
    repo.git_ai(&["checkpoint", "--", "foo.cpp"]).unwrap();

    let checkpoints = repo
        .current_working_logs()
        .read_all_checkpoints()
        .expect("read checkpoints");
    let human_entry = checkpoints
        .iter()
        .rev()
        .find_map(|checkpoint| {
            if checkpoint.kind != CheckpointKind::Human {
                return None;
            }
            checkpoint.entries.iter().find(|entry| entry.file == "foo.cpp")
        })
        .expect("human checkpoint entry for foo.cpp");

    assert!(
        human_entry
            .attributions
            .iter()
            .any(|attr| attr.author_id == agent_author_id),
        "commit-time human checkpoint should preserve AI byte attribution from prior line-only checkpoint: {human_entry:#?}"
    );

    repo.stage_all_and_commit("add foo after synthetic human capture")
        .unwrap();

    let mut file = repo.filename("foo.cpp");
    file.assert_lines_and_blame(crate::lines![
        "int main() {".ai(),
        "    int value = 1;".ai(),
        "    value += 3;".ai(),
        "    return value;".ai(),
        "}".ai(),
    ]);
}
```

- [ ] **Step 2: Add the test to the worktree reuse macro**

In the `crate::reuse_tests_in_worktree!` block near the end of `tests/integration/simple_additions.rs`, add the new test before `test_ai_generated_file_then_human_full_rewrite`:

```rust
    test_commit_time_human_checkpoint_preserves_line_only_ai_for_new_file,
```

- [ ] **Step 3: Run the targeted test to verify RED**

Run:

```bash
task test TEST_FILTER=test_commit_time_human_checkpoint_preserves_line_only_ai_for_new_file
```

Expected before the production fix:

```text
FAIL
```

The failure should be one of these, all of which confirm the bug:

- assertion says the Human checkpoint did not preserve `agent_author_id`
- `foo.cpp` is missing from the AI note
- `foo.cpp` committed lines are reported as human instead of AI

If the test passes immediately, stop and inspect whether the current branch already contains the production fix. Do not weaken the test. Instead, preserve this test as coverage and continue with Task 3 verification.

---

### Task 2: Preserve Previous Line Attribution in Checkpoint Generation

**Files:**
- Modify: `src/commands/checkpoint.rs`

- [ ] **Step 1: Confirm `PreviousFileState` stores line attribution**

Ensure the struct has all three fields:

```rust
#[derive(Debug, Clone)]
struct PreviousFileState {
    blob_sha: String,
    attributions: Vec<Attribution>,
    line_attributions: Vec<LineAttribution>,
}
```

- [ ] **Step 2: Confirm previous state map copies line attribution**

In `build_previous_file_state_maps()`, ensure each stored `PreviousFileState` copies both attribution forms:

```rust
previous_file_state_by_file.insert(
    entry.file.clone(),
    PreviousFileState {
        blob_sha: entry.blob_sha.clone(),
        attributions: entry.attributions.clone(),
        line_attributions: entry.line_attributions.clone(),
    },
);
```

Keep the existing `ai_touched_files` behavior that marks files with non-human attribution as AI-touched. If that helper only checks `entry.attributions`, extend it so non-human `entry.line_attributions` also count as prior AI edits:

```rust
let has_ai_attribution = entry
    .attributions
    .iter()
    .any(|attr| attr.author_id != CheckpointKind::Human.to_str())
    || entry
        .line_attributions
        .iter()
        .any(|attr| attr.author_id != CheckpointKind::Human.to_str());
```

- [ ] **Step 3: Reconstruct char attribution from previous line attribution**

In `get_checkpoint_entry_for_file()`, keep this precedence for `from_checkpoint`:

```rust
let from_checkpoint: Option<(String, Vec<Attribution>)> = previous_state
    .as_ref()
    .map(|state| -> Result<(String, Vec<Attribution>), GitAiError> {
        if !state.attributions.is_empty() {
            let previous_content = working_log
                .get_file_version(&state.blob_sha)
                .unwrap_or_default();
            return Ok((previous_content, state.attributions.clone()));
        }

        if state.line_attributions.is_empty() {
            let previous_content = working_log
                .get_file_version(&state.blob_sha)
                .unwrap_or_default();
            return Ok((previous_content, Vec::new()));
        }

        let previous_content = working_log.get_file_version(&state.blob_sha)?;
        let prev_attributions =
            crate::authorship::attribution_tracker::line_attributions_to_attributions(
                &state.line_attributions,
                &previous_content,
                ts.saturating_sub(1),
            );

        Ok((previous_content, prev_attributions))
    })
    .transpose()?;
```

Do not add compatibility fallbacks that mark the whole current file as AI. The conversion must use `previous_content`, not `current_content`.

- [ ] **Step 4: Run diagnostics on changed Rust files**

Run LSP diagnostics on:

```text
src/commands/checkpoint.rs
tests/integration/simple_additions.rs
```

Expected: no new diagnostics in the touched files.

- [ ] **Step 5: Run the targeted test to verify GREEN**

Run:

```bash
task test TEST_FILTER=test_commit_time_human_checkpoint_preserves_line_only_ai_for_new_file
```

Expected:

```text
PASS
```

---

### Task 3: Verify Reverse and Existing Legacy Coverage

**Files:**
- Verify: `tests/integration/simple_additions.rs`
- Verify: `src/commands/checkpoint.rs`

- [ ] **Step 1: Run the reverse human rewrite regression**

Run:

```bash
task test TEST_FILTER=test_ai_generated_file_then_human_full_rewrite
```

Expected:

```text
PASS
```

This proves the fix does not blindly ignore all later Human checkpoints.

- [ ] **Step 2: Run the existing legacy line-only preservation test**

Run:

```bash
task test TEST_FILTER=test_human_checkpoint_preserves_legacy_ai_line_only_attribution
```

Expected:

```text
PASS
```

This proves a Human checkpoint after a legacy line-only AI checkpoint preserves unchanged AI ranges and marks actual human-edited ranges as human.

- [ ] **Step 3: Run the simple additions suite**

Run:

```bash
task test TEST_FILTER=simple_additions
```

Expected:

```text
PASS
```

If snapshots change, inspect them with `cargo insta review`. Accept only snapshot updates that reflect the new correct AI attribution behavior.

---

### Task 4: Manual Validation Against `git-notes-test2`

**Files / Repos:**
- Source repo: `/Users/neptune/deepDark/banz/dk/git-ai-code-metrics/git-ai`
- Validation repo: `/Users/neptune/deepDark/cai/git-notes-test2`

- [ ] **Step 1: Install the debug build using the project-approved command**

Run from `/Users/neptune/deepDark/banz/dk/git-ai-code-metrics/git-ai`:

```bash
task dev
```

Expected:

```text
exit code 0
```

- [ ] **Step 2: Inspect the validation repo state before changing it**

Run from `/Users/neptune/deepDark/cai/git-notes-test2`:

```bash
git status --short
```

Expected: record the output. If the repo is dirty, do not overwrite user changes. Use a fresh copy or ask for guidance before destructive cleanup.

- [ ] **Step 3: Reproduce the old-c3 style flow**

Use `/Users/neptune/deepDark/cai/git-notes-test2` as the reference scenario. The validation must create or replay a commit where:

```text
PPTClient/Lot/LotHoldListDlg.cpp is added or modified in the commit
PPTClient/Lot/LotHoldListDlg.cpp has committed added lines
the pre-commit working log has valid AI line attribution before a commit-time Human checkpoint is appended
```

If using the existing archived data, base the replay on:

```text
.git/ai/working_logs/old-c3cdc8ce9e0bd4d2535ed1861f6941a2d86495f7/checkpoints.jsonl
.git/ai/working_logs/old-c3cdc8ce9e0bd4d2535ed1861f6941a2d86495f7/notes
```

- [ ] **Step 4: Verify the target commit diff includes the C++ file**

Run from `/Users/neptune/deepDark/cai/git-notes-test2` after setting `VALIDATION_COMMIT` to the commit created by the replay in Step 3. If Step 3 created the current `HEAD`, use:

```bash
VALIDATION_COMMIT=$(git rev-parse HEAD)
```

Then run:

```bash
git diff --name-status "$VALIDATION_COMMIT^" "$VALIDATION_COMMIT" -- PPTClient/Lot/LotHoldListDlg.cpp PPTClient/Lot/LotHoldListDlg.h
git diff --numstat "$VALIDATION_COMMIT^" "$VALIDATION_COMMIT" -- PPTClient/Lot/LotHoldListDlg.cpp PPTClient/Lot/LotHoldListDlg.h
```

Expected:

```text
PPTClient/Lot/LotHoldListDlg.cpp appears as A or M
PPTClient/Lot/LotHoldListDlg.cpp has non-zero added lines
```

- [ ] **Step 5: Verify notes contain the C++ file AI attribution**

Run:

```bash
git notes --ref=ai show "$VALIDATION_COMMIT"
```

Expected: output contains `PPTClient/Lot/LotHoldListDlg.cpp` and at least one AI prompt hash from the known scenario:

```text
PPTClient/Lot/LotHoldListDlg.cpp
b1ec13f32c390334
```

or:

```text
PPTClient/Lot/LotHoldListDlg.cpp
749e81da762200d1
```

- [ ] **Step 6: Verify the header file did not regress**

In the same note output, confirm `PPTClient/Lot/LotHoldListDlg.h` still has its AI attribution entries.

Expected:

```text
PPTClient/Lot/LotHoldListDlg.h
b1ec13f32c390334
749e81da762200d1
```

- [ ] **Step 7: If `.cpp` is missing, inspect the latest checkpoint**

Run a focused inspection of the latest `checkpoints.jsonl` in the validation repo. The failure condition to look for is:

```text
latest Human checkpoint entry for PPTClient/Lot/LotHoldListDlg.cpp has only human attributions
latest Human checkpoint entry for PPTClient/Lot/LotHoldListDlg.cpp has no preserved non-human attribution
```

If that condition is present, the fix did not reach the checkpoint-generation path used by the commit-time capture.

---

### Task 5: Final Verification Gates

**Files:**
- Verify: `src/commands/checkpoint.rs`
- Verify: `tests/integration/simple_additions.rs`
- Verify: `docs/superpowers/specs/2026-06-15-commit-time-human-checkpoint-ai-preservation-design.md`
- Verify: `docs/superpowers/plans/2026-06-15-commit-time-human-checkpoint-ai-preservation.md`

- [ ] **Step 1: Run build**

Run:

```bash
task build
```

Expected:

```text
exit code 0
```

- [ ] **Step 2: Run targeted tests one final time**

Run:

```bash
task test TEST_FILTER=test_commit_time_human_checkpoint_preserves_line_only_ai_for_new_file
task test TEST_FILTER=test_ai_generated_file_then_human_full_rewrite
task test TEST_FILTER=test_human_checkpoint_preserves_legacy_ai_line_only_attribution
```

Expected:

```text
all three commands exit 0
```

- [ ] **Step 3: Review git diff**

Run:

```bash
git diff -- tests/integration/simple_additions.rs src/commands/checkpoint.rs docs/superpowers/specs/2026-06-15-commit-time-human-checkpoint-ai-preservation-design.md docs/superpowers/plans/2026-06-15-commit-time-human-checkpoint-ai-preservation.md
```

Expected:

```text
diff only includes the planned test, checkpoint generation fix, and docs/plan updates
```

- [ ] **Step 4: Summarize residual risks**

In the final implementation summary, include:

```text
whether the new test failed before the fix
which targeted tests passed after the fix
whether task build passed
whether git-notes-test2 validation showed LotHoldListDlg.cpp in notes with AI prompt hash
any pre-existing dirty files not touched by this work
```
