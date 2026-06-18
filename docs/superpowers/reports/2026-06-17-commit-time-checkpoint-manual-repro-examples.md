# Commit-time Checkpoint Manual Reproduction Examples

Date: 2026-06-17

## 目的

本文档提供一组人工操作样例，用于验证以下两个设计覆盖的问题：

- `2026-06-15-commit-time-human-checkpoint-ai-preservation-design.md`
- `2026-06-15-checkpoint-line-attribution-fallback-design.md`

样例要求通过人实际操作复现，不通过测试代码实现；除本文明确要求把 AI
checkpoint blob 内容改成 CRLF 外，不手动修改 `.git/ai/working_logs`
中的 `checkpoints.jsonl` 或其他 attribution 数据，也不使用
`git-ai checkpoint mock_ai` 伪造 AI checkpoint。

核心复现链路：

1. 人安装修复前或修复后的 `git-ai` 版本。
2. 人在临时仓库中使用 Claude Code 生成 `fail_example.py`。
3. Claude Code 的正常编辑流程产生 AI checkpoint。
4. 人把 Claude Code 产生的 AI checkpoint blob 内容改成 CRLF，同时把工作区
   `fail_example.py` 保持为 LF，制造 checkpoint blob 与源代码换行符不一致。
5. 人在 Claude Code 生成文件后执行普通 `git-ai checkpoint -- fail_example.py`，
   模拟错误的后置 Human checkpoint 数据。
6. 提交前执行 `git-ai status` 检查 checkpoint 状态。
7. 提交后执行 `git stats` 检查统计是否失败，并辅助查看 notes / blame。

## 失败样例：旧版本 + Claude Code + 后置 Human checkpoint

### 1. 安装修复前旧版本

安装一个不包含 CRLF/LF remap 修复的旧版本 `git-ai`，并确保当前 shell 中
`git` / `git-ai` 调用都会走该旧版本。

### 2. 创建干净复现仓库

```bash
mkdir /tmp/git-ai-fail-example
cd /tmp/git-ai-fail-example
git init
git config user.name "Manual Tester"
git config user.email "manual@example.com"
git config core.autocrlf false
git config core.eol lf

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
- 除下一步修改 AI checkpoint blob 的换行符外，不允许手动修改
  `.git/ai/working_logs`。
- 让 Claude Code 自己的 checkpoint 流程自然记录 AI 编辑。

### 4. 修改 AI checkpoint blob 为 CRLF，保持源文件为 LF

Claude Code 生成 `fail_example.py` 后，先不要提交，也不要运行 Human
checkpoint。用 VS Code 手动定位 Claude Code 产生的 AI checkpoint blob，并只修改该
blob 文件的换行符：

1. 在 VS Code 中打开复现仓库目录。
2. 打开 `.git/ai/working_logs/<base_commit>/checkpoints.jsonl`。
3. 找到最后一个 `kind` 为 `AiAgent`，且 `entries[].file` 为
   `fail_example.py` 的 checkpoint。
4. 复制该 entry 的 `blob_sha` 值。
5. 在同一个 `<base_commit>` 目录下打开
   `.git/ai/working_logs/<base_commit>/blobs/<blob_sha>`。
6. 在 VS Code 右下角点击换行符状态，把该 blob 文件从 `LF` 改为
   `CRLF`，然后保存。
7. 打开工作区中的 `fail_example.py`，在 VS Code 右下角确认换行符为
   `LF`；如果不是 `LF`，改成 `LF` 并保存。

要求：

- 对应 `blob_sha` 的 checkpoint blob 在 VS Code 右下角必须显示为 `CRLF`。
- 工作区 `fail_example.py` 在 VS Code 右下角必须显示为 `LF`。
- 只允许修改 `blob_sha` 对应的 blob 文件内容；不要修改
  `checkpoints.jsonl` 中的 `blob_sha`、`attributions` 或
  `line_attributions`。

这一步是复现关键：checkpoint 记录仍然指向 Claude Code 的 AI attribution，但
checkpoint blob 使用 CRLF，工作区源代码使用 LF。旧版本在后续 Human checkpoint
中基于这两个内容计算 diff 时，可能因为换行符不一致，把 AI 行错误计算成 human。

### 5. 执行普通 Human checkpoint，模拟错误后置 checkpoint

Claude Code 生成 `fail_example.py` 后，在同一个仓库中执行：

```bash
git-ai checkpoint -- fail_example.py
```

这一步是复现关键：它模拟提交阶段或人工流程里追加的 Human checkpoint。在旧版本中，
这个后置 Human checkpoint 可能因为 checkpoint blob 为 CRLF、源文件为 LF，而覆盖
Claude Code 已产生的 AI attribution。

### 6. 提交前检查 checkpoint 状态

```bash
git-ai status
```

要求：

- 输出中应能看到 `fail_example.py` 相关状态。
- 如果完全看不到该文件或 checkpoint 信息，本次复现无效，应重新从干净仓库开始。

### 7. 提交

```bash
git add fail_example.py
git commit -m "add fail example"
```

### 8. 提交后检查失败

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
- 除第 4 步修改 AI checkpoint blob 的换行符外，只能查看 working log 数据，不能再
  修改 checkpoint 文件；否则不再是该换行符不一致问题的真实复现样例。
- 可以再次确认：AI checkpoint blob 内容为 CRLF，而工作区或提交后的
  `fail_example.py` 内容为 LF。

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
git config core.autocrlf false
git config core.eol lf

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
- 除下一步修改 AI checkpoint blob 的换行符外，不允许手动修改
  `.git/ai/working_logs`。

### 4. 按失败样例相同方式制造 CRLF/LF 不一致

重复失败样例第 4 步的 VS Code 手工操作：

- 把 Claude Code 产生的 AI checkpoint blob 内容转换为 CRLF。
- 把工作区 `fail_example.py` 内容转换或保持为 LF。
- 确认 VS Code 右下角显示：checkpoint blob 为 `CRLF`，工作区源文件为
  `LF`。

### 5. 执行同样的后置 Human checkpoint

```bash
git-ai checkpoint -- fail_example.py
```

### 6. 提交前检查 checkpoint 状态

```bash
git-ai status
```

要求：

- 输出中应能看到 `fail_example.py` 相关状态。
- 如果完全看不到该文件或 checkpoint 信息，本次验证无效，应重新从干净仓库开始。

### 7. 提交并验证

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
- 即使 AI checkpoint blob 为 CRLF、源文件为 LF，后置
  `git-ai checkpoint -- fail_example.py` 也没有把 Claude Code 的 AI
  attribution 覆盖掉。

## 注意事项

- 失败样例必须安装旧版本后执行，不能在修复后版本上期待失败。
- 成功样例必须重新创建干净仓库，不能复用失败样例仓库。
- 不要使用 `git-ai checkpoint mock_ai`，因为目标是验证 Claude Code 真实生成代码过程中的
  checkpoint 数据。
- 除本文明确要求修改 AI checkpoint blob 的换行符外，不要人工编辑
  `.git/ai/working_logs`；尤其不要修改 `checkpoints.jsonl`。
- `git-ai status` 是提交前必跑检查；`git stats` 是提交后必跑检查。
