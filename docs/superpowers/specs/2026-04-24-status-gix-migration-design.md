# status.rs gix migration design（2026-04-24）

> 前置阅读：`docs/git2-gix-对照表.md`（`src/git/status.rs` 段落与 “哪些条目更适合 gix 而非 git2” 小节）

## 1. 目标

在不改变 `Repository::status()` 对外返回语义的前提下，把 `src/git/status.rs` 中剩余的 `git status --porcelain=v2 -z` CLI 依赖迁移为进程内实现。

这次迁移的硬目标不是“换一个库拿状态”，而是：

- **完全保留当前 `StatusEntry` 语义**
- **优先采用 `gix` 路线**，与该模块现有 `gix_index` 使用方向一致
- 在 `gix` 某些子能力不顺手时，允许**局部、受控地借用 `git2`**，但不再回退到 `git status --porcelain=v2 -z`

迁移完成后，`Repository::status()` 应继续稳定支持：

- tracked / untracked / ignored 文件状态枚举
- rename / copy / unmerged 语义
- `orig_path` 回填
- pathspec 过滤
- staged 路径与显式 pathspec 的并集语义
- pathspec 超量时的 Rust 侧 post-filter
- macOS NFD/NFC 混用场景下的 NFC 归一化行为

## 2. 非目标

- **不改变 `StatusEntry` / `StatusCode` / `EntryKind` 的公开结构**
- **不调整 `Repository::status()` 函数签名**
- **不把 `repo_state.rs` 合并进本次迁移**；该文件只作为现有“状态元数据可进程内读取”的参考路径
- **不顺带重构 `get_staged_filenames()`**；它已经通过 `gix_index + git2` 实现并稳定工作
- **不把这次迁移扩展为通用状态框架重写**；改动应尽量收敛在 `src/git/status.rs` 与相关测试
- **不以“仿造 porcelain v2 文本再复用 parser”作为长期方案**

## 3. 背景与约束

当前 `Repository::status()` 的实现流程是：

1. 通过 `get_staged_filenames()` 拿到 staged 路径集合
2. 将 staged 路径与显式 `pathspecs` 做并集
3. 根据 pathspec 数量与是否含非 ASCII 决定是否把 pathspec 直接传给 CLI
4. 调用 `git status --porcelain=v2 -z`
5. 用 `parse_porcelain_v2()` 解析为 `Vec<StatusEntry>`
6. 在需要时做 Rust 侧 NFC post-filter

这条路径的问题不在于 parser 本身，而在于：

- 仍有 Windows VM 下高额的 git CLI 固定开销
- 该模块已经在 index 层使用了 `gix_index`，继续依赖 porcelain v2 文本会让实现层次割裂
- 调用方依赖的是 `StatusEntry` 语义，而不是 porcelain v2 文本格式；继续围绕文本格式建模会把内部实现绑死在 CLI 输出上

同时，这里有几条不可退让的现有语义约束：

### 3.1 pathspec 并集语义

即使调用方显式传入了 `pathspecs`，实现仍必须把它与 staged 路径做并集，以免漏掉只存在于 staged 集合中的路径。

### 3.2 全量扫描回退语义

当调用方未传 `pathspecs` 且 staged 集合为空时，仍必须触发一次完整扫描，以捕获纯 unstaged 改动。

### 3.3 `ARG_MAX` / 非 ASCII 回退语义

当 pathspec 数量过多，或任一路径含非 ASCII 时，不能依赖底层库的直接 pathspec 匹配作为唯一真值，而应保留“全量收集 + Rust 侧 NFC post-filter”的现有行为。

### 3.4 macOS NFC 语义

所有对外输出路径以及路径比较都必须继续走 NFC 归一化，避免 `core.precomposeunicode=false` 时 NFD 路径与内部 NFC 路径不匹配。

## 4. 技术路线选择

本次设计评估了三条路线：

### 方案 A — `gix` 主导，`git2` 兜底（推荐）

使用 `gix` 承接 worktree / index / pathspec 相关的底层枚举能力，延续当前模块已使用 `gix_index` 的方向。若 rename / copy 探测或局部对象读取在 `gix` 上实现成本明显更高，则允许局部借用 `git2`，但最终统一组装成现有 `StatusEntry`。

**优点**：

- 与 `status.rs` 现有 index plumbing 方向一致
- 更贴近底层状态枚举模型，不必继续围绕 porcelain v2 文本组织实现
- 长期更容易与 `bash_tool.rs` 等同类状态需求复用

**缺点**：

- 需要自己建立“原始状态事实 → `StatusEntry`”的映射层
- rename / copy / unmerged 等复杂状态不能依赖 CLI 现成输出，需要显式建模

### 方案 B — 直接用 `git2::StatusOptions`

沿 `get_staged_and_unstaged_filenames()` 现有做法继续往前推进，把 `status()` 也改成 `repo.statuses()` 驱动。

**优点**：

- 看起来最接近现有已迁移的小步路径
- 初期可快速拿到基础 tracked/untracked 视图

**缺点**：

- 与对照表结论一致：rename/copy/unmerged/pathspec/NFC 等价性补丁会比较碎
- 抽象层并不贴合当前模块对 index/worktree plumbing 的需求
- 越往“完全等价”补，越容易退化为复杂的兼容层

### 方案 C — 生成“伪 porcelain v2”再复用 parser

底层不再调用 CLI，但内部仍产出一份仿造 porcelain v2 的记录流，再继续交给 `parse_porcelain_v2()`。

**优点**：

- 可以复用现有 parser 与部分测试资产

**缺点**：

- 会形成“双重映射”：先映射到伪 CLI 记录，再映射到 `StatusEntry`
- 维护成本最高，且没有长期架构价值

### 推荐结论

采用 **方案 A：`gix` 主导，`git2` 兜底**。

这条路线最符合文档中“`status()` 更适合优先考虑 `gix`”的结论，也最适合在保持外部语义不变的前提下，逐步摆脱对 porcelain v2 文本格式的依赖。

## 5. 目标架构

`Repository::status()` 迁移后分为三层，且三层职责严格分离。

### 5.1 收集层（collection layer）

职责：从仓库读取原始状态事实，而不是直接生成 `StatusEntry`。

这一层负责收集：

- `HEAD tree` 视图
- `index` 视图
- `worktree` 视图
- untracked / ignored 视图
- rename / copy / unmerged 所需的附加元数据

这层的输出应是内部结构，例如：

```rust
struct CollectedStatusRecord {
    path: String,
    head_state: Option<...>,
    index_state: Option<...>,
    worktree_state: Option<...>,
    kind_hint: CollectedKind,
    orig_path: Option<String>,
}
```

具体字段可以调整，但原则是不暴露给模块外部，并且必须足以表达 rename/copy/unmerged 的原始事实。

### 5.2 归一化层（normalization layer）

职责：对收集层结果做路径与过滤语义归一化。

这一层必须集中处理：

- `nfc_path()` 归一化
- `staged_filenames ∪ pathspecs` 并集逻辑
- “无 pathspec 且 staged 为空时仍做 full scan” 逻辑
- `MAX_PATHSPEC_ARGS` 回退
- 非 ASCII pathspec 触发 post-filter 回退
- `orig_path` 同样参与 post-filter 匹配

设计原则是：**这些规则属于本项目的语义契约，而不是底层库契约**。因此不应散落到收集层里由底层库隐式决定。

### 5.3 组装层（assembly layer）

职责：把内部原始状态记录映射成对外稳定的 `StatusEntry`。

映射时要明确区分：

- `staged`：index 相对 HEAD 的状态
- `unstaged`：worktree 相对 index 的状态
- `kind`：ordinary / rename / copy / unmerged / untracked / ignored
- `orig_path`：rename / copy 情况下的来源路径

这层是“完全等价”的核心承诺点。调用方可观察到的行为全部由这一层负责守住。

## 6. 状态语义映射

### 6.1 Ordinary

普通 tracked 文件的状态由两段差异组成：

- `HEAD -> index`
- `index -> worktree`

分别映射到 `StatusEntry.staged` 与 `StatusEntry.unstaged`。未变化一侧仍应落成 `StatusCode::Unmodified`。

### 6.2 Unmerged

一旦某路径存在 conflict stages 或底层收集结果表明其处于冲突状态，该路径必须稳定映射为：

- `kind = EntryKind::Unmerged`
- 至少一侧状态为 `StatusCode::Unmerged`

这里的关键不是复刻 CLI 某个具体字节序列，而是保住调用方今天依赖的“这是冲突条目”的可观察语义。

### 6.3 Rename / Copy

rename / copy 是当前 CLI 版最容易“白送”的能力，但迁移后必须显式建模：

- 记录目标路径 `path`
- 记录源路径 `orig_path`
- 记录它属于 rename 还是 copy
- 记录它发生在 staged 侧还是 unstaged 侧

设计要求：

- 若底层能够直接给出 rename/copy 结果，则直接采纳
- 若底层对 rename/copy 的表达不足，允许局部借 `git2 diff` 做受控探测
- 无论底层使用哪种方式，最终都统一映射为当前 `StatusEntry` 语义

### 6.4 Untracked

untracked 不应由 tracked diff 逻辑“顺手推导”，而应在收集层作为独立枚举结果进入系统。

`skip_untracked=true` 时：

- 仅裁掉 untracked 条目
- 不应影响 ordinary / rename / copy / unmerged / ignored 的收集与返回

### 6.5 Ignored

当前 parser 已支持 ignored，因此设计上应继续保留 ignored 条目能力，哪怕当前某些调用路径未显式依赖它。

是否默认收集 ignored 由现有行为决定，但内部架构必须允许它作为一种一等状态被表示，而不是未来需要时再大改结构。

## 7. 文件与代码改动边界

本设计要求改动面尽量收敛。

### 7.1 必改文件

- `src/git/status.rs`

### 7.2 高概率会改的文件

- `tests/` 下与 `status.rs` 相关的单测或集成测试文件

### 7.3 原则上不改的文件

- `src/git/repo_state.rs`
- `src/git/repository.rs`（除非极小的共享 helper 抽取确有必要）
- 其他与 status 无直接耦合的 git plumbing 模块

设计原则：这次迁移是一次**语义守恒的实现替换**，不是面向整个 git plumbing 层的重构。

## 8. 分阶段实施策略

为了降低回归风险，本设计要求分三步落地。

### Phase 1 — 引入内部原始状态模型，不切生产路径

先在 `status.rs` 内部引入收集层 / 归一化层 / 组装层所需的新结构与 helper，但 `Repository::status()` 暂时仍以 CLI 结果为主。

这一阶段的目标是：

- 把内部新模型搭起来
- 明确 rename/copy/unmerged/untracked/ignored 的表示方式
- 为后续新旧实现对照测试做好承接点

### Phase 2 — 新旧实现并存，对照验证语义

保留旧 CLI 路径作为测试真值源，让新实现与 `parse_porcelain_v2()` 的结果做一致性对照。

此阶段必须优先验证：

- 普通 tracked 文件
- rename / copy
- unmerged
- `skip_untracked`
- 非 ASCII pathspec
- pathspec 超量触发 post-filter

### Phase 3 — 切换主路径并清理过渡代码

当一致性测试稳定后，再把 `Repository::status()` 主路径切到新实现，最后删除：

- `git status --porcelain=v2 -z` 组参逻辑
- 与生产路径绑定的 parser 依赖
- 不再需要的过渡 helper

`parse_porcelain_v2()` 是否完全删除，可以在 Phase 3 末尾再决定：若仍有测试价值，可只在测试模块中保留；若无，则彻底移除。

## 9. 测试策略

测试目标是验证**外部语义保持不变**，而不是验证“内部具体使用了哪个库”。

### 9.1 核心语义测试

至少覆盖：

- 普通 tracked 修改（仅 staged / 仅 unstaged / staged+unstaged）
- 文件新增与删除
- rename 正确设置 `kind=Rename` 与 `orig_path`
- copy 正确设置 `kind=Copy` 与 `orig_path`
- unmerged 正确设置 `kind=Unmerged`
- `skip_untracked=true` 时仅裁掉 untracked
- ignored 条目行为与当前实现保持一致

### 9.2 路径与过滤测试

至少覆盖：

- 显式 pathspec 与 staged 路径做并集后不漏结果
- pathspec 为空、staged 为空时仍执行完整扫描
- `combined_pathspecs.len() > MAX_PATHSPEC_ARGS` 时走 post-filter
- pathspec 含非 ASCII 时走 NFC post-filter
- `orig_path` 在 post-filter 中同样可命中

### 9.3 边界状态测试

至少覆盖：

- 空仓库 / unborn branch
- 纯 index 改动
- 纯 worktree 改动
- tracked 与 untracked 混合
- 冲突 index stage 存在时的行为

### 9.4 对照测试策略

在迁移过渡期，建议保留“旧 CLI 实现 vs 新实现”的对照测试。该对照测试不是长期架构的一部分，而是为了确保切换前后对调用方的可观察语义完全一致。

## 10. 风险与缓解

### 风险 1：rename / copy 语义偏差

这是最容易与 CLI 行为产生微妙偏差的部分。

**缓解**：

- 在收集层中把 rename/copy 作为一等信息建模
- 在迁移期保留对照测试
- 如 `gix` 路径实现成本明显过高，允许局部 `git2` 兜底，而不是为了“纯 gix”牺牲等价性

### 风险 2：pathspec 与 NFC 过滤回归

现有实现已经明确规避了 macOS NFD/NFC 与 `ARG_MAX` 问题，迁移后若把过滤责任完全下放给底层库，很容易重新引入 bug。

**缓解**：

- 把并集、回退与 post-filter 固定在归一化层
- 明确要求 `orig_path` 也参与过滤

### 风险 3：改动范围扩散成重构

如果在实现时同时抽公共模块、统一多个状态入口，很容易把一次迁移做成大重构。

**缓解**：

- 本次只以 `src/git/status.rs` 为主战场
- 共享 helper 只在“明显减少重复且不增加耦合”时才抽取

## 11. 交付完成定义

本设计对应的实现工作只有在以下条件全部满足时才算完成：

- `Repository::status()` 不再调用 `git status --porcelain=v2 -z`
- 对外 `StatusEntry` 语义保持不变
- pathspec / NFC / post-filter / `skip_untracked` 行为与现实现一致
- rename / copy / unmerged / untracked / ignored 语义通过测试验证
- 至少保留一轮新旧实现一致性验证，证明切换不是拍脑袋完成

## 12. 后续工作（不在本 spec 内实施）

如果本次迁移成功，后续可以复用相同思路推进：

- `src/commands/checkpoint_agent/bash_tool.rs::get_changed_files()`
- 其他需要 index/worktree 状态枚举且更适合 `gix` 的调用点

这些不属于本 spec 的实现范围，但本设计会为它们提供模式参考。
