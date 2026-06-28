# 普通 Merge 冲突解决 AI 归因修复设计文档

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复普通 `git merge` 冲突解决后的 AI 归因，使 AI 在冲突解决阶段写出的行能够正确进入 merge commit 的 authorship note。

**Architecture:** 采用 post-commit 窄口 overlay 方案，不新增完整普通 merge rewrite 生命周期。先移植 `fantacy/git-ai` 的 uncovered-line conflict-resolution 合并语义，再在当前项目的 merge commit note 生成路径中只对存在 AI resolution checkpoint 的场景做补充合并。

**Tech Stack:** Rust 2024, Git CLI, git notes `refs/notes/ai`, git-ai working logs, integration tests under `tests/integration/`.

## Global Constraints

- 不迁移 `D:/banz/code/fantacy/git-ai` 的 daemon / trace 架构。
- 不新增完整 `MergeStart / MergeComplete / MergeAbort` rewrite event 生命周期作为第一阶段方案。
- 不默认放开所有 unmerged 文件 checkpoint。
- 不用启发式判断“这段代码像 AI 写的”；仍然只依赖 checkpoint / working log / notes 等明确证据。
- 不影响现有 `merge --squash`、rebase、cherry-pick、stash、commit amend 归因逻辑。
- 第一阶段保持现有约束：AI 解决 merge conflict 后必须先 `git add <resolved-file>`，再 `git-ai checkpoint <agent> <resolved-file>`。

---

## 背景

当前项目：`D:/banz/code/w/dk/git-ai`

参考项目：`D:/banz/code/fantacy/git-ai`

问题场景：

在 demo 仓库 `D:/banz/code/demo/git-notes-test` 中，将 `git-ai-1` 合并到 `git-ai-2` 后，`bubble_sort.py` 发生冲突。冲突由 AI 解决，最终文件中第 15、16 行为：

```python
def bubble_sort(arr, *, inplace=True, reverse=False):
    """对列表进行冒泡排序，并返回排序结果。
```

这两行是冲突解决阶段生成的最终内容，按事实应归属为 AI。但当前 merge commit `b8a060e` 的 `refs/notes/ai` 是空 note，`git-ai blame` 显示这两行归属为 human / merge commit author。

## 目标

修复普通 `git merge` 冲突解决后的 AI 归因，使 AI 在冲突解决阶段写出的行能够正确进入 merge commit 的 authorship note。

具体目标：

- AI 解决冲突并 checkpoint 后，冲突解决行归属为 AI。
- human 解决冲突且没有 AI checkpoint 时，不应凭空归 AI。
- 不影响现有 `merge --squash`、rebase、cherry-pick、stash、commit amend 归因逻辑。
- 不引入大规模普通 merge rewrite 生命周期，先做最小安全修复。

## 非目标

本次不做：

- 不完整迁移 `D:/banz/code/fantacy/git-ai` 的 daemon / trace 架构。
- 不新增完整 `MergeStart / MergeComplete / MergeAbort` rewrite event 生命周期。
- 不默认放开所有 unmerged 文件 checkpoint。
- 不用启发式判断“这段代码像 AI 写的”。仍然只依赖 checkpoint / working log / notes 等明确证据。

## 当前行为分析

当前项目相关路径：

- `src/commands/hooks/merge_hooks.rs`
  - 只处理 `git merge --squash`
  - 普通 `git merge` 没有专门归因逻辑

- `src/commands/hooks/commit_hooks.rs`
  - 普通 merge 冲突解决完成后，最终走普通 `git commit`
  - post hook 生成 `RewriteLogEvent::commit(original_head, new_sha)`

- `src/authorship/post_commit.rs`
  - `post_commit_with_final_state(...)` 从 first-parent/base commit 的 working log 生成 authorship note
  - 对 merge commit 只跳过 stats，不做特殊 attribution 合并

- `src/commands/checkpoint.rs`
  - 对 `EntryKind::Unmerged` 文件明确跳过
  - 现有测试要求冲突解决后必须先 `git add`，再 checkpoint

- `tests/integration/merge_rebase.rs`
  - 已有 `test_merge_conflict_ai_resolution_outside_session`
  - 该测试证明当前设计上支持“AI 解决冲突 -> git add -> checkpoint -> commit”
  - 但 bubble_sort 这种更复杂合并 hunk 仍出现空 note，说明 post-commit 合并层不够稳健

## 参考项目逻辑

`D:/banz/code/fantacy/git-ai` 中关键逻辑：

- `src/authorship/conflict_resolution.rs`
  - `merge_conflict_resolution_authorship(existing_shifted_log, resolution_log, commit_sha)`

核心语义：

1. 先保留已有 shifted attribution。
2. 再读取冲突解决阶段 working log 生成的 `resolution_log`。
3. `resolution_log` 只补充已有 attribution 未覆盖的行。
4. 合并 file attestations 和 metadata。
5. 更新 `base_commit_sha = commit_sha`。

这解决的是：

- 父提交已有 AI 归因不能丢。
- 冲突解决阶段 AI 新写出的行也要归 AI。
- 不应重复或覆盖已有归因。

## 设计方案

### 总体思路

采用“post-commit 窄口 overlay”方案。

不新增普通 merge rewrite event，而是在现有 `post_commit_with_final_state(...)` 中，对 merge commit 做一个很窄的补充分支：

- 如果不是 merge commit：保持原逻辑。
- 如果是 merge commit，但没有 AI resolution checkpoint：保持原逻辑。
- 如果是 merge commit，且 first-parent working log 中存在 AI checkpoint 触碰本次 merge changed files：
  - 生成正常 authorship log。
  - 从 conflict resolution checkpoint 生成 resolution log。
  - 用 `merge_conflict_resolution_authorship(...)` 合并两者。
  - 写入合并后的 note。

### 为什么不先改 merge hook

普通 merge conflict 生命周期复杂：

- `git merge` 可能失败并停在冲突状态。
- 冲突解决可能由 editor / AI / 脚本完成。
- 最终可能用 `git commit`、`git merge --continue` 或 abort。
- 当前项目没有普通 merge pending state。

如果直接在 `merge_hooks.rs` 增加普通 merge lifecycle，需要处理 start / conflict / continue / abort，影响面大。相比之下，post-commit 已经能看到最终 merge commit，是更安全的切入点。

## 文件改动设计

### 1. 新增 `src/authorship/conflict_resolution.rs`

职责：

提供纯 authorship log 合并工具，不依赖 git 操作。

建议移植并适配 `fantacy/git-ai` 的函数：

```rust
pub fn merge_conflict_resolution_authorship(
    existing_shifted_log: Option<AuthorshipLog>,
    resolution_log: AuthorshipLog,
    commit_sha: &str,
) -> AuthorshipLog
```

内部 helper：

- `normalize_line_ranges`
- `subtract_line_ranges`
- `line_coverage_by_file`
- `retain_referenced_metadata`
- `filter_resolution_log_to_uncovered_lines`
- `merge_file_attestations`
- `merge_authorship_metadata`

行为：

- `existing_shifted_log` 为空时，直接以 resolution log 为基础。
- `existing_shifted_log` 非空时，resolution log 只保留未覆盖行。
- metadata 只保留仍被 attestation 引用的 prompt / human / session。
- 最终设置 `metadata.base_commit_sha = commit_sha`。

### 2. 修改 `src/authorship/mod.rs`

导出新模块：

```rust
pub mod conflict_resolution;
```

### 3. 修改 `src/authorship/post_commit.rs`

位置：

`post_commit_with_final_state(...)` 中：

```rust
let (mut authorship_log, initial_attributions) = working_va
    .to_authorship_log_and_initial_working_log(...)?;
```

之后、`notes_add(...)` 之前。

新增逻辑：

```rust
if is_merge_commit(repo, &commit_sha) {
    if let Some(merged_log) = maybe_merge_conflict_resolution_authorship(
        repo,
        &parent_sha,
        &commit_sha,
        authorship_log.clone(),
    ) {
        authorship_log = merged_log;
    }
}
```

其中 helper 逻辑：

1. 判断 `commit_sha` 是否 `parent_count > 1`。
2. 获取 `changed_files = repo.diff_changed_files(parent_sha, commit_sha)`。
3. 读取 `repo.storage.working_log_for_base_commit(parent_sha)`。
4. 判断是否存在非 Human checkpoint 且 entry file 在 changed files 中。
5. 如果没有，返回 `None`。
6. 如果有，构造 `resolution_log`。
7. 调用：

```rust
merge_conflict_resolution_authorship(
    Some(authorship_log),
    resolution_log,
    commit_sha,
)
```

8. 返回合并后的 log。

注意：

- helper 失败不应导致 commit 失败，应 fallback 到原 `authorship_log`。
- merge commit stats 仍保持跳过，不改变现有 stats 行为。

### 4. 可复用 `rebase_authorship.rs` 中逻辑

当前 `src/authorship/rebase_authorship.rs` 已有：

```rust
fn build_note_from_conflict_wl(...)
```

该函数可作为构造 `resolution_log` 的参考，但它当前是私有函数，且返回 `Option<String>`。

建议不要直接扩大它的可见性并强耦合 post_commit，而是抽出一个更通用 helper：

```rust
pub(crate) fn build_authorship_log_from_ai_checkpoints(
    repo: &Repository,
    base_commit: &str,
    commit_sha: &str,
    changed_files: &HashSet<String>,
) -> Option<AuthorshipLog>
```

可以放在：

- `src/authorship/conflict_resolution.rs`，或
- `src/authorship/rebase_authorship.rs`

更推荐放在 `conflict_resolution.rs`，因为它服务的是冲突解决 attribution。

## Checkpoint 行为设计

当前 `checkpoint.rs` 会跳过 unmerged 文件：

```rust
if entry.kind == EntryKind::Unmerged {
    continue;
}
```

第一阶段不建议修改这点。

原因：

- 未 `git add` 的冲突文件可能仍含 conflict markers。
- 贸然允许 checkpoint `UU` 文件，可能把 `<<<<<<<` 等内容记录为 AI。
- 当前测试和注释已经明确要求“先 stage resolved file，再 checkpoint”。

因此第一阶段文档约束：

AI 解决 merge conflict 后，调用顺序必须是：

```bash
git add <resolved-file>
git-ai checkpoint mock_ai <resolved-file>
git commit
```

如果后续确认真实 agent 流程无法保证 `git add` 在 checkpoint 前，再做第二阶段。

### 第二阶段可选方案

仅对显式路径 AI checkpoint 放宽 unmerged skip：

条件：

- checkpoint kind 是 AI。
- 用户显式传入 file path。
- 文件内容可读。
- 文件内容不包含 conflict markers：
  - `<<<<<<<`
  - `=======`
  - `>>>>>>>`

满足条件时，即使 git status 仍是 `UU`，也允许 checkpoint。

不建议放宽：

- 全量 status 扫描。
- human / known_human checkpoint。
- 含 conflict marker 的文件。

## 测试设计

### 测试 1：bubble_sort 普通 merge AI 解决冲突

文件：

`tests/integration/merge_rebase.rs`

目标：

复现 demo 问题，确保第 15、16 行归 AI。

流程：

1. 初始提交 `bubble_sort.py`。
2. 分支 A 添加 `inplace=True`，由 AI checkpoint。
3. 分支 B 添加 `reverse=False`，由 AI checkpoint。
4. 在 `git-ai-2` 合并 `git-ai-1`，产生冲突。
5. AI 写 resolved 内容：

```python
def bubble_sort(arr, *, inplace=True, reverse=False):
    """对列表进行冒泡排序，并返回排序结果。
```

6. `git add bubble_sort.py`
7. `git-ai checkpoint mock_ai bubble_sort.py`
8. commit merge resolution
9. 断言 15、16 行 `.ai()`

### 测试 2：human 解决冲突不应归 AI

同样冲突场景，但 resolve 后不调用 AI checkpoint，或者调用 known human checkpoint。

期望：

- resolved 行不应凭空归 AI。
- 防止 overlay 逻辑过度归因。

### 测试 3：已有简单 merge conflict AI resolution 继续通过

保留并运行：

`test_merge_conflict_ai_resolution_outside_session`

该测试是现有行为保护。

### 测试 4：可选，未 `git add` 前 checkpoint

仅在第二阶段实现时添加。

流程：

1. merge 冲突。
2. AI 写 resolved 文件。
3. 不 `git add`。
4. `git-ai checkpoint mock_ai file`
5. 再 `git add` + commit。
6. 断言 AI attribution。

如果不做第二阶段，不添加该测试。

## 验证命令

先跑目标测试：

```bash
task test TEST_FILTER=merge_conflict_ai_resolution
```

再跑 merge/rebase 相关测试：

```bash
task test TEST_FILTER=merge_rebase
task test TEST_FILTER=rebase_conflict
task test TEST_FILTER=squash_merge
```

最后跑构建：

```bash
task build
```

提交前按项目要求：

```bash
task lint
task fmt
```

## 风险分析

### 风险 1：过度归因

如果只要存在 AI checkpoint 就把整个冲突区域归 AI，可能会把实际从父提交继承的行错误标成 AI。

缓解：

- resolution log 只补未覆盖行。
- 不覆盖 existing shifted attribution。
- human-only resolution 不触发 overlay。

### 风险 2：空 note 被错误保留

当前 demo 中 merge commit note 是空的。如果 helper 认为空 note 已经“处理过”，可能仍不修复。

缓解：

- 判断是否有 actual attestation，而不是只判断 note 是否存在。
- 空 note 不应阻止 merge resolution overlay。

### 风险 3：checkpoint 未记录

如果 AI checkpoint 发生在 `git add` 前，当前项目会跳过 unmerged 文件。post-commit overlay 无法凭空恢复。

缓解：

- 第一阶段明确要求 `git add` 后 checkpoint。
- 第二阶段再做显式路径 AI checkpoint 放宽。

### 风险 4：影响 rebase/cherry-pick/squash

如果把普通 merge 接入 `rebase_authorship` 的 rewrite 事件流，可能破坏已有重写逻辑。

缓解：

- 不新增普通 merge rewrite event。
- 只在 `post_commit_with_final_state` 对 `parent_count > 1` 的 merge commit 做窄处理。
- rebase/cherry-pick/squash 路径保持原样。

## 推荐实现顺序

### Phase 1：测试先行

新增 bubble_sort 回归测试，确认当前失败。

### Phase 2：移植合并 helper

新增 `src/authorship/conflict_resolution.rs`。

### Phase 3：post_commit 窄口接入

在 `post_commit_with_final_state` 中对 merge commit 应用 overlay。

### Phase 4：验证现有测试

确保：

- 新 bubble_sort 测试通过。
- human resolution 测试通过。
- 原 merge/rebase/squash 测试通过。

### Phase 5：评估 checkpoint 放宽

如果真实 agent 流程必须支持 `git add` 前 checkpoint，再做第二阶段。

## 最终结论

当前项目不需要先照搬 `fantacy/git-ai` 的 daemon merge 架构。最小安全修改是：

1. 移植 `merge_conflict_resolution_authorship` 的 uncovered-line 合并语义。
2. 在 `post_commit_with_final_state` 中，仅对普通 merge commit 且存在 AI resolution checkpoint 的场景，合并父提交已有归因与冲突解决归因。
3. 保持 `git add` 后 checkpoint 的现有约束。
4. 用 bubble_sort 回归测试验证第 15、16 行归 AI。

这样能解决当前问题，同时最大限度降低对 rebase、squash、cherry-pick 等既有路径的影响。
