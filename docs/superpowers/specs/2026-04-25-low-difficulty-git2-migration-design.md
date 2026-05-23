# low-difficulty Git CLI → git2 / gix migration design（2026-04-25）

> 前置阅读：`docs/git2-gix-对照表.md` 中所有“未迁移、低难度、git2 可直接或近直接实现”的条目。

## 1. 目标

在**不改变现有对外行为**的前提下，分批把仓库中剩余的、**低难度 / 只读 / 适合进程内替代** 的 Git CLI 调用迁移为 `git2`、`gix` 或两者配合的实现。

这次迁移的目标不是“继续大面积清空所有 CLI”，而是把最稳、最值、最容易验证的一批点先拿下，并为后续中等难度迁移建立统一约定。

本次设计的硬目标：

- 只迁移**低难度、只读**的 Git CLI 调用
- **以性能优先** 选择后端，而不是预设只能迁到 `git2`
- 允许使用 `git2`、`gix`，或 `git2 + gix` 组合实现单个迁移点
- 优先选择**语义最贴近、性能最好、验证成本最低**的实现路线
- **不改变**函数签名、调用方语义、错误边界与现有测试意图
- 每个迁移函数都显式写出原始 Git CLI 命令注释
- 修改或补充现有 test case，确保测试继续约束“行为等价”，而不是偶然绑定旧实现

## 2. 范围定义

本次“低难度”严格定义为同时满足以下条件的调用：

1. **只读**：不写对象、不改 ref、不发网络请求
2. **低语义风险**：不依赖 patch 文本、porcelain 文本、notes 合并、fast-import 或 pager 语义
3. **git2 或 gix 可直接或近直接实现**：可以用少量对象读取 / revwalk / reference / index / discover API 完成，不需要额外重写复杂 parser
4. **现有行为可通过现有或可补充的测试稳定验证**

## 3. 非目标

以下内容明确不在本次迁移范围内：

- notes 读写、notes merge、fast-import
- diff / patch / porcelain 格式强依赖逻辑
- fetch / push / clone 等网络 IO 主导操作
- `status --porcelain=v2` 等需要完整文本兼容的路径
- `blame --line-porcelain`
- `commit-tree` / `update-ref` / 写对象等写路径
- 顺手做与迁移无关的重构

## 4. 后端选择原则（性能优先）

本次迁移不使用“默认先迁 git2”的单一路线，而采用以下选择顺序：

1. **先看性能与调用热度**：热路径优先选择进程内成本最低、最少字符串/进程往返的方案
2. **再看语义贴合度**：谁更贴近当前需求就用谁，不强求库统一
3. **最后看验证成本**：如果两个库都能做，优先选更容易做行为等价验证的一侧

具体决策规则：

- **对象读取、commit / ref / graph 查询**：通常优先 `git2`
- **路径、仓库发现、index / worktree 视图**：优先评估 `gix`，必要时与 `git2` 组合
- **单点功能若 `gix` 更快、更贴近当前模块语义**：直接用 `gix`
- **单点功能若 `git2` 更成熟、等价关系更直接**：直接用 `git2`
- **若最优方案需要组合**：允许 `git2 + gix`，不为了“纯度”牺牲性能

## 5. 候选项筛选结果

按当前源码与对照表交叉核对，本次最合适的低难度候选可分为三批。

### 5.1 Batch 1 — 纯解析 / 元数据读取（推荐先做）

这些点最稳，且大多更适合用 `git2` 直接承接，适合作为第一批。

- `src/commands/diff.rs::resolve_commit()`
  - 当前命令：`git rev-parse <rev>`
  - 预期替换：优先 `git2::Repository::revparse_single()` + `peel_to_commit()` / `id()`

- `src/api/client.rs::resolve_git_identity()`
  - 当前命令：`git var GIT_COMMITTER_IDENT`
  - 预期替换：优先复用 / 对齐 `repository.rs` 中已有 git identity 解析逻辑；若配置读取路径用 `gix-config` 更直接，也允许切到 `gix`

- `src/commands/checkpoint_agent/bash_tool.rs::get_git_dir()`
  - 当前命令：`git -C <repo> rev-parse --git-dir`
  - 预期替换：优先评估 `gix::discover()` / `open`；若 worktree / common-dir 语义在当前代码里通过 `git2` 更稳，则允许 `git2 + gix` 组合

- `src/commands/prompts_db.rs::reachable_commits()`
  - 当前命令：`git rev-list --all`
  - 预期替换：优先 `git2::Repository::revwalk()`；如后续已有更统一的 `gix` revision walk 基础设施，可在实现时重判

- `src/commands/prompts_db.rs::commit_dates_for()`
  - 当前命令：`git show -s --format=%H %ct <sha>...`
  - 预期替换：优先批量 `git2 find_commit() + Commit::time()`

### 5.2 Batch 2 — 会话 / hooks / daemon 的拓扑与上下文读取

这些调用仍属低难度，但更依赖“顺序、祖先关系、历史展示”的行为一致性，应在 Batch 1 稳住后推进。此批次整体上仍偏向 `git2`，但不排斥局部 `gix` 参与。

- `src/commands/continue_session.rs::CommitInfo::from_commit_sha()`
  - 当前命令：
    - `git log -1 --format=%H|||%an|||%ai|||%s <sha>`
    - `git log -1 --format=%B <sha>`
  - 预期替换：优先 `git2 find_commit()` + author / summary / message 读取

- `src/commands/continue_session.rs::get_git_status_info()`
  - 当前命令：
    - `git branch --show-current`
    - `git log --oneline -5`
  - 预期替换：优先 `git2 head()` / `shorthand()` + `revwalk()` + `summary()`；若分支名读取通过 `gix` 更稳，可做局部组合

- `src/daemon.rs::is_ancestor_commit()`
  - 当前命令：`git merge-base --is-ancestor <ancestor> <descendant>`
  - 预期替换：优先 `git2 graph_descendant_of()`

- `src/commands/hooks/rebase_hooks.rs::walk_first_parent_commits()`
  - 当前命令：`git rev-list --first-parent --topo-order --max-count=N <base>..<head>`
  - 预期替换：优先 `git2 revwalk()` + first-parent / topo-order 等价过滤；若后续已有 `gix` revision walk 实测更优，可重新评估

- `src/commands/hooks/rebase_hooks.rs::is_ancestor()`
- `src/commands/hooks/update_ref_hooks.rs::is_ancestor()`
- `src/commands/hooks/reset_hooks.rs::is_ancestor()`
  - 当前命令：`git merge-base --is-ancestor <ancestor> <descendant>`
  - 预期替换：优先 `git2 graph_descendant_of()`

- `src/commands/hooks/cherry_pick_hooks.rs::expand_commit_range()`
  - 当前命令：`git rev-list --reverse <range>`
  - 预期替换：优先 `git2 revwalk()` + 逆序收集

- `src/commands/hooks/cherry_pick_hooks.rs::resolve_commit_sha()`
  - 当前命令：`git rev-parse <commit_ref>`
  - 预期替换：优先 `git2 revparse_single()` + peel to commit

### 5.3 Batch 3 — 仍属低难度，但要小心 detached/worktree 语义

- `src/daemon/git_backend.rs::repo_context()`
  - 当前命令：`git symbolic-ref --quiet --short HEAD`
  - 预期替换：优先评估 `gix` 的 head / refname 读取路径；若 detached/worktree 语义在 `git2` 更好守住，则允许 `git2 + gix`

- `src/daemon/git_backend.rs::rev_parse_head()`
  - 当前命令：`git rev-parse --verify HEAD`
  - 预期替换：优先 `git2 head()` + `peel_to_commit()`，但不排斥 `gix` 在 repo context 收集层参与

这一批依然属于低难度，但涉及 worktree / detached HEAD 语义，建议作为最后一批做收尾。

## 6. 技术路线评估

本次设计评估三种推进方式。

### 方案 A — 按语义簇分批推进，并按性能选 git2 / gix（推荐）

先迁同一类语义的函数，而不是按文件切；每个点独立选择 `git2`、`gix` 或两者组合。

**优点：**

- 可以复用同一类 helper 和测试思路
- 每一批的风险模型更单一
- 更适合统一引入“原始 git 命令注释”规范
- 可以避免为了“全仓统一一个库”而放弃更快的实现路线

**缺点：**

- 会跨多个文件改动，需要在 PR 描述里解释清楚批次边界

### 方案 B — 先补测试，再决定 git2 / gix 路线

先把所有目标函数的行为回归测试补全，再根据测试约束决定每个点用 `git2` 还是 `gix`。

**优点：**

- 安全性最高
- 更容易界定“行为等价”

**缺点：**

- 前置工作较多
- 有些测试仍需依赖 helper 暴露或间接入口搭桥

### 方案 C — 按文件推进，并统一押注单一后端

例如先完整做 `continue_session.rs`，再做 `prompts_db.rs`，并强行要求整批统一只用 `git2` 或只用 `gix`。

**优点：**

- 组织直观

**缺点：**

- 同类命令（如 `merge-base --is-ancestor`）会在不同文件里重复建模
- 容易把“迁移策略”做成一组不一致的小修小补
- 为了单一后端纯度，容易放弃更好的性能/语义贴合方案

### 推荐结论

采用 **方案 A**，但吸收 **方案 B** 的纪律：

- **按语义簇分批迁移**
- **按性能和语义贴合度选择 git2 / gix / git2+gix**
- **每一批开始前先补齐或调整对应测试**

## 7. 代码注释约定（硬要求）

本次迁移新增一条明确约定：**每个迁移掉 Git CLI 的函数，都必须写出原始命令注释。**

如涉及混合实现，允许追加一行说明：

```rust
// Backend: git2 + gix (performance-first)
```

统一格式：

```rust
// Migrated from: git rev-parse <rev>
```

或在多命令场景：

```rust
// Migrated from:
// - git branch --show-current
// - git log --oneline -5
```

要求：

- 注释放在函数上方，或紧贴关键实现分支
- 注释描述的是**被替换掉的原始 Git CLI 命令**，不是抽象意图
- 如果一个函数替换了多个 CLI 调用，必须全部列出
- 注释应与行为保持同步，后续若再改实现必须同步更新

## 8. 测试策略

本次迁移要求同步修改现有 test case，使测试继续约束行为，而不是意外绑定“旧实现一定通过 exec_git 调 CLI”。

### 8.1 现有测试承接点

#### `tests/integration/git2_migration_aux_comprehensive.rs`

这是当前最适合继续承接非 `repository.rs` 迁移回归测试的文件，已经覆盖：

- `walk_commits_to_base()`
- `search_by_commit_range()`
- `ref_exists()`
- `copy_ref()`
- blame hash 缩写等已迁移辅助路径

本文件后续应继续追加以下测试分组：

- `resolve_commit()` 行为与 `git rev-parse` 对齐
- `reachable_commits()` 的结果集合与 `git rev-list --all` 对齐
- `commit_dates_for()` 的时间戳映射与 Git 结果对齐
- `expand_commit_range()` 顺序与 `git rev-list --reverse` 对齐
- `resolve_commit_sha()` 与 `git rev-parse` 对齐
- `is_ancestor()` / `is_ancestor_commit()` 在相等、祖先、非祖先、缺失对象场景下保持现有行为
- `walk_first_parent_commits()` 对 first-parent 历史保持现有顺序

#### `tests/integration/prompts_db_test.rs`

该文件现在主要验证 `prompts` 数据库和 populate 语义，适合作为 `reachable_commits()` / `commit_dates_for()` 的**行为级回归测试**承接点。

要求：

- 保持现有 tests 的对外行为断言不变
- 视需要追加针对 orphaned commits / since filter / author filter 的更精确数据断言

#### `tests/integration/bash_tool_conformance.rs`

该文件已显式出现 `rev-parse --git-dir` 语义相关断言，是 `get_git_dir()` 迁移后的自然测试承接点。

要求：

- 继续验证 worktree / `.git` 路径语义正确
- 若现有断言过度依赖“手动跑 git 命令拿路径”，则应调整为验证结果路径正确，而非绑定旧实现步骤

#### `continue_session` 相关测试

当前未看到一个现成、专门面向 `CommitInfo::from_commit_sha()` 与 `get_git_status_info()` 迁移回归的集中测试文件。

因此设计要求：

- 优先寻找现有 `continue_session` 集成测试并并入
- 如果没有合适承接点，则新增一个**聚焦 continue_session 读路径**的 integration test 文件

应覆盖：

- subject / full message 的多行提交消息语义
- current branch 展示
- recent commits 的条目数与顺序
- detached HEAD 下的表现

#### `daemon/git_backend` 相关测试

`repo_context()` / `rev_parse_head()` 应优先复用 daemon / trace normalizer / repo context 相关测试，而不是新增过多分散小测。

应覆盖：

- attached HEAD 返回 branch
- detached HEAD 返回 `detached=true`
- HEAD OID 解析在 worktree 下正确

### 8.2 测试设计原则

所有测试都应遵守以下原则：

1. **断言行为，不断言实现手段**
2. 需要与 Git 对照时，可以用真实 `git` 命令生成期望值
3. 不要求测试证明“没有调用 CLI”，而要求证明“输出与现有契约一致”
4. 对于顺序敏感路径（如 `rev-list --reverse`、recent commits、first-parent walk），必须显式断言顺序

## 9. 分批实施要求

### Phase 1 — Batch 1

范围：

- `diff.rs::resolve_commit()`
- `api/client.rs::resolve_git_identity()`
- `bash_tool.rs::get_git_dir()`
- `prompts_db.rs::{reachable_commits, commit_dates_for}`

要求：

- 先补测试，再迁实现
- 每个函数补 `Migrated from:` 注释
- 每个函数实现时明确记录后端选择理由：`git2`、`gix` 或组合
- 不抽象大型共享框架，只做最小复用 helper

### Phase 2 — Batch 2

范围：

- `continue_session.rs::{from_commit_sha, get_git_status_info}`
- `daemon.rs::is_ancestor_commit()`
- `rebase_hooks.rs::{walk_first_parent_commits, is_ancestor}`
- `cherry_pick_hooks.rs::{expand_commit_range, resolve_commit_sha}`
- `update_ref_hooks.rs::is_ancestor()`
- `reset_hooks.rs::is_ancestor()`

要求：

- 优先复用已有 `Repository` 封装和现成 git2 helper
- 可在单点上引入 `gix` helper，但必须有明确性能或语义理由
- 合并同类 ancestor 判断逻辑的测试策略，但不要顺手大改模块结构

### Phase 3 — Batch 3

范围：

- `daemon/git_backend.rs::{repo_context, rev_parse_head}`

要求：

- 聚焦 detached HEAD / worktree 行为等价
- `repo_context()` 默认优先评估 `gix` 路线
- 如果实现需要少量共享 helper，可抽取，但不得引出无关重构

## 10. 实现边界

本次迁移允许：

- 复用 `repository.rs` 中已存在的 git2 能力
- 复用已有 `gix` / `gix_index` 基础设施
- 在单个迁移点上采用 `git2 + gix` 组合
- 为低难度读路径抽取少量共享 helper
- 调整或扩充现有 integration tests

本次迁移不允许：

- 顺带修改外部 API
- 为了“统一风格”重写不在本次范围内的 CLI 路径
- 把中高难度路径混入同一批次

## 11. 验收标准

当且仅当以下条件同时满足，本次迁移才算完成：

- 所有选定的 Batch 1/2/3 目标函数都已迁移到 `git2`、`gix` 或两者组合的进程内实现
- 每个迁移函数都写明 `Migrated from: git ...` 注释
- 每个迁移函数的后端选择符合“性能优先”原则
- 所有相关现有 test case 已更新并通过
- 新增测试覆盖了顺序、祖先关系、branch/detached、commit metadata、git-dir 解析等关键行为
- 未引入对 notes / diff / network / porcelain 类高风险路径的顺手扩张

## 12. 推荐执行顺序

建议严格按照以下顺序推进：

1. Batch 1 测试补齐
2. Batch 1 实现迁移
3. Batch 2 测试补齐
4. Batch 2 实现迁移
5. Batch 3 测试补齐
6. Batch 3 实现迁移
7. 统一跑相关测试与 build 验证

这样能把风险曲线压到最低，也最适合后续拆成多个小 PR。

## 13. 开放问题

本设计默认以下决定已成立：

- “低难度”严格按本文件第 2 节定义收口
- 注释规范采用 `// Migrated from: git ...`
- 以方案 A 为主线推进
- 后端选择遵循“性能优先，允许 git2 + gix 配合迁移”

若后续实现中发现某个 Batch 2/3 项目虽然表面低难度，但为了保持语义等价需要引入明显更高复杂度，则应：

1. 将该项从当前批次移出
2. 在对照表和实现计划里重新标记优先级
3. 不得为了赶进度降低行为等价标准
