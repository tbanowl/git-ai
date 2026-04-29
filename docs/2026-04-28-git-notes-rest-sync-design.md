# Git Notes REST 同步客户端设计

## 背景

`docs/2026-04-28-authorship-notes-incremental-sync-design.md` 已经定义了后台侧的强一致增量协议：服务端为每条 authorship note 维护 `content_hash = sha256(note_content)` 和单调递增的 `change_seq`，`/worker/authorship_notes/list` 返回按 `change_seq ASC` 排序的摘要分页，`/batch` 返回完整 note 内容，`/push` 负责幂等写入。

客户端当前的 REST 同步入口在 `src/git/sync_authorship.rs`：

- `fetch_authorship_notes()` 根据 `Config::get().notes_store()` 选择 Git Notes ref 同步或 REST 同步；
- `rest_fetch_authorship_notes()` 先调用 `/list` 拉取远端全量 `commit_shas`，再用 `note_blob_oids_for_commits()` 判断本地缺失项，最后通过 `/batch` 拉取缺失 note 并用 `notes_add_batch()` 写入 `refs/notes/ai`；
- `rest_push_notes()` 先枚举本地 notes，再调用 `/list` 拉取远端全量 `commit_shas`，只 push 远端不存在的 commit；
- REST API 类型定义在 `src/api/types.rs`，调用封装在 `src/api/authorship_notes.rs`；
- Git Notes 读写工具集中在 `src/git/refs.rs`，包括 `notes_add_batch()`、`show_authorship_note()`、`note_blob_oids_for_commits()`、`list_all_notes()`。

这个流程有两个核心问题：第一，每次 fetch 和 push 都传输远端全量 commit 清单；第二，只判断“远端是否有这个 commit 的 note”，无法发现本地已有 commit 的 note 内容在远端被更新、重写或补录。

## 目标

1. 客户端 fetch 使用 `since_change_seq` 增量拉取远端摘要，避免每次下载全量 `commit_shas`。
2. 客户端能发现本地已有 commit 的 note 内容变化，并通过 `content_hash` 判断是否需要覆盖本地 Git Notes。
3. 客户端 push 使用稳定内容 hash 判断远端缺失或内容不同，避免依赖 `note_blob_oid`。
4. 同步失败时不推进本地水位，避免漏同步。
5. 保持旧服务端和旧响应的基本兼容：如果服务端没有返回 `items`，客户端可以回退到现有全量路径。
6. 保持现有 hook 集成点不变：fetch/pull、push、clone 和 `git ai fetch-notes` 仍调用同一批顶层函数。

## 非目标

1. 本阶段不实现远端删除同步；删除可后续通过服务端墓碑字段扩展。
2. 本阶段不改变本地 Git Notes 存储 ref，仍使用 `refs/notes/ai`。
3. 本阶段不改变 Git 协议同步路径；`notes_store = "git"` 继续走现有 `refs/notes/ai` fetch/push。
4. 本阶段不把 `content_hash` 写入 Git note 内容本身；hash 由同步时读取 note 内容动态计算。
5. 本阶段不要求 push 侧必须做到 O(本地差异)；初始版本可分页拉远端摘要，但不再拉完整内容或依赖 blob oid。

## 推荐方案

客户端新增一个 REST notes 同步状态文件，记录每个规范化 `repo_url` 的 fetch 水位：

```json
{
  "schema_version": 1,
  "repo_url": "https://example.com/org/repo.git",
  "last_change_seq": 12346,
  "updated_at": 1775973635847
}
```

fetch 侧按 `change_seq` 增量分页，逐页比较远端摘要和本地 note 内容 hash，只对缺失或 hash 不同的 commit 调 `/batch`，成功写入本地 Git Notes 后才允许推进本地水位。push 侧枚举本地 Git Notes 并计算 `sha256(content)`，再用远端摘要构建 `{commit_sha -> content_hash}` 映射，只 push 远端缺失或 hash 不同的 note。

`note_blob_oid` 保留为 API 兼容字段和调试信息，但不参与一致性判断。

## 本地状态设计

### 存储位置

推荐存储在仓库 `.git` 内，避免跨仓库污染：

```text
.git/ai/rest_notes_sync_state/<repo-key>.json
```

`repo-key` 基于 `normalize_repo_url()` 的输出生成，要求：

1. 同一个远端 URL 的 SSH/HTTPS 形式归一后使用同一个 key；
2. 文件名不包含路径分隔符、冒号或 shell 特殊字符；
3. key 可使用 `sha256(normalized_repo_url)`，避免 URL 过长和隐私泄露。

### 状态字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `schema_version` | 整数 | 初始为 `1`，便于未来迁移 |
| `repo_url` | 字符串 | 规范化后的仓库 URL，用于校验状态文件没有被误用 |
| `last_change_seq` | 整数 | 已成功应用到本地 Git Notes 的最大远端变更序号 |
| `updated_at` | 整数 | 客户端更新时间，毫秒时间戳，仅用于调试 |

### 水位推进规则

`last_change_seq` 只在以下条件全部满足后更新：

1. 本轮 `/list` 的所有分页都成功返回；
2. 所有需要拉取完整内容的 commit 都已通过 `/batch` 成功返回，且没有当前摘要页中的 commit 出现在 `missing` 中；
3. 所有返回的 note 内容都已通过 `notes_add_batch()` 成功写入本地 `refs/notes/ai`；
4. 本轮没有 hash 校验失败、JSON 解析失败、网络错误或 Git Notes 写入错误。

如果任一步失败，客户端保留旧 `last_change_seq`。下次 fetch 会从旧水位重新拉取，允许重复应用同一批 notes；`notes_add_batch()` 对同一 commit 覆盖写入是可接受的幂等行为。

## API 类型变更

客户端在 `src/api/types.rs` 中扩展现有类型，字段默认可选以兼容旧服务端。

### List 请求

```json
{
  "repo_url": "https://example.com/org/repo.git",
  "since_change_seq": 12345,
  "limit": 1000
}
```

新增字段：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `since_change_seq` | `Option<i64>` | 只请求 `change_seq > since_change_seq` 的记录 |
| `limit` | `Option<usize>` | 分页大小，客户端建议默认 `1000` |

旧字段 `since_commit_time` 可以保留，但新客户端不再依赖它做强一致增量。

### List 响应

```json
{
  "ok": true,
  "data": {
    "commit_shas": ["abc123"],
    "items": [
      {
        "commit_sha": "abc123",
        "content_hash": "sha256:...",
        "change_seq": 12346,
        "updated_at": 1775973635847
      }
    ],
    "next_change_seq": 12346,
    "has_more": false
  }
}
```

新增客户端结构：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `items` | `Option<Vec<AuthorshipNotesListItem>>` | 新增量摘要；缺失时表示旧服务端 |
| `next_change_seq` | `Option<i64>` | 当前页最大 `change_seq` |
| `has_more` | `Option<bool>` | 是否还有下一页 |

`AuthorshipNotesListItem` 字段：`commit_sha`、`content_hash`、`change_seq`、`updated_at`。

### Batch 响应

`AuthorshipNotesBatchItem` 增加可选字段：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `content_hash` | `Option<String>` | 服务端基于返回 `content` 计算的 hash |
| `change_seq` | `Option<i64>` | 与 list 摘要对应的服务端变更序号 |

客户端收到 batch note 后必须基于 `content` 自行计算 hash。如果 list 摘要中有该 commit 的 expected hash，且本地计算值不匹配，则本轮 fetch 失败并不推进水位。

### Push 响应

`AuthorshipNotesPushData` 增加：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `unchanged` | `Option<usize>` | 内容相同而被服务端跳过的数量 |

客户端不信任也不要求服务端使用客户端传入的 hash。即使未来 push item 增加 `content_hash` 字段，服务端仍必须按收到的 `content` 自行计算。

## Fetch 同步流程

### 正常增量路径

1. 解析 remote：继续复用 `fetch_remote_from_args()` 和 `normalize_repo_url()`。
2. 读取 `.git/ai/rest_notes_sync_state/<repo-key>.json`；不存在或 repo_url 不匹配时使用 `last_change_seq = 0`。
3. 调用 `/worker/authorship_notes/list`，传入 `{ repo_url, since_change_seq: last_change_seq, limit }`。
4. 如果响应缺少 `data.items`，回退到旧全量 fetch 路径。
5. 对当前页每个 item：
   - 本地没有 note：加入 `to_fetch`；
   - 本地已有 note：用 `show_authorship_note()` 读取内容，计算 `sha256(content)`，与 `item.content_hash` 比较；不同则加入 `to_fetch`；
   - hash 相同：跳过 batch 拉取。
6. 对 `to_fetch` 调 `/worker/authorship_notes/batch` 获取完整内容。
7. 对 batch 返回的每条 note：
   - 计算 `sha256(content)`；
   - 与 list item 的 `content_hash` 比较；不一致则失败；
   - 收集 `(commit_sha, content)`。
8. 调用 `notes_add_batch()` 写入本地 `refs/notes/ai`。
9. 如果 `has_more = true`，用当前页 `next_change_seq` 继续请求下一页，但暂不落盘最终水位。
10. 所有分页和 batch 都成功后，把状态文件的 `last_change_seq` 更新为最后成功页的 `next_change_seq`。

### 旧服务端兼容路径

如果 `/list` 返回 `commit_shas` 但没有 `items`，客户端执行现有逻辑：

1. 使用远端全量 `commit_shas`；
2. 用 `note_blob_oids_for_commits()` 或 `commits_with_authorship_notes()` 判断本地缺失；
3. 对缺失项调用 `/batch`；
4. 用 `notes_add_batch()` 写入。

兼容路径不更新 `last_change_seq`，因为旧服务端没有强一致水位。

### 缺失和异常处理

| 场景 | 行为 |
| --- | --- |
| `/batch` 返回 `missing` | 记录 debug 日志；如果 missing commit 来自当前页摘要，视为服务端不一致，本轮失败且不推进水位 |
| 本地 note 读取失败且错误是“note 不存在” | 加入 `to_fetch` |
| 本地 note 读取失败且是 Git 命令错误 | 本轮失败且不推进水位 |
| `content_hash` 格式不是 `sha256:<hex>` 或 `<hex>` | 本轮失败且不推进水位 |
| `next_change_seq` 小于当前页最大 `change_seq` | 本轮失败且不推进水位 |
| 分页重复返回同一 `commit_sha` | 以后出现的较大 `change_seq` 覆盖前一摘要；仍按页序处理 |

## Push 同步流程

### 初始实现

1. 用 `list_all_notes()` 或现有 `git notes --ref=ai list` 枚举本地 Git Notes。
2. 对每个本地 note 调 `show_authorship_note()` 读取内容并计算 `content_hash = sha256(content)`。
3. 分页调用 `/worker/authorship_notes/list` 拉取远端摘要，构建 `remote_hash_by_commit`。
4. 对每个本地 note：
   - 远端没有该 `commit_sha`：加入 `to_push`；
   - 远端 hash 不同：加入 `to_push`；
   - 远端 hash 相同：跳过。
5. 对 `to_push` 保持现有 push item 内容：`branch`、`commit_sha`、`note_blob_oid`、`author_name`、`author_email`、`content`、`commit_time`。
6. 调 `/worker/authorship_notes/push`。
7. 记录 `created`、`updated`、`unchanged` 统计；`unchanged` 缺失时按旧服务端响应处理。

这个版本仍可能需要分页扫描远端全部摘要，但传输的是稳定摘要而不是完整 note 内容，并且能发现内容差异。后续如果本地 notes 很多，可以增加 `/worker/authorship_notes/diff`，由客户端上传 `{commit_sha, content_hash}` 摘要，让服务端返回缺失或不同的 commit 列表。

### Push 与 fetch 水位的关系

push 成功后不直接推进 fetch 的 `last_change_seq`。原因是服务端可能为本次 push 分配新的 `change_seq`，客户端只有在下一次 `/list` 看见并成功应用这些摘要后，才能证明本地水位覆盖了这些服务端变更。

为了优化重复 push，push 侧可以只依赖远端摘要 hash 去跳过相同内容；不需要单独维护 push 水位。

## Hash 规范

1. 算法固定为 SHA-256。
2. 输入为 Git note content 的 UTF-8 原文 bytes，不做 JSON 解析、格式化或换行归一化。
3. 协议层接受两种格式：`sha256:<64 hex>` 和 `<64 hex>`；客户端内部统一规范化为小写 `<64 hex>` 比较。
4. 如果 note content 不是有效 UTF-8，但 Git 命令能返回 bytes，后续可扩展 byte hash；当前 authorship notes 是 JSON 文本，本阶段按 UTF-8 文本处理。

## 并发与一致性

### 多客户端并发

服务端以 `content_hash` 做幂等判断：重复 push 相同内容不推进 `change_seq`；内容变化才推进。客户端 fetch 使用服务端 `change_seq` 水位，因此可以发现其他客户端对旧 commit note 的更新。

### 本地并发

fetch/push hook 可能由多个 Git 命令触发。状态文件写入应采用原子替换：

1. 写入同目录临时文件；
2. flush 文件内容；
3. rename 覆盖正式状态文件。

如果实现已有仓库级锁机制，应复用；否则至少保证状态文件不会写出半截 JSON。即使两个 fetch 并发执行，较旧水位的任务最多重复拉取和重复写入 notes，不应把水位回退。写状态前应重新读取现有状态，只允许写入 `max(existing.last_change_seq, new_last_change_seq)`。

## 代码改动范围

| 文件 | 改动 |
| --- | --- |
| `src/api/types.rs` | 扩展 list/batch/push 的请求响应字段，新增 `AuthorshipNotesListItem` |
| `src/api/authorship_notes.rs` | 保持 endpoint 不变，确保新字段序列化/反序列化生效 |
| `src/git/sync_authorship.rs` | 新增状态读写、hash 计算、增量 fetch、hash-based push |
| `src/git/refs.rs` | 复用现有 Git Notes 读写；如有必要增加批量读取 note 内容 helper |
| `src/repo_url.rs` | 复用规范化 URL；不改变现有行为 |
| `tests/` | 增加 REST 增量协议相关单元/集成测试 |

## 测试计划

### API 类型测试

1. 新客户端能反序列化包含 `items`、`next_change_seq`、`has_more` 的 list 响应。
2. 新客户端能反序列化旧 list 响应；`items = None` 时触发兼容路径。
3. batch item 的 `content_hash` 和 `change_seq` 缺失时仍兼容旧服务端。
4. push response 缺失 `unchanged` 时仍兼容旧服务端。

### Fetch 客户端测试

1. 本地缺失 note 时，list 摘要触发 batch 拉取并写入 `refs/notes/ai`。
2. 本地已有 note 且 hash 相同，不调用 batch 拉取。
3. 本地已有 note 但 hash 不同，调用 batch 并覆盖本地 note。
4. 多页 list 全部成功后才推进 `last_change_seq` 到最后一页 `next_change_seq`。
5. 第二页失败时，状态文件保持旧 `last_change_seq`。
6. batch 内容 hash 与 list 摘要不一致时失败，不写入水位。
7. list 返回 `items = None` 时走旧全量路径且不写水位。
8. 不同 `note_blob_oid` 但相同 note 内容时，不触发重复同步。

### Push 客户端测试

1. 远端缺失 commit 时 push 本地 note。
2. 远端 hash 相同的 note 不 push。
3. 远端 hash 不同的 note 会 push 更新。
4. push 响应包含 `unchanged` 时正确统计；缺失时不失败。
5. push 成功不直接更新 fetch `last_change_seq`。

### Git Notes 集成测试

1. 使用 `TestRepo` 创建真实仓库，验证 REST fetch 后 `file.assert_committed_lines()` 仍能从 `refs/notes/ai` 读取正确 attribution。
2. 对每次 commit 后的 authorship note 都断言 line-level attribution，避免只验证 note 存在。
3. clone/fetch/pull/push hooks 在 `notes_store = "rest"` 时仍调用 REST 路径。

## 风险与处理

| 风险 | 处理 |
| --- | --- |
| 旧服务端没有增量字段 | `items = None` 时回退旧全量路径，不推进水位 |
| 状态文件提前推进导致漏同步 | 只有所有分页和 batch 成功写入 Git Notes 后才原子写水位 |
| hash 格式不一致 | 客户端统一规范化 `sha256:<hex>` 和 `<hex>`；非法格式直接失败 |
| push 侧分页拉远端摘要仍较大 | 初始版本先保证正确性；后续新增 `/diff` 减少远端摘要传输 |
| 并发 fetch 写状态文件 | 写入时取 `max(existing, new)`，避免水位回退 |
| 本地 note 在 fetch 过程中被其他进程修改 | 最终以服务端当前页 hash 和 batch 内容为准；失败不推进水位，下次重试 |

## 实施顺序建议

1. 扩展 `src/api/types.rs` 的 REST 类型，保持 serde 向后兼容。
2. 在 `src/git/sync_authorship.rs` 增加 `sha256_note_content()`、hash 规范化、状态文件路径、读写和原子更新 helper。
3. 重写 `rest_fetch_authorship_notes()` 为增量分页流程，保留旧全量 fallback。
4. 重写 `rest_push_notes()` 为 hash-based 差异判断。
5. 增加 API 类型和同步状态单元测试。
6. 增加 REST fetch/push 行为测试，覆盖失败不推进水位和 hash 差异覆盖。
7. 运行 `task fmt`、`task lint`、相关 `task test TEST_FILTER=...`，最后视改动范围运行 `task test`。
