# Duplicate Parent AI Note Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent git notes from attributing a human-copied duplicate of a parent AI line to AI while keeping stats behavior unchanged and correct.

**Architecture:** Extract the duplicate-parent-AI line detection currently embedded in stats into a shared authorship helper. Use that helper in `VirtualAttributions::to_authorship_log_and_initial_working_log` before writing AI attestation entries, so note generation filters the same duplicate parent AI lines that stats already excludes from accepted-line counts.

**Tech Stack:** Rust 2024, existing git-ai integration test framework, `AuthorshipLog` serialization, Git notes under `refs/notes/ai`, existing `task test TEST_FILTER=...` test runner.

---

## File Structure

- Create: `src/authorship/duplicate_parent_ai.rs`
  - Shared duplicate parent AI line detection.
  - Owns cached current/parent file line reads and parent prompt line text collection.
  - Provides `DuplicateParentAiContext::is_duplicate_parent_ai_line(...)`.
- Modify: `src/authorship/mod.rs`
  - Expose the new internal module.
- Modify: `src/authorship/stats.rs`
  - Remove the local `ParentAiDuplicateContext` implementation.
  - Reuse `duplicate_parent_ai::DuplicateParentAiContext` in `accepted_lines_from_attestations_with_duplicate_filter(...)`.
- Modify: `src/authorship/virtual_attribution.rs`
  - Build a duplicate filter context in `to_authorship_log_and_initial_working_log(...)`.
  - Filter candidate AI committed lines before creating `AttestationEntry` values.
- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`
  - Strengthen the existing copy-after-AI-commit test to assert the second commit note has no AI attestation for the duplicate copied line.
  - Keep the existing blame assertion that final file line 13 is human.

---

### Task 1: Add Regression Test for Note-Level Duplicate Filtering

**Files:**
- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`

- [ ] **Step 1: Add imports for note parsing**

At the top of `tests/integration/formatting_non_substantial_ai_attribution.rs`, add `AuthorshipLog` to the existing imports:

```rust
use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use git_ai::authorship::authorship_log_serialization::AuthorshipLog;
use git_ai::authorship::working_log::CheckpointKind;
use git_ai::git::find_repository_in_path;
```

- [ ] **Step 2: Replace the existing simple duplicate-copy test with note assertions**

Replace `test_uncheckpointed_human_copy_after_ai_commit_stays_human` with this version:

```rust
#[test]
fn test_uncheckpointed_human_copy_after_ai_commit_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("copy_after_commit.rs");

    std::fs::write(&file_path, "let generated = compute();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "copy_after_commit.rs"])
        .unwrap();
    repo.stage_all_and_commit("Commit AI line").unwrap();

    std::fs::write(
        &file_path,
        "let generated = compute();\nlet generated = compute();\n",
    )
    .unwrap();
    let human_copy_commit = repo
        .stage_all_and_commit("Human copies committed AI line without checkpoint")
        .unwrap();

    let mut file = repo.filename("copy_after_commit.rs");
    file.assert_lines_and_blame(crate::lines![
        "let generated = compute();".ai(),
        "let generated = compute();".human(),
    ]);

    assert!(
        human_copy_commit.authorship_log.attestations.is_empty(),
        "human duplicate copy should not create an AI note attestation: {:#?}",
        human_copy_commit.authorship_log
    );
}
```

- [ ] **Step 3: Add exact selection_sort regression from the provided artifact**

Add this new test below `test_uncheckpointed_human_copy_after_ai_commit_stays_human`:

```rust
#[test]
fn test_human_duplicate_copy_of_parent_ai_line_does_not_write_ai_note() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("selection_sort.py");

    let ai_content = "def selection_sort(arr):\n    for i in range(len(arr) - 1):\n        min_idx = i\n        for j in range(i + 1, len(arr)):\n            if arr[j] < arr[min_idx]:\n                min_idx = j\n        arr[i], arr[min_idx] = arr[min_idx], arr[i]\n    return arr\n\n\nif __name__ == \"__main__\":\n    data = [64, 34, 25, 12, 22, 11, 90]\n    print(\"Before:\", data)\n    print(\"After: \", selection_sort(data.copy()))\n";
    std::fs::write(&file_path, ai_content).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "selection_sort.py"])
        .unwrap();
    let ai_commit = repo.stage_all_and_commit("Commit AI selection sort").unwrap();
    assert_eq!(
        ai_commit.authorship_log.attestations.len(),
        1,
        "precondition: AI commit should have one file attestation"
    );

    let human_copy_content = "def selection_sort(arr):\n    for i in range(len(arr) - 1):\n        min_idx = i\n        for j in range(i + 1, len(arr)):\n            if arr[j] < arr[min_idx]:\n                min_idx = j\n        arr[i], arr[min_idx] = arr[min_idx], arr[i]\n    return arr\n\n\nif __name__ == \"__main__\":\n    data = [64, 34, 25, 12, 22, 11, 90]\n    data = [64, 34, 25, 12, 22, 11, 90]\n    print(\"Before:\", data)\n    print(\"After: \", selection_sort(data.copy()))\n";
    std::fs::write(&file_path, human_copy_content).unwrap();
    let human_copy_commit = repo
        .stage_all_and_commit("Human copies AI data line")
        .unwrap();

    let mut file = repo.filename("selection_sort.py");
    file.assert_lines_and_blame(crate::lines![
        "def selection_sort(arr):".ai(),
        "    for i in range(len(arr) - 1):".ai(),
        "        min_idx = i".ai(),
        "        for j in range(i + 1, len(arr)):".ai(),
        "            if arr[j] < arr[min_idx]:".ai(),
        "                min_idx = j".ai(),
        "        arr[i], arr[min_idx] = arr[min_idx], arr[i]".ai(),
        "    return arr".ai(),
        "".ai(),
        "".ai(),
        "if __name__ == \"__main__\":".ai(),
        "    data = [64, 34, 25, 12, 22, 11, 90]".ai(),
        "    data = [64, 34, 25, 12, 22, 11, 90]".human(),
        "    print(\"Before:\", data)".ai(),
        "    print(\"After: \", selection_sort(data.copy()))".ai(),
    ]);

    assert!(
        human_copy_commit.authorship_log.attestations.is_empty(),
        "human duplicate copy should not create an AI note attestation: {:#?}",
        human_copy_commit.authorship_log
    );
}
```

- [ ] **Step 4: Run the focused test and verify it fails before implementation**

Run:

```bash
task test TEST_FILTER=test_human_duplicate_copy_of_parent_ai_line_does_not_write_ai_note
```

Expected before implementation: FAIL because the second commit still writes an AI note attestation for the duplicate parent AI line.

---

### Task 2: Extract Shared Duplicate Parent AI Detection

**Files:**
- Create: `src/authorship/duplicate_parent_ai.rs`
- Modify: `src/authorship/mod.rs`
- Modify: `src/authorship/stats.rs`

- [ ] **Step 1: Create the shared helper module**

Create `src/authorship/duplicate_parent_ai.rs` with this content:

```rust
use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::git::repository::Repository;
use std::collections::{HashMap, HashSet};

pub(crate) struct DuplicateParentAiContext<'a> {
    repo: &'a Repository,
    commit_sha: &'a str,
    parent_sha: Option<&'a str>,
    parent_authorship_log: Option<&'a AuthorshipLog>,
    current_file_lines: HashMap<String, Option<Vec<String>>>,
    parent_file_lines: HashMap<String, Option<Vec<String>>>,
    parent_prompt_lines: HashMap<(String, String), HashSet<String>>,
}

impl<'a> DuplicateParentAiContext<'a> {
    pub(crate) fn new(
        repo: &'a Repository,
        commit_sha: &'a str,
        parent_sha: Option<&'a str>,
        parent_authorship_log: Option<&'a AuthorshipLog>,
    ) -> Self {
        Self {
            repo,
            commit_sha,
            parent_sha,
            parent_authorship_log,
            current_file_lines: HashMap::new(),
            parent_file_lines: HashMap::new(),
            parent_prompt_lines: HashMap::new(),
        }
    }

    pub(crate) fn is_duplicate_parent_ai_line(
        &mut self,
        file_path: &str,
        current_line: u32,
        prompt_hash: &str,
    ) -> bool {
        let Some(line_text) = self.current_line_text(file_path, current_line) else {
            return false;
        };
        if line_text.is_empty() {
            return false;
        }

        self.parent_prompt_line_texts(file_path, prompt_hash)
            .contains(&line_text)
    }

    fn current_line_text(&mut self, file_path: &str, line: u32) -> Option<String> {
        let commit_sha = self.commit_sha;
        let repo = self.repo;
        let lines = self
            .current_file_lines
            .entry(file_path.to_string())
            .or_insert_with(|| read_normalized_lines_at_commit(repo, commit_sha, file_path));
        line.checked_sub(1)
            .and_then(|idx| lines.as_ref()?.get(idx as usize).cloned())
    }

    fn parent_prompt_line_texts(&mut self, file_path: &str, prompt_hash: &str) -> &HashSet<String> {
        let key = (file_path.to_string(), prompt_hash.to_string());
        if !self.parent_prompt_lines.contains_key(&key) {
            let lines = self.collect_parent_prompt_line_texts(file_path, prompt_hash);
            self.parent_prompt_lines.insert(key.clone(), lines);
        }
        self.parent_prompt_lines
            .get(&key)
            .expect("parent prompt line cache should contain key")
    }

    fn collect_parent_prompt_line_texts(
        &mut self,
        file_path: &str,
        prompt_hash: &str,
    ) -> HashSet<String> {
        let Some(parent_sha) = self.parent_sha else {
            return HashSet::new();
        };
        let Some(parent_log) = self.parent_authorship_log else {
            return HashSet::new();
        };

        let repo = self.repo;
        let parent_lines = self
            .parent_file_lines
            .entry(file_path.to_string())
            .or_insert_with(|| read_normalized_lines_at_commit(repo, parent_sha, file_path));
        let Some(parent_lines) = parent_lines.as_ref() else {
            return HashSet::new();
        };

        let mut texts = HashSet::new();
        for file_attestation in &parent_log.attestations {
            if file_attestation.file_path != file_path {
                continue;
            }
            for entry in &file_attestation.entries {
                if entry.hash != prompt_hash {
                    continue;
                }
                for range in &entry.line_ranges {
                    for line in range.expand() {
                        let Some(text) = line
                            .checked_sub(1)
                            .and_then(|idx| parent_lines.get(idx as usize))
                        else {
                            continue;
                        };
                        if !text.is_empty() {
                            texts.insert(text.clone());
                        }
                    }
                }
            }
        }
        texts
    }
}

fn read_normalized_lines_at_commit(
    repo: &Repository,
    commit_sha: &str,
    file_path: &str,
) -> Option<Vec<String>> {
    let bytes = repo.get_file_content(file_path, commit_sha).ok()?;
    let content = String::from_utf8_lossy(&bytes);
    Some(content.lines().map(normalize_line_for_copy_match).collect())
}

fn normalize_line_for_copy_match(line: &str) -> String {
    line.trim().to_string()
}
```

- [ ] **Step 2: Expose the module**

In `src/authorship/mod.rs`, add:

```rust
pub(crate) mod duplicate_parent_ai;
```

- [ ] **Step 3: Reuse the helper in stats**

In `src/authorship/stats.rs`, add this import near the top:

```rust
use crate::authorship::duplicate_parent_ai::DuplicateParentAiContext;
```

Change the local variable creation in `stats_for_commit_stats(...)` from:

```rust
let mut duplicate_context = ParentAiDuplicateContext::new(
    repo,
    commit_sha,
    parent_commit_sha.as_deref(),
    parent_authorship_log.as_ref(),
);
```

to:

```rust
let mut duplicate_context = DuplicateParentAiContext::new(
    repo,
    commit_sha,
    parent_commit_sha.as_deref(),
    parent_authorship_log.as_ref(),
);
```

Change the helper function signature from:

```rust
mut duplicate_context: Option<&mut ParentAiDuplicateContext<'_>>,
```

to:

```rust
mut duplicate_context: Option<&mut DuplicateParentAiContext<'_>>,
```

Delete the local `ParentAiDuplicateContext` struct, its `impl`, `read_normalized_lines_at_commit`, and `normalize_line_for_copy_match` from `src/authorship/stats.rs`.

- [ ] **Step 4: Run stats-adjacent tests to verify extraction did not change behavior**

Run:

```bash
task test TEST_FILTER=stats
```

Expected: PASS. If unrelated pre-existing tests fail, capture the failing test names and continue only after confirming the failures are unrelated to this extraction.

---

### Task 3: Filter Duplicate Parent AI Lines During Note Generation

**Files:**
- Modify: `src/authorship/virtual_attribution.rs`

- [ ] **Step 1: Import shared helper and parent authorship reader**

At the top of `src/authorship/virtual_attribution.rs`, add:

```rust
use crate::authorship::authorship_log_serialization::get_authorship;
use crate::authorship::duplicate_parent_ai::DuplicateParentAiContext;
```

- [ ] **Step 2: Build duplicate context inside note generation**

Inside `VirtualAttributions::to_authorship_log_and_initial_working_log(...)`, after `referenced_prompts` is declared, add:

```rust
let parent_authorship_log = if parent_sha == "initial" {
    None
} else {
    get_authorship(repo, parent_sha)
};
let mut duplicate_parent_ai_context = DuplicateParentAiContext::new(
    repo,
    commit_sha,
    (parent_sha != "initial").then_some(parent_sha),
    parent_authorship_log.as_ref(),
);
```

- [ ] **Step 3: Filter candidate committed AI lines before writing note entries**

In the block that writes `committed_lines_map` to `authorship_log`, replace this section:

```rust
lines.sort();
lines.dedup();

if lines.is_empty() {
    continue;
}
```

with:

```rust
lines.sort();
lines.dedup();

lines.retain(|line| {
    !duplicate_parent_ai_context.is_duplicate_parent_ai_line(
        &nfc_file_path,
        *line,
        &author_id,
    )
});

if lines.is_empty() {
    continue;
}
```

Keep the existing earlier guard unchanged:

```rust
if author_id == CheckpointKind::Human.to_str() {
    continue;
}
```

- [ ] **Step 4: Run the focused regression test**

Run:

```bash
task test TEST_FILTER=test_human_duplicate_copy_of_parent_ai_line_does_not_write_ai_note
```

Expected: PASS.

- [ ] **Step 5: Run the existing simple copy test**

Run:

```bash
task test TEST_FILTER=test_uncheckpointed_human_copy_after_ai_commit_stays_human
```

Expected: PASS.

---

### Task 4: Verify Broader Attribution Behavior

**Files:**
- Test only; no production edits expected.

- [ ] **Step 1: Run formatting attribution tests**

Run:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
```

Expected: PASS.

- [ ] **Step 2: Run stale prompt carry tests**

Run:

```bash
task test TEST_FILTER=stale_prompt_carry
```

Expected: PASS.

- [ ] **Step 3: Run full test suite if focused tests pass**

Run:

```bash
task test
```

Expected: PASS. If the suite fails for unrelated pre-existing reasons, document exact failing tests and confirm the focused regression tests pass.

- [ ] **Step 4: Run LSP diagnostics on changed Rust files**

Run diagnostics for:

```text
src/authorship/duplicate_parent_ai.rs
src/authorship/mod.rs
src/authorship/stats.rs
src/authorship/virtual_attribution.rs
tests/integration/formatting_non_substantial_ai_attribution.rs
```

Expected: no new errors in changed files.

---

### Task 5: Final Review and Commit

**Files:**
- Review all modified files.

- [ ] **Step 1: Inspect git diff**

Run:

```bash
git diff -- src/authorship/duplicate_parent_ai.rs src/authorship/mod.rs src/authorship/stats.rs src/authorship/virtual_attribution.rs tests/integration/formatting_non_substantial_ai_attribution.rs
```

Expected: diff only contains the shared helper extraction, note-generation duplicate filter, and regression tests.

- [ ] **Step 2: Inspect working tree status**

Run:

```bash
git status --short
```

Expected: only intended files from this plan are modified or added. Existing unrelated workspace files such as `tmp/`, `.omo/`, or pre-existing `.bak` changes must not be staged unless explicitly requested.

- [ ] **Step 3: Commit only if the user explicitly requested a commit**

If and only if the user asks for a commit, run:

```bash
git add src/authorship/duplicate_parent_ai.rs src/authorship/mod.rs src/authorship/stats.rs src/authorship/virtual_attribution.rs tests/integration/formatting_non_substantial_ai_attribution.rs
git commit -m "fix: filter duplicate parent AI notes"
```

Expected: commit succeeds and contains only intended files.

---

## Self-Review

- Spec coverage: The plan covers方案 A by sharing duplicate parent AI detection, applying it to note generation, retaining stats behavior, and adding a regression matching the provided artifact.
- Placeholder scan: No placeholder tasks remain; each code-changing step includes exact file paths and code snippets.
- Type consistency: `DuplicateParentAiContext` is used consistently in `stats.rs` and `virtual_attribution.rs`, and method signatures match the extracted stats logic.
