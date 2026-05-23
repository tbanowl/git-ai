# AI Attribution Restoration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 当 AI 生成的代码被人类删除后，在同一 working-log 生命周期内由人类恢复 token 等价代码时，恢复后的代码继续标为 AI 归属。

**Architecture:** 在 checkpoint/working-log 层增加 deleted-AI tombstone 状态。`AttributionTracker` 在 human checkpoint 删除 AI 归属文本时产生 tombstone，在后续 human checkpoint 插入 token 等价文本时恢复原 AI author；`WorkingLogEntry` 持久化 tombstone，`checkpoint.rs` 负责把上一个 checkpoint 的 tombstone 传给下一次处理。

**Tech Stack:** Rust 2024、serde JSONL working log、现有 `AttributionTracker` 字符级归属、`tests/integration/simple_additions.rs` 集成测试、`task test`/`task build`/`task lint`/`task fmt`。

---

## 文件结构

- 修改：`src/authorship/attribution_tracker.rs`
  - 新增 `DeletedAiTombstone` 结构体。
  - 新增 tombstone 归一化、提取、匹配 helper。
  - 新增 `update_attributions_for_checkpoint_with_tombstones()`，保持旧 `update_attributions_for_checkpoint()` 兼容。
  - 扩展 `transform_attributions()`，让 human 插入在非 move 情况下先尝试 tombstone 恢复。
- 修改：`src/authorship/working_log.rs`
  - `WorkingLogEntry` 增加 `deleted_ai_tombstones` 字段，使用 serde default 保持旧 working log 可读。
  - `WorkingLogEntry::new()` 默认写入空 tombstone。
  - 新增 `WorkingLogEntry::new_with_tombstones()` 供 checkpoint 处理写入 tombstone。
- 修改：`src/commands/checkpoint.rs`
  - `PreviousFileState` 携带上一个 checkpoint entry 的 tombstone。
  - `build_previous_file_state_maps()` 保留每个文件最新 tombstone。
  - `FileEntryInput` 携带 `previous_tombstones`。
  - `make_entry_for_file()` 调用 tombstone-aware tracker API，并把输出 tombstone 写入 `WorkingLogEntry`。
  - CRLF-only remap 和 human-only fast path 显式保留或清空 tombstone，避免 stale 状态。
- 修改：`tests/integration/simple_additions.rs`
  - 增加 5 个回归测试，覆盖 spec 的所有恢复、格式变化、实质变化、歧义、混合 checkpoint 场景。

---

### Task 1: 写第一个失败的集成测试：人类精确恢复 AI 行

**Files:**
- Modify: `tests/integration/simple_additions.rs`

- [ ] **Step 1: 在 `tests/integration/simple_additions.rs` 的 `test_ai_generated_file_then_human_full_rewrite` 后面添加测试**

```rust
#[test]
fn test_human_restores_deleted_ai_line_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("restore_ai.txt");

    fs::write(
        &file_path,
        "Header\nAI generated line\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "restore_ai.txt"])
        .unwrap();
    repo.stage_all_and_commit("AI adds line").unwrap();

    let mut file = repo.filename("restore_ai.txt");
    file.assert_committed_lines(crate::lines![
        "Header".ai(),
        "AI generated line".ai(),
        "Footer".ai(),
    ]);

    fs::write(&file_path, "Header\nFooter\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "restore_ai.txt"])
        .unwrap();

    fs::write(
        &file_path,
        "Header\nAI generated line\nFooter\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "restore_ai.txt"])
        .unwrap();
    repo.stage_all_and_commit("Human restores AI line").unwrap();

    file.assert_committed_lines(crate::lines![
        "Header".ai(),
        "AI generated line".ai(),
        "Footer".ai(),
    ]);
}
```

- [ ] **Step 2: 运行该测试，确认当前实现失败**

Run:

```bash
task test TEST_FILTER=test_human_restores_deleted_ai_line_keeps_ai_attribution
```

Expected: FAIL。失败点应在 `file.assert_committed_lines(...)`，恢复后的 `AI generated line` 当前会被标为 human 或 unattributed，而不是 `.ai()`。

- [ ] **Step 3: 提交失败测试**

```bash
git add tests/integration/simple_additions.rs
git commit -m "test: cover human restoration of deleted ai line"
```

---

### Task 2: 增加 tombstone 数据结构和 working log 持久化字段

**Files:**
- Modify: `src/authorship/attribution_tracker.rs`
- Modify: `src/authorship/working_log.rs`

- [ ] **Step 1: 在 `src/authorship/attribution_tracker.rs` 的 `MoveMapping` 后添加 `DeletedAiTombstone`**

```rust
/// Deleted AI-attributed content that may be restored by a later human checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeletedAiTombstone {
    /// The AI author that originally owned the deleted content.
    pub author_id: String,
    /// Original attribution timestamp. Restored ranges keep this timestamp so
    /// line-level dominance matches the original AI authorship.
    pub ts: u128,
    /// Raw deleted content for diagnostics and future matching improvements.
    pub content: String,
    /// Whitespace-insensitive token key used for safe restoration matching.
    pub normalized_content: String,
    /// Number of non-whitespace tokens/chars in the normalized key.
    pub token_count: usize,
    /// Whether this tombstone has already restored one insertion.
    #[serde(default)]
    pub consumed: bool,
}
```

- [ ] **Step 2: 修改 `src/authorship/working_log.rs` 的 import**

Replace:

```rust
use crate::authorship::attribution_tracker::{Attribution, LineAttribution};
```

With:

```rust
use crate::authorship::attribution_tracker::{
    Attribution, DeletedAiTombstone, LineAttribution,
};
```

- [ ] **Step 3: 给 `WorkingLogEntry` 添加 tombstone 字段**

Replace the struct at `src/authorship/working_log.rs:12-23` with:

```rust
/// Represents a working log entry for a specific file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingLogEntry {
    /// The file path relative to the repository root
    pub file: String,
    /// SHA256 hash of the file content at this checkpoint
    #[serde(default)]
    pub blob_sha: String,
    #[serde(default)]
    pub attributions: Vec<Attribution>,
    #[serde(default)]
    pub line_attributions: Vec<LineAttribution>,
    /// Deleted AI-attributed snippets still available for restoration matching.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_ai_tombstones: Vec<DeletedAiTombstone>,
}
```

- [ ] **Step 4: 修改 `WorkingLogEntry` impl，保留旧 constructor 并添加 tombstone-aware constructor**

Replace `impl WorkingLogEntry` at `src/authorship/working_log.rs:25-40` with:

```rust
impl WorkingLogEntry {
    /// Create a new working log entry.
    pub fn new(
        file: String,
        blob_sha: String,
        attributions: Vec<Attribution>,
        line_attributions: Vec<LineAttribution>,
    ) -> Self {
        Self::new_with_tombstones(file, blob_sha, attributions, line_attributions, Vec::new())
    }

    /// Create a new working log entry with deleted-AI tombstones.
    pub fn new_with_tombstones(
        file: String,
        blob_sha: String,
        attributions: Vec<Attribution>,
        line_attributions: Vec<LineAttribution>,
        deleted_ai_tombstones: Vec<DeletedAiTombstone>,
    ) -> Self {
        Self {
            file,
            blob_sha,
            attributions,
            line_attributions,
            deleted_ai_tombstones,
        }
    }
}
```

- [ ] **Step 5: 运行类型检查用构建，确认新增字段兼容旧调用点**

Run:

```bash
task build
```

Expected: PASS。如果失败，修复所有编译错误；常见失败是缺少 `DeletedAiTombstone` import。

- [ ] **Step 6: 提交数据结构变更**

```bash
git add src/authorship/attribution_tracker.rs src/authorship/working_log.rs
git commit -m "feat: persist deleted ai tombstones in working logs"
```

---

### Task 3: 在 AttributionTracker 中实现 tombstone 捕获与宽松匹配

**Files:**
- Modify: `src/authorship/attribution_tracker.rs`

- [ ] **Step 1: 在 tracker 单元测试模块中添加 helper 测试，先制造失败**

Append inside `#[cfg(test)] mod tests` in `src/authorship/attribution_tracker.rs`:

```rust
#[test]
fn human_restores_deleted_ai_block_with_indent_change_from_tombstone() {
    let tracker = AttributionTracker::new();
    let before_delete = "fn demo() {\n    ai_call();\n}\n";
    let after_delete = "fn demo() {\n}\n";
    let ai_attrs = vec![Attribution::new(
        0,
        before_delete.len(),
        "ai-author".into(),
        TEST_TS,
    )];

    let (after_delete_attrs, tombstones) = tracker
        .update_attributions_for_checkpoint_with_tombstones(
            before_delete,
            after_delete,
            &ai_attrs,
            &[],
            "human",
            TEST_TS + 1,
            false,
        )
        .unwrap();

    assert!(
        tombstones.iter().any(|tombstone| tombstone.author_id == "ai-author"
            && tombstone.normalized_content.contains("ai_call();")),
        "AI deletion should create a reusable tombstone: {tombstones:?}"
    );

    let restored = "fn demo() {\n        ai_call();   \n}\n";
    let (restored_attrs, restored_tombstones) = tracker
        .update_attributions_for_checkpoint_with_tombstones(
            after_delete,
            restored,
            &after_delete_attrs,
            &tombstones,
            "human",
            TEST_TS + 2,
            false,
        )
        .unwrap();

    assert_non_ws_owned_by(
        &restored_attrs,
        restored,
        "ai-author",
        "restored token-equivalent AI block should stay AI-owned",
    );
    assert!(
        restored_tombstones.iter().all(|tombstone| tombstone.consumed),
        "matched tombstone should be consumed: {restored_tombstones:?}"
    );
}

#[test]
fn human_token_change_does_not_match_deleted_ai_tombstone() {
    let tracker = AttributionTracker::new();
    let before_delete = "let value = ai_value();\n";
    let after_delete = "";
    let ai_attrs = vec![Attribution::new(
        0,
        before_delete.len(),
        "ai-author".into(),
        TEST_TS,
    )];

    let (after_delete_attrs, tombstones) = tracker
        .update_attributions_for_checkpoint_with_tombstones(
            before_delete,
            after_delete,
            &ai_attrs,
            &[],
            "human",
            TEST_TS + 1,
            false,
        )
        .unwrap();

    let changed_restore = "let value = human_value();\n";
    let (restored_attrs, _restored_tombstones) = tracker
        .update_attributions_for_checkpoint_with_tombstones(
            after_delete,
            changed_restore,
            &after_delete_attrs,
            &tombstones,
            "human",
            TEST_TS + 2,
            false,
        )
        .unwrap();

    let changed_pos = changed_restore.find("human_value").unwrap();
    assert_range_owned_by(
        &restored_attrs,
        changed_pos,
        changed_pos + "human_value".len(),
        "human",
    );
}
```

- [ ] **Step 2: 运行新增 tracker 测试，确认 API 不存在导致失败**

Run:

```bash
task test TEST_FILTER=human_restores_deleted_ai_block_with_indent_change_from_tombstone CARGO_TEST_ARGS="--lib"
```

Expected: FAIL with a Rust compile error mentioning `update_attributions_for_checkpoint_with_tombstones` is not found.

- [ ] **Step 3: 在 `AttributionTracker` impl 中添加 tombstone-aware public API**

Replace `update_attributions_for_checkpoint()` at `src/authorship/attribution_tracker.rs:568-615` with these two functions:

```rust
pub fn update_attributions_for_checkpoint(
    &self,
    old_content: &str,
    new_content: &str,
    old_attributions: &[Attribution],
    current_author: &str,
    ts: u128,
    is_ai_checkpoint: bool,
) -> Result<Vec<Attribution>, GitAiError> {
    let (attributions, _) = self.update_attributions_for_checkpoint_with_tombstones(
        old_content,
        new_content,
        old_attributions,
        &[],
        current_author,
        ts,
        is_ai_checkpoint,
    )?;
    Ok(attributions)
}

pub fn update_attributions_for_checkpoint_with_tombstones(
    &self,
    old_content: &str,
    new_content: &str,
    old_attributions: &[Attribution],
    previous_tombstones: &[DeletedAiTombstone],
    current_author: &str,
    ts: u128,
    is_ai_checkpoint: bool,
) -> Result<(Vec<Attribution>, Vec<DeletedAiTombstone>), GitAiError> {
    let sorted_old_storage = (!is_attribution_list_sorted(old_attributions))
        .then(|| sort_attributions_for_transform(old_attributions));
    let old_attributions = sorted_old_storage.as_deref().unwrap_or(old_attributions);

    let diff_result = self.compute_diffs(old_content, new_content, is_ai_checkpoint)?;
    let (deletions, insertions) = self.build_diff_catalog(&diff_result.diffs);

    let move_mappings = if is_ai_checkpoint {
        Vec::new()
    } else if self.should_skip_move_detection(old_content, new_content, &deletions, &insertions) {
        Vec::new()
    } else {
        self.detect_moves(old_content, new_content, &deletions, &insertions)
    };

    let mut tombstones = previous_tombstones.to_vec();
    if !is_ai_checkpoint {
        tombstones.extend(self.deleted_ai_tombstones_from_diff(
            old_content,
            old_attributions,
            &deletions,
            &move_mappings,
        ));
    }

    let mut tombstone_matches = HashMap::new();
    let mut tombstone_consumed = vec![false; tombstones.len()];
    if !is_ai_checkpoint {
        tombstone_matches = self.match_insertions_to_tombstones(
            &insertions,
            &tombstones,
            &mut tombstone_consumed,
        );
    }

    let new_attributions = self.transform_attributions(
        &diff_result.diffs,
        old_attributions,
        current_author,
        &insertions,
        &move_mappings,
        &tombstones,
        &tombstone_matches,
        ts,
        &diff_result.substantive_new_ranges,
        is_ai_checkpoint,
    );

    for (idx, consumed) in tombstone_consumed.into_iter().enumerate() {
        if consumed && let Some(tombstone) = tombstones.get_mut(idx) {
            tombstone.consumed = true;
        }
    }

    Ok((self.merge_attributions(new_attributions), tombstones))
}
```

- [ ] **Step 4: 在 `AttributionTracker` impl 中添加 tombstone helper 函数**

Insert before `should_skip_move_detection()`:

```rust
fn deleted_ai_tombstones_from_diff(
    &self,
    old_content: &str,
    old_attributions: &[Attribution],
    deletions: &[Deletion],
    move_mappings: &[MoveMapping],
) -> Vec<DeletedAiTombstone> {
    let mut moved_deletions = std::collections::HashSet::new();
    for mapping in move_mappings {
        moved_deletions.insert(mapping.deletion_idx);
    }

    let mut tombstones = Vec::new();
    for (deletion_idx, deletion) in deletions.iter().enumerate() {
        if moved_deletions.contains(&deletion_idx) {
            continue;
        }
        if data_is_whitespace(&deletion.bytes) {
            continue;
        }

        for attr in old_attributions {
            if attr.author_id == CheckpointKind::Human.to_str() {
                continue;
            }
            let Some((start, end)) = attr.intersection(deletion.start, deletion.end) else {
                continue;
            };
            if start >= end || end > old_content.len() {
                continue;
            }
            let content = old_content[start..end].to_string();
            let normalized_content = normalize_tombstone_content(&content);
            let token_count = normalized_content.chars().count();
            if token_count < 8 {
                continue;
            }
            tombstones.push(DeletedAiTombstone {
                author_id: attr.author_id.clone(),
                ts: attr.ts,
                content,
                normalized_content,
                token_count,
                consumed: false,
            });
        }
    }

    tombstones
}

fn match_insertions_to_tombstones(
    &self,
    insertions: &[Insertion],
    tombstones: &[DeletedAiTombstone],
    tombstone_consumed: &mut [bool],
) -> HashMap<usize, usize> {
    let mut matches = HashMap::new();
    for (insertion_idx, insertion) in insertions.iter().enumerate() {
        let Ok(inserted_text) = std::str::from_utf8(&insertion.bytes) else {
            continue;
        };
        let inserted_key = normalize_tombstone_content(inserted_text);
        if inserted_key.chars().count() < 8 {
            continue;
        }

        let mut candidates = Vec::new();
        for (tombstone_idx, tombstone) in tombstones.iter().enumerate() {
            if tombstone.consumed || tombstone_consumed[tombstone_idx] {
                continue;
            }
            if tombstone.normalized_content == inserted_key {
                candidates.push(tombstone_idx);
            }
        }

        if candidates.len() == 1 {
            let tombstone_idx = candidates[0];
            tombstone_consumed[tombstone_idx] = true;
            matches.insert(insertion_idx, tombstone_idx);
        }
    }

    matches
}
```

- [ ] **Step 5: 在 impl 外添加归一化 helper**

Insert after the `impl AttributionTracker` block ends or before it starts as a private free function:

```rust
fn normalize_tombstone_content(content: &str) -> String {
    content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 6: 扩展 `transform_attributions()` 签名**

Change the function signature from:

```rust
fn transform_attributions(
    &self,
    diffs: &[ByteDiff],
    old_attributions: &[Attribution],
    current_author: &str,
    insertions: &[Insertion],
    move_mappings: &[MoveMapping],
    ts: u128,
    substantive_new_ranges: &[(usize, usize)],
    is_ai_checkpoint: bool,
) -> Vec<Attribution> {
```

To:

```rust
fn transform_attributions(
    &self,
    diffs: &[ByteDiff],
    old_attributions: &[Attribution],
    current_author: &str,
    insertions: &[Insertion],
    move_mappings: &[MoveMapping],
    tombstones: &[DeletedAiTombstone],
    tombstone_matches: &HashMap<usize, usize>,
    ts: u128,
    substantive_new_ranges: &[(usize, usize)],
    is_ai_checkpoint: bool,
) -> Vec<Attribution> {
```

- [ ] **Step 7: 在 Insert 分支的 move check 后、AI checkpoint fast path 前添加 tombstone 恢复**

Insert immediately before the comment `// Add attribution for this insertion`:

```rust
if let Some(tombstone_idx) = tombstone_matches.get(&insertion_idx)
    && let Some(tombstone) = tombstones.get(*tombstone_idx)
{
    new_attributions.push(Attribution::new(
        new_pos,
        new_pos + len,
        tombstone.author_id.clone(),
        tombstone.ts,
    ));

    new_pos += len;
    insertion_idx += 1;
    prev_whitespace_delete = false;
    continue;
}
```

- [ ] **Step 8: 运行 tracker 测试，确认通过**

Run:

```bash
task test TEST_FILTER=human_restores_deleted_ai_block_with_indent_change_from_tombstone CARGO_TEST_ARGS="--lib"
task test TEST_FILTER=human_token_change_does_not_match_deleted_ai_tombstone CARGO_TEST_ARGS="--lib"
```

Expected: both PASS.

- [ ] **Step 9: 运行构建，确认签名修改完整**

Run:

```bash
task build
```

Expected: PASS。

- [ ] **Step 10: 提交 tracker 实现**

```bash
git add src/authorship/attribution_tracker.rs
git commit -m "feat: restore ai attribution from deleted tombstones"
```

---

### Task 4: 把 tombstone 状态接入 checkpoint working-log 流程

**Files:**
- Modify: `src/commands/checkpoint.rs`

- [ ] **Step 1: 修改 `checkpoint.rs` import，加入 `DeletedAiTombstone`**

Replace:

```rust
use crate::authorship::attribution_tracker::{
    Attribution, AttributionTracker, INITIAL_ATTRIBUTION_TS, LineAttribution,
};
```

With:

```rust
use crate::authorship::attribution_tracker::{
    Attribution, AttributionTracker, DeletedAiTombstone, INITIAL_ATTRIBUTION_TS, LineAttribution,
};
```

- [ ] **Step 2: 扩展 `PreviousFileState`**

Replace `PreviousFileState` with:

```rust
/// Latest checkpoint state needed to process a file in the next checkpoint.
#[derive(Debug, Clone)]
struct PreviousFileState {
    blob_sha: String,
    attributions: Vec<Attribution>,
    deleted_ai_tombstones: Vec<DeletedAiTombstone>,
}
```

- [ ] **Step 3: 在 `build_previous_file_state_maps()` 中保留 tombstone**

Replace the `PreviousFileState { ... }` construction at `checkpoint.rs:1649-1652` with:

```rust
PreviousFileState {
    blob_sha: entry.blob_sha.clone(),
    attributions: entry.attributions.clone(),
    deleted_ai_tombstones: entry.deleted_ai_tombstones.clone(),
}
```

- [ ] **Step 4: 在 `from_checkpoint` 中传递 tombstone**

Replace `checkpoint.rs:1743-1750` with:

```rust
let from_checkpoint = previous_state.as_ref().map(|state| {
    (
        working_log
            .get_file_version(&state.blob_sha)
            .unwrap_or_default(),
        state.attributions.clone(),
        state.deleted_ai_tombstones.clone(),
    )
});
```

- [ ] **Step 5: 修改 previous content 解包逻辑，得到 `previous_tombstones`**

Replace:

```rust
let is_from_checkpoint = from_checkpoint.is_some();
let (previous_content, prev_attributions) = if let Some((content, attrs)) = from_checkpoint {
    // File exists in a previous checkpoint - use that
    (content, attrs)
} else {
```

With:

```rust
let is_from_checkpoint = from_checkpoint.is_some();
let (previous_content, prev_attributions, previous_tombstones) =
    if let Some((content, attrs, tombstones)) = from_checkpoint {
        // File exists in a previous checkpoint - use that.
        (content, attrs, tombstones)
    } else {
```

At the end of the `else` branch, replace:

```rust
(adjusted_previous, prev_attributions)
```

With:

```rust
(adjusted_previous, prev_attributions, Vec::new())
```

- [ ] **Step 6: Preserve tombstones in CRLF-only remap entries**

Replace the `WorkingLogEntry::new(...)` call at `checkpoint.rs:1907-1912` with:

```rust
let entry = WorkingLogEntry::new_with_tombstones(
    file_path,
    file_content_hash,
    remapped_attributions,
    line_attributions,
    previous_tombstones,
);
```

- [ ] **Step 7: Extend `FileEntryInput` with previous tombstones**

Replace `FileEntryInput` with:

```rust
struct FileEntryInput<'a> {
    file_path: &'a str,
    blob_sha: &'a str,
    author_id: &'a str,
    is_ai_checkpoint: bool,
    previous_content: &'a str,
    previous_attributions: &'a [Attribution],
    previous_tombstones: &'a [DeletedAiTombstone],
    content: &'a str,
    ts: u128,
}
```

- [ ] **Step 8: Pass `previous_tombstones` into `make_entry_for_file()`**

In the `FileEntryInput` construction at `checkpoint.rs:1916-1925`, add:

```rust
previous_tombstones: &previous_tombstones,
```

The full construction should be:

```rust
let (entry, stats) = make_entry_for_file(FileEntryInput {
    file_path: &file_path,
    blob_sha: &file_content_hash,
    author_id: author_id.as_ref(),
    is_ai_checkpoint: kind.is_ai(),
    previous_content: &previous_content,
    previous_attributions: &prev_attributions,
    previous_tombstones: &previous_tombstones,
    content: &current_content,
    ts,
})?;
```

- [ ] **Step 9: Destructure and use tombstone-aware tracker API in `make_entry_for_file()`**

In `make_entry_for_file()`, add `previous_tombstones` to the destructuring:

```rust
let FileEntryInput {
    file_path,
    blob_sha,
    author_id,
    is_ai_checkpoint,
    previous_content,
    previous_attributions,
    previous_tombstones,
    content,
    ts,
} = input;
```

Replace the `tracker.update_attributions_for_checkpoint(...)` call with:

```rust
let (new_attributions, deleted_ai_tombstones) = tracker
    .update_attributions_for_checkpoint_with_tombstones(
        previous_content,
        content,
        &filled_in_prev_attributions,
        previous_tombstones,
        author_id,
        ts,
        is_ai_checkpoint,
    )?;
```

Replace the final `WorkingLogEntry::new(...)` call with:

```rust
let entry = WorkingLogEntry::new_with_tombstones(
    file_path.to_string(),
    blob_sha.to_string(),
    new_attributions,
    line_attributions,
    deleted_ai_tombstones,
);
```

- [ ] **Step 10: 运行第一个集成测试，确认通过**

Run:

```bash
task test TEST_FILTER=test_human_restores_deleted_ai_line_keeps_ai_attribution
```

Expected: PASS。

- [ ] **Step 11: 提交 checkpoint 接入**

```bash
git add src/commands/checkpoint.rs
git commit -m "feat: carry deleted ai tombstones across checkpoints"
```

---

### Task 5: 补齐 spec 要求的集成回归测试

**Files:**
- Modify: `tests/integration/simple_additions.rs`

- [ ] **Step 1: 添加多行 block + 缩进/行尾空白恢复测试**

Append after `test_human_restores_deleted_ai_line_keeps_ai_attribution()`:

```rust
#[test]
fn test_human_restores_deleted_ai_block_with_whitespace_changes_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("restore_ai_block.rs");

    let original = "fn demo() {\n    first_ai_call();\n    second_ai_call();\n}\n";
    fs::write(&file_path, original).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "restore_ai_block.rs"])
        .unwrap();
    repo.stage_all_and_commit("AI adds block").unwrap();

    let mut file = repo.filename("restore_ai_block.rs");
    file.assert_committed_lines(crate::lines![
        "fn demo() {".ai(),
        "    first_ai_call();".ai(),
        "    second_ai_call();".ai(),
        "}".ai(),
    ]);

    fs::write(&file_path, "fn demo() {\n}\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "restore_ai_block.rs"])
        .unwrap();

    let restored = "fn demo() {\n        first_ai_call();   \n        second_ai_call();\t\n}\n";
    fs::write(&file_path, restored).unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "restore_ai_block.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human restores AI block with whitespace changes")
        .unwrap();

    file.assert_committed_lines(crate::lines![
        "fn demo() {".ai(),
        "        first_ai_call();   ".ai(),
        "        second_ai_call();\t".ai(),
        "}".ai(),
    ]);
}
```

- [ ] **Step 2: 添加 token 改变不恢复 AI 的测试**

Append:

```rust
#[test]
fn test_human_restores_changed_ai_line_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("changed_restore.rs");

    fs::write(&file_path, "let answer = ai_answer();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "changed_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("AI adds answer").unwrap();

    let mut file = repo.filename("changed_restore.rs");
    file.assert_committed_lines(crate::lines!["let answer = ai_answer();".ai()]);

    fs::write(&file_path, "").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "changed_restore.rs"])
        .unwrap();

    fs::write(&file_path, "let answer = human_answer();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "changed_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human restores changed answer")
        .unwrap();

    file.assert_committed_lines(crate::lines!["let answer = human_answer();".human()]);
}
```

- [ ] **Step 3: 添加歧义相同片段不恢复 AI 的测试**

Append:

```rust
#[test]
fn test_human_restores_ambiguous_deleted_ai_line_stays_human() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("ambiguous_restore.rs");

    fs::write(
        &file_path,
        "repeat_call();\nkeep_human_anchor();\nrepeat_call();\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "ambiguous_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("AI adds repeated calls").unwrap();

    let mut file = repo.filename("ambiguous_restore.rs");
    file.assert_committed_lines(crate::lines![
        "repeat_call();".ai(),
        "keep_human_anchor();".ai(),
        "repeat_call();".ai(),
    ]);

    fs::write(&file_path, "keep_human_anchor();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "ambiguous_restore.rs"])
        .unwrap();

    fs::write(&file_path, "repeat_call();\nkeep_human_anchor();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "ambiguous_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human restores ambiguous repeated call")
        .unwrap();

    file.assert_committed_lines(crate::lines![
        "repeat_call();".human(),
        "keep_human_anchor();".ai(),
    ]);
}
```

- [ ] **Step 4: 添加混合 checkpoint 场景测试**

Append:

```rust
#[test]
fn test_human_restores_ai_line_after_intermediate_ai_checkpoint_keeps_ai_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("mixed_restore.rs");

    fs::write(
        &file_path,
        "start\ndeleted_ai_line();\nkept_ai_line();\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "mixed_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("AI adds initial lines").unwrap();

    let mut file = repo.filename("mixed_restore.rs");
    file.assert_committed_lines(crate::lines![
        "start".ai(),
        "deleted_ai_line();".ai(),
        "kept_ai_line();".ai(),
    ]);

    fs::write(&file_path, "start\nkept_ai_line();\n").unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "mixed_restore.rs"])
        .unwrap();

    fs::write(
        &file_path,
        "start\nkept_ai_line();\nnew_ai_line();\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "mixed_restore.rs"])
        .unwrap();

    fs::write(
        &file_path,
        "start\ndeleted_ai_line();\nkept_ai_line();\nnew_ai_line();\n",
    )
    .unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "mixed_restore.rs"])
        .unwrap();
    repo.stage_all_and_commit("Human restores deleted AI after AI checkpoint")
        .unwrap();

    file.assert_committed_lines(crate::lines![
        "start".ai(),
        "deleted_ai_line();".ai(),
        "kept_ai_line();".ai(),
        "new_ai_line();".ai(),
    ]);
}
```

- [ ] **Step 5: 运行全部新增集成测试**

Run:

```bash
task test TEST_FILTER=test_human_restores_deleted_ai_line_keeps_ai_attribution
task test TEST_FILTER=test_human_restores_deleted_ai_block_with_whitespace_changes_keeps_ai_attribution
task test TEST_FILTER=test_human_restores_changed_ai_line_stays_human
task test TEST_FILTER=test_human_restores_ambiguous_deleted_ai_line_stays_human
task test TEST_FILTER=test_human_restores_ai_line_after_intermediate_ai_checkpoint_keeps_ai_attribution
```

Expected: all PASS。

- [ ] **Step 6: 提交完整回归测试**

```bash
git add tests/integration/simple_additions.rs
git commit -m "test: cover ai attribution restoration edge cases"
```

---

### Task 6: 加强持久化兼容测试

**Files:**
- Modify: `src/git/repo_storage.rs`

- [ ] **Step 1: 在 repo_storage tests 中添加旧 JSONL 缺少 tombstone 字段仍可读取的测试**

Append inside `#[cfg(test)] mod tests` in `src/git/repo_storage.rs`:

```rust
#[test]
fn test_read_checkpoint_without_deleted_ai_tombstones_defaults_empty() {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo_storage = RepoStorage::new(temp_dir.path().to_path_buf());
    repo_storage.ensure_dirs().unwrap();
    let working_log = repo_storage.get_working_log("abc123");
    fs::create_dir_all(&working_log.dir).unwrap();

    let checkpoints_file = working_log.dir.join("checkpoints.jsonl");
    fs::write(
        &checkpoints_file,
        r#"{"kind":"Human","diff":"","author":"human","entries":[{"file":"a.txt","blob_sha":"blob","attributions":[],"line_attributions":[]}],"timestamp":1,"transcript":null,"agent_id":null,"agent_metadata":null,"line_stats":{"additions":0,"deletions":0,"additions_sloc":0,"deletions_sloc":0},"api_version":"checkpoint/1.0.0","git_ai_version":null}
"#,
    )
    .unwrap();

    let checkpoints = working_log.read_all_checkpoints().unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].entries.len(), 1);
    assert!(checkpoints[0].entries[0].deleted_ai_tombstones.is_empty());
}
```

- [ ] **Step 2: Run the compatibility test**

Run:

```bash
task test TEST_FILTER=test_read_checkpoint_without_deleted_ai_tombstones_defaults_empty CARGO_TEST_ARGS="--lib"
```

Expected: PASS。

- [ ] **Step 3: 提交兼容测试**

```bash
git add src/git/repo_storage.rs
git commit -m "test: ensure tombstone working logs are backward compatible"
```

---

### Task 7: 全量验证与收尾

**Files:**
- Verify: `src/authorship/attribution_tracker.rs`
- Verify: `src/authorship/working_log.rs`
- Verify: `src/commands/checkpoint.rs`
- Verify: `src/git/repo_storage.rs`
- Verify: `tests/integration/simple_additions.rs`

- [ ] **Step 1: 运行 LSP diagnostics**

Run diagnostics on these files:

```text
src/authorship/attribution_tracker.rs
src/authorship/working_log.rs
src/commands/checkpoint.rs
src/git/repo_storage.rs
tests/integration/simple_additions.rs
```

Expected: zero errors。Warnings 可以存在，但不能出现新增的 type/import/borrow checker errors。

- [ ] **Step 2: 运行格式化**

Run:

```bash
task fmt
```

Expected: exit code 0。If it changes files, inspect the diff before committing.

- [ ] **Step 3: 运行 lint**

Run:

```bash
task lint
```

Expected: exit code 0。

- [ ] **Step 4: 运行构建**

Run:

```bash
task build
```

Expected: exit code 0。

- [ ] **Step 5: 运行相关测试集合**

Run:

```bash
task test TEST_FILTER=human_restores_deleted_ai_block_with_indent_change_from_tombstone CARGO_TEST_ARGS="--lib"
task test TEST_FILTER=human_token_change_does_not_match_deleted_ai_tombstone CARGO_TEST_ARGS="--lib"
task test TEST_FILTER=test_human_restores_deleted_ai_line_keeps_ai_attribution
task test TEST_FILTER=test_human_restores_deleted_ai_block_with_whitespace_changes_keeps_ai_attribution
task test TEST_FILTER=test_human_restores_changed_ai_line_stays_human
task test TEST_FILTER=test_human_restores_ambiguous_deleted_ai_line_stays_human
task test TEST_FILTER=test_human_restores_ai_line_after_intermediate_ai_checkpoint_keeps_ai_attribution
task test TEST_FILTER=test_read_checkpoint_without_deleted_ai_tombstones_defaults_empty CARGO_TEST_ARGS="--lib"
```

Expected: all PASS。

- [ ] **Step 6: 运行 full test suite**

Run:

```bash
task test
```

Expected: exit code 0。

- [ ] **Step 7: 检查最终 diff**

Run:

```bash
git diff -- src/authorship/attribution_tracker.rs src/authorship/working_log.rs src/commands/checkpoint.rs src/git/repo_storage.rs tests/integration/simple_additions.rs docs/superpowers/specs/2026-05-12-ai-attribution-restoration-design.md docs/superpowers/plans/2026-05-12-ai-attribution-restoration.md
```

Expected: diff 只包含 tombstone 归属恢复、测试和文档/计划相关变更；不包含 unrelated formatting 或 debug prints。

- [ ] **Step 8: 提交验证修正**

If `task fmt` changed files or verification required fixes, commit them:

```bash
git add src/authorship/attribution_tracker.rs src/authorship/working_log.rs src/commands/checkpoint.rs src/git/repo_storage.rs tests/integration/simple_additions.rs docs/superpowers/specs/2026-05-12-ai-attribution-restoration-design.md docs/superpowers/plans/2026-05-12-ai-attribution-restoration.md
git commit -m "chore: verify ai attribution restoration"
```

If there are no changes after verification, skip this commit.

---

## Self-Review

**Spec coverage:**
- 同一 working-log 生命周期恢复 AI 归属：Task 3 tracker 实现，Task 4 checkpoint 状态传递，Task 5 集成测试覆盖。
- 轻微格式差异：Task 3 单元测试和 Task 5 block 测试覆盖缩进、行尾空白、token 等价。
- 歧义时回退 human：Task 3 unique-match 逻辑和 Task 5 ambiguous 测试覆盖。
- 不支持跨 commit 历史追溯：计划没有引入 blame/history index，也没有改 post-commit 历史逻辑。
- AI checkpoint 不消费 tombstone：Task 3 仅在 `!is_ai_checkpoint` 时匹配 tombstone；Task 5 混合场景覆盖 AI checkpoint 之后 tombstone 仍可由 human 恢复。

**Placeholder scan:** 本计划不包含待补充占位项；所有测试名、文件路径、命令和新增类型名均已具体化。

**Type consistency:** 统一使用 `DeletedAiTombstone`、`deleted_ai_tombstones`、`update_attributions_for_checkpoint_with_tombstones()`、`previous_tombstones`；`WorkingLogEntry::new()` 保持旧签名，新增 `new_with_tombstones()` 供 checkpoint 使用。
