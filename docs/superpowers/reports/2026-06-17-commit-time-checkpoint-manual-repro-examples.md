# Commit-time Checkpoint Manual Reproduction Examples

Date: 2026-06-17

## 目的

本文档提供一组人工操作样例，用于验证以下两个设计覆盖的问题：

- `2026-06-15-commit-time-human-checkpoint-ai-preservation-design.md`
- `2026-06-15-checkpoint-line-attribution-fallback-design.md`

样例要求通过人实际操作复现，不通过测试代码实现，不手动修改
`.git/ai/working_logs`，也不使用 `git-ai checkpoint mock_ai` 伪造 AI
checkpoint。

核心复现链路：

1. 人安装修复前或修复后的 `git-ai` 版本。
2. 人在临时仓库中使用 Claude Code 生成 `fail_example.py`。
3. Claude Code 的正常编辑流程产生 AI checkpoint。
4. 人在 Claude Code 生成文件后执行普通 `git-ai checkpoint -- fail_example.py`，
   模拟错误的后置 Human checkpoint 数据。
5. 提交前执行 `git-ai status` 检查 checkpoint 状态。
6. 提交后执行 `git stats` 检查统计是否失败，并辅助查看 notes / blame。

## 失败样例：旧版本 + Claude Code + 后置 Human checkpoint

### 1. 安装修复前旧版本

在 `git-ai` 源码仓库执行：

```bash
cd /Users/neptune/deepDark/banz/dk/git-ai-code-metrics/git-ai
git checkout 89409ca3
task dev
```

`89409ca3` 是相关修复提交之前的版本。不要使用包含以下提交之后的版本：

- `a3617982`
- `a0e7880b`
- `773b7a04`
- `03904fa0`
- `983daf06`
- `67c5f06e`

### 2. 创建干净复现仓库

```bash
mkdir /tmp/git-ai-fail-example
cd /tmp/git-ai-fail-example
git init
git config user.name "Manual Tester"
git config user.email "manual@example.com"

echo "# demo" > README.md
git add README.md
git commit -m "seed"
```

### 3. 使用 Claude Code 生成 `fail_example.py`

在 `/tmp/git-ai-fail-example` 中打开 Claude Code，让 Claude Code 创建
`fail_example.py`。

可使用以下提示词：

```text
请创建 fail_example.py。写一个 Python 示例模块，包含 FailExample 类，至少 40 行代码，包括初始化、添加 item、过滤 active items、render 输出、build_example 函数和 main 入口。请直接修改文件。
```

要求：

- 文件必须由 Claude Code 生成。
- 不允许执行 `git-ai checkpoint mock_ai fail_example.py`。
- 不允许手动修改 `.git/ai/working_logs`。
- 让 Claude Code 自己的 checkpoint 流程自然记录 AI 编辑。

### 4. 执行普通 Human checkpoint，模拟错误后置 checkpoint

Claude Code 生成 `fail_example.py` 后，在同一个仓库中执行：

```bash
git-ai checkpoint -- fail_example.py
```

这一步是复现关键：它模拟提交阶段或人工流程里追加的 Human checkpoint。在旧版本中，
这个后置 Human checkpoint 可能覆盖 Claude Code 已产生的 AI attribution。

### 5. 提交前检查 checkpoint 状态

```bash
git-ai status
```

要求：

- 输出中应能看到 `fail_example.py` 相关状态。
- 如果完全看不到该文件或 checkpoint 信息，本次复现无效，应重新从干净仓库开始。

### 6. 提交

```bash
git add fail_example.py
git commit -m "add fail example"
```

### 7. 提交后检查失败

```bash
git stats
git notes --ref=ai show HEAD
git-ai blame fail_example.py
```

失败判定：

- `git stats` 显示 `fail_example.py` 的 AI 行数或 AI 占比异常为 0，或主要被统计为
  human / unattributed。
- `git notes --ref=ai show HEAD` 没有 `fail_example.py` 的 AI attestation。
- `git-ai blame fail_example.py` 没有把 Claude Code 生成的行显示为 AI。

只读诊断：

- 可以查看 `.git/ai/working_logs/*/checkpoints.jsonl`，确认前面有 Claude Code 产生的
  AI checkpoint，后面有 Human checkpoint。
- 只能查看，不能修改 checkpoint 文件；否则不再是人工真实复现样例。

## 成功样例：修复后同样流程

### 1. 安装修复后版本

在 `git-ai` 源码仓库执行：

```bash
cd /Users/neptune/deepDark/banz/dk/git-ai-code-metrics/git-ai
git checkout beta
task dev
```

也可以使用任何已经包含修复的版本。

### 2. 创建新的干净复现仓库

```bash
mkdir /tmp/git-ai-success-example
cd /tmp/git-ai-success-example
git init
git config user.name "Manual Tester"
git config user.email "manual@example.com"

echo "# demo" > README.md
git add README.md
git commit -m "seed"
```

### 3. 使用 Claude Code 生成同名文件

在 `/tmp/git-ai-success-example` 中打开 Claude Code，用失败样例中相同的提示词生成
`fail_example.py`。

要求保持一致：

- 文件必须由 Claude Code 生成。
- 不允许执行 `git-ai checkpoint mock_ai fail_example.py`。
- 不允许手动修改 `.git/ai/working_logs`。

### 4. 执行同样的后置 Human checkpoint

```bash
git-ai checkpoint -- fail_example.py
```

### 5. 提交前检查 checkpoint 状态

```bash
git-ai status
```

要求：

- 输出中应能看到 `fail_example.py` 相关状态。
- 如果完全看不到该文件或 checkpoint 信息，本次验证无效，应重新从干净仓库开始。

### 6. 提交并验证

```bash
git add fail_example.py
git commit -m "add fail example"

git stats
git notes --ref=ai show HEAD
git-ai blame fail_example.py
```

成功判定：

- `git stats` 能体现 `fail_example.py` 的 AI 归属，不再显示为 0 AI。
- `git notes --ref=ai show HEAD` 包含 `fail_example.py` 的 AI attestation。
- `git-ai blame fail_example.py` 显示 Claude Code 生成的行归属为 AI。
- 后置 `git-ai checkpoint -- fail_example.py` 没有把 Claude Code 的 AI attribution 覆盖掉。

## 注意事项

- 失败样例必须安装旧版本后执行，不能在修复后版本上期待失败。
- 成功样例必须重新创建干净仓库，不能复用失败样例仓库。
- 不要使用 `git-ai checkpoint mock_ai`，因为目标是验证 Claude Code 真实生成代码过程中的
  checkpoint 数据。
- 不要人工编辑 `.git/ai/working_logs`，否则无法证明问题来自真实操作链路。
- `git-ai status` 是提交前必跑检查；`git stats` 是提交后必跑检查。
