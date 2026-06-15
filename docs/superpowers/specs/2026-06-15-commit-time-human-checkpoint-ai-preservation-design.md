# Commit-time Human Checkpoint AI Preservation Design

Date: 2026-06-15

## 背景

`git-ai` 通过 checkpoint 记录文件在编辑过程中的归属状态，并在 `git commit` 的 post-commit 阶段把本次提交的 AI 行写入 `refs/notes/ai`。已定位的 `LotHoldListDlg.cpp` 案例中，`.cpp` 文件在提交中是新增文件，`git diff` 有 3443 行 added lines，但最终 git notes 没有 `.cpp` 的 AI 信息。

直接证据来自 `/Users/neptune/deepDark/cai/git-notes-test2/.git/ai/working_logs/old-c3cdc8ce9e0bd4d2535ed1861f6941a2d86495f7/checkpoints.jsonl`：

- 第 27 行是 `AiAgent` checkpoint，`PPTClient/Lot/LotHoldListDlg.cpp` 有 13 条 AI `line_attributions`，但 `attributions` 为空。
- 第 28 行是在提交流程中产生的 `Human` checkpoint，同一个 `.cpp` 文件的 `line_attributions` 为空，`attributions` 为两条 `human`，其中一条覆盖整份文件内容。
- post-commit 读取 checkpoint 时按顺序处理，同一文件后面的 entry 覆盖前面的 entry；第 28 行因此把第 27 行的 AI 归属覆盖成 human。
- `to_authorship_log_and_initial_working_log()` 之后跳过 `author_id == human` 的 committed lines，导致 `.cpp` 没有任何 AI attestation。

已有设计 `2026-06-15-checkpoint-line-attribution-fallback-design.md` 解决了“上一条 checkpoint 只有 line-level AI attribution 时，后续 checkpoint 如何重建 previous state”的问题，但它不覆盖“commit-time / synthetic Human checkpoint 仍可能把已有 AI attribution 整文件覆盖成 human”的前向行为。

## 范围

本设计只修复前向行为：以后提交流程生成的 commit-time / synthetic `Human` checkpoint 不应把已有 AI line attribution 整文件覆盖成 human。

本设计不包含旧数据自动恢复：

- 不自动重写 archived working logs。
- 不自动改写已经生成的 git notes。
- 不新增修复旧数据的命令或脚本。
- 不改变 authorship note schema。

这样做的原因是旧数据恢复需要判断“后置 all-human checkpoint 是 synthetic 捕获错误，还是用户真实人工重写了文件”。在没有额外证据时自动恢复有误判风险。

## 目标

- 当一个文件已有 AI attribution，而 commit-time `Human` checkpoint 捕获最终提交快照时，保留未被真实 human 改写的 AI 行。
- 防止 synthetic `Human` checkpoint 把同文件早先的 AI attribution 全量清空。
- 保持真实 human 编辑覆盖 AI 行的现有语义：人真正改写的行仍应归属 human。
- 保持 post-commit notes 阶段的语义不变：notes 只记录本次提交中与 committed added lines 相交的非 human attribution。
- 用回归测试覆盖 `LotHoldListDlg.cpp` 的最小失败形态。

## 非目标

- 不在 `post_commit.rs` 中硬编码“如果后一个 Human checkpoint 是 all-human 就忽略它”。这种策略会误伤真实人工重写。
- 不让 notes 记录 parent 中已经存在的 AI 行。notes 仍只记录本次提交新增或修改后进入 commit 的 AI lines。
- 不引入新的归属格式，也不改变 `WorkingLogEntry` 的序列化字段。
- 不把所有 line-only attribution 都强制提升成永久 char-level 存储；只在 checkpoint 计算需要 previous state 时做必要转换。

## 可选方案

### 方案 A：在 post-commit 阶段忽略可疑 all-human checkpoint

在 `VirtualAttributions::from_working_log_snapshot()` 或 `from_just_working_log()` 中，如果发现同一文件先有 AI attribution、后有 `Human` checkpoint 且后者转换出的 line attribution 为空，就保留前者。

优点：实现位置靠近最终症状，能直接让 notes 出现 `.cpp`。

缺点：无法可靠区分 synthetic checkpoint 与真实人工重写。真实用户把 AI 文件重写成 human 时，也可能被错误恢复为 AI。这个方案修的是消费端症状，不是 checkpoint 生成端根因。

### 方案 B：在 checkpoint 生成阶段保留 previous AI attribution（推荐）

在 `src/commands/checkpoint.rs` 中，确保 `Human` checkpoint 处理已有 previous state 时：

1. 如果 previous state 有 char-level attribution，继续使用现有 char attribution。
2. 如果 previous state 只有 line-level attribution，用 previous blob 内容转换成 char attribution。
3. 用 `make_entry_for_file()` 比较 previous content 与 current content，只把真实 diff 产生的人类修改标成 human，未变的 AI ranges 保持 AI。

优点：修复根因，适用于普通 Human checkpoint 与 commit-time synthetic Human checkpoint；不改变 post-commit 输出语义；能保留真实人工改写行为。

缺点：需要精确测试 checkpoint 生成逻辑，尤其是 line-only previous state 与 current content 不同版本之间的归属转换。

### 方案 C：给 commit-time checkpoint 增加专门模式

为提交流程产生的 Human checkpoint 增加显式模式，例如 `is_pre_commit` 或 `is_commit_snapshot` 下使用不同合并策略：只捕获最终快照，不覆盖 prior AI attribution。

优点：语义最明确，能把真实 editor human checkpoint 与 commit hook synthetic checkpoint 分开。

缺点：需要沿 daemon/control API、checkpoint request、hook 调用链传递更多上下文，改动面更大。除非现有 `is_pre_commit` 信息不足，否则不应优先采用。

## 推荐设计

采用方案 B：把修复放在 checkpoint 生成阶段，并复用已有 `PreviousFileState` / `get_checkpoint_entry_for_file()` / `make_entry_for_file()` 数据流。

核心原则：后续 checkpoint 不能因为 previous state 的 `attributions` 为空就认为文件没有 prior AI attribution。只要 previous state 有非空 `line_attributions`，就必须基于 previous blob 内容恢复出可参与 diff tracking 的 char attribution。

### 数据流

1. AI agent 写入文件并产生 checkpoint。历史上可能出现 `line_attributions` 非空但 `attributions` 为空的 entry。
2. 后续 `Human` checkpoint 运行时，`build_previous_file_state_maps()` 选择同文件最新 previous entry，并保留 `blob_sha`、`attributions`、`line_attributions`。
3. `get_checkpoint_entry_for_file()` 读取 previous state：
   - 优先使用非空 `attributions`；
   - 否则使用 `line_attributions_to_attributions(previous_state.line_attributions, previous_content, ts - 1)`；
   - 如果两者都为空，才按无归属处理。
4. `make_entry_for_file()` 用 previous content / previous attribution 与 current content 做 diff tracking。
5. 生成的 `Human` checkpoint entry 应包含保留下来的 AI attribution，以及实际 human 修改产生的 human attribution。
6. post-commit 继续按现有规则把本次提交 added lines 中的非 human attribution 写入 git notes。

### 行为约束

- 如果 previous char attribution 非空，不使用 line fallback 覆盖它。
- 如果 previous line attribution 非空但 previous blob 读取失败，应返回错误或保持现有错误路径，不能静默整文件 human。
- 如果 current content 与 previous content 字节完全相同，仍可按现有 no-op 逻辑跳过 entry。
- 如果只有换行符差异，应保留现有 CRLF/LF remap 逻辑。
- 如果 human 真正删除或改写 AI 行，`make_entry_for_file()` 应继续把对应行标为 human 或移除 AI。

## 测试设计

采用 TDD。先写失败测试，确认当前 bug 真实复现，再实现最小修复。测试应优先放在 `tests/integration/simple_additions.rs` 或相近 checkpoint/attribution 集成测试文件中；如果为了避免污染已有大文件，也可以新增一个聚焦的 integration test 文件。

### 回归测试 1：复现 `old-c3...` 的最小版本

目标：用小型 `foo.cpp` 复现 `LotHoldListDlg.cpp` 的关键形态：新文件、本次提交有 added lines、提交前已有有效 AI line attribution、commit-time `Human` checkpoint 追加后不应清空 AI 归属。

步骤：

1. 使用 `TestRepo::new()` 创建真实 git repo。
2. 显式创建新文件 `foo.cpp`，内容用多行 C++ 风格文本即可，例如 20-50 行，避免只测单行路径。
3. 调用 `mock_ai`，让 `foo.cpp` 先得到 AI attribution。
4. 将该 checkpoint 调整或构造成 line-only 形态：`line_attributions` 非空、`attributions` 为空，用来模拟 `old-c3...` 第 27 行的历史数据形态。
5. 模拟 commit-time `Human` checkpoint 捕获最终内容。它代表提交流程追加的第 28 行，而不是用户主动重写文件。
6. 提交 `foo.cpp`。
7. 断言 `refs/notes/ai` 中包含 `foo.cpp` 的 AI attestation，或用 `assert_committed_lines()` 断言新增行仍是 AI。

修复前必须先看到测试失败。正确的失败表现是：

- notes 中没有 `foo.cpp`；或
- `foo.cpp` 在 notes 中没有任何非 human attribution；或
- committed blame / line assertion 显示这些新增行被记成 human。

修复后预期：

- notes 包含 `foo.cpp`；
- `foo.cpp` 的未被 human 改写的新增行保留 AI prompt hash；
- commit-time `Human` checkpoint 不再把 prior AI attribution 整文件覆盖成 human。

### 回归测试 2：实现前后的红绿流程

这个测试不是额外场景，而是执行约束：实现者必须按以下顺序工作。

1. 先提交测试代码，不改 `src/commands/checkpoint.rs`。
2. 运行 `task test TEST_FILTER=<新增测试名>`。
3. 确认测试因为 `foo.cpp` 缺少 AI notes 或变成 human 而失败。
4. 再修改 `src/commands/checkpoint.rs`：
   - 确保 `PreviousFileState` 保留 `line_attributions`；
   - 确保 `Human` checkpoint 在 previous state 只有 line attribution 时，会把 previous blob 的 line attribution 转换成 char attribution，并传入 `make_entry_for_file()`；
   - 确保 commit-time pre-commit / synthetic checkpoint 不会把 prior AI attribution 整文件覆盖成 human。
5. 再次运行同一个测试，确认测试通过。

### 回归测试 3：真实 human 重写仍覆盖 AI attribution

目标：证明修复不是简单忽略后置 `Human` checkpoint。

步骤：

1. AI 写入 `foo.cpp` 并 checkpoint。
2. Human 明确重写整份 `foo.cpp`，内容与 AI 版本有实质差异。
3. 调用 Human checkpoint。
4. 提交。
5. 断言 notes 不应错误保留旧 AI；被 human 重写后的新增行应为 human 或没有 AI attestation。

这个测试防止方案退化成“任何后置 Human checkpoint 都保留前一条 AI attribution”。

### 回归测试 4：已有 char attribution 优先级不变

步骤：

1. 构造 previous state 同时有 char attribution 和 line attribution。
2. 运行后续 Human checkpoint。
3. 断言使用 char attribution 作为 previous ownership，不被 line fallback 覆盖。

这个测试保护现有更精确的 char-level 行为。

### 手工验证：以 `git-notes-test2` 为参照

自动化测试通过后，还需要用 `/Users/neptune/deepDark/cai/git-notes-test2` 作为真实参照做最终验证。

验证目标不是“只要看到 `.cpp` 就算成功”，而是：当 `LotHoldListDlg.cpp` 是本次提交新增文件，且提交前已有有效 AI line attribution，即使提交流程追加了 `Human` checkpoint，最终 notes 仍应保留那些没有被真实 human 重写的 AI 行。

建议验证步骤：

1. 使用修复后的 `git-ai` debug build，通过项目约定的 `task dev` 安装本地调试版本。
2. 在 `/Users/neptune/deepDark/cai/git-notes-test2` 中复现 old-c3 类流程，或基于已有 `old-c3cdc8ce9e0bd4d2535ed1861f6941a2d86495f7` 数据重新执行等价提交。
3. 确认实际 commit diff 中 `PPTClient/Lot/LotHoldListDlg.cpp` 是 added/modified，并且有 committed added lines。
4. 查看 `git notes --ref=ai show <commit>`。
5. 验证 notes 中包含 `PPTClient/Lot/LotHoldListDlg.cpp`，且至少包含 `b1ec13f32c390334` 或 `749e81da762200d1` 这类 AI prompt hash 对应的 line ranges。
6. 验证 `PPTClient/Lot/LotHoldListDlg.h` 的既有 AI 归属不回退。
7. 如果 notes 中 `.cpp` 仍缺失，检查最新 `checkpoints.jsonl` 中 commit-time `Human` checkpoint 是否仍把 `.cpp` 写成整文件 human；如果是，说明修复没有进入正确 checkpoint 生成路径。

## 代码影响范围

主要文件：

- `src/commands/checkpoint.rs`
  - `PreviousFileState`
  - `build_previous_file_state_maps()`
  - `get_checkpoint_entry_for_file()`
  - 与 previous attribution reconstruction 相关的单元测试或集成测试辅助

测试文件：

- `tests/integration/simple_additions.rs`，或新增更聚焦的 attribution/checkpoint integration test。
- 必要时更新 `tests/integration/snapshots/` 中对应快照。

不建议改动：

- `src/authorship/post_commit.rs`：保持 notes 输出语义。
- `src/authorship/virtual_attribution.rs`：除非测试证明消费旧 working log 时还有独立 bug，否则不要在这里绕过后置 Human checkpoint。
- authorship note schema 和 prompt metadata 存储逻辑。

## 错误处理

- 对 previous blob 读取失败的 line fallback，不应静默降级为整文件 human。静默降级会重新制造本问题。
- line range 超出 previous content 范围时，沿用 `line_attributions_to_attributions()` 的现有行为：无法映射的 range 不产生 char attribution。
- 如果转换后仍为空，才允许后续逻辑按无 prior attribution 处理。

## 验收标准

- 对 `LotHoldListDlg.cpp` 的最小复现：第 27 类 line-only AI checkpoint 后，即使提交前产生 Human checkpoint，最终 commit notes 仍包含 `.cpp` AI attribution。
- `.h` 这类本来已有 line attribution 的文件行为不回退。
- 真正 human 重写 AI 内容时，重写行仍归属 human。
- `task test TEST_FILTER=<新增测试名>` 通过。
- `task test TEST_FILTER=simple_additions` 或相关集成测试通过。
- `task build` 通过。

## 后续可选工作

旧数据恢复应单独设计。若后续确实需要恢复 archived working logs 或已写 notes，应另起设计，明确以下判定条件：

- 如何证明后置 all-human checkpoint 是 commit-time synthetic，而不是用户真实重写。
- 如何比较 previous AI checkpoint blob、Human checkpoint blob、最终 commit blob。
- 是否需要用户逐文件确认。
- 是否允许重写已有 git notes。

本设计不处理这些问题。
