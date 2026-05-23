# git-ai 目录专项分析报告

## 1. 分析范围

本报告只基于当前 `D:\ai-code-statistics-alpha\git-ai` 目录下源码进行分析，不对外部系统做超出代码证据的推断。

分析目标是下面这个具体问题：

- 用户运行环境为虚拟机
- 系统为 Windows 10 中文版
- 机器性能较差
- 磁盘写入 I/O 最高约 `300KB/s`
- 用户通过 Claude 进行 AI 编程
- `ai-code` 软件负责统计 AI 编码占比
- 实际使用中经常出现 `ai-code` 后台挂起、僵死
- 结果是部分 AI 改动没有进入最终统计

本报告不讨论“异步模式抽象上是否先进”，而是回答：

**在上述机器条件下，为什么 `git-ai` 当前实现会放大后台挂起和 AI 统计漏记问题，以及应该如何修复。**

---

## 2. 结论摘要

基于源码可以得出四个直接结论：

1. `git-ai` 的 AI 占比统计依赖一条严格链路：`checkpoint -> working log -> post-commit -> refs/notes/ai -> stats`。
2. 在 Windows 慢虚拟机环境下，这条链路存在多个易失点：bash hook 超时、daemon 排队、trace ingest 等待、整文件重写、磁盘写放大、最终 note 可见性延迟。
3. 当前实现里不少失败会被“吞掉”或以 `exit(0)` 结束，导致上层 `ai-code` 很难知道 checkpoint 实际失败，因此不会自动补救。
4. 所以你看到的不是“统计计算错误”，而是 **AI 改动没有稳定进入采集链路**，最终表现为 AI 编码占比偏低。

换句话说：

**问题根因是采集不可靠，不是统计公式不可靠。**

---

## 3. 统计链路在代码里的真实落点

### 3.1 统计前提：AI 改动必须先写进 working log

`checkpoint` 最终会把本次改动转成 `Checkpoint` 并写入 `.git/ai/working_logs/<base_commit>/checkpoints.jsonl`。

关键代码：

- `src/commands/checkpoint.rs`
- `src/git/repo_storage.rs`

其中：

- `execute_resolved_checkpoint()` 负责构建 checkpoint
- `working_log.append_checkpoint(&checkpoint)` 负责持久化

### 3.2 commit 时再把 working log 转成 authorship note

提交后，`post_commit()` 会读取 working log，生成 `AuthorshipLog`，再写入 `refs/notes/ai`。

关键代码：

- `src/authorship/post_commit.rs:74`
- `src/authorship/post_commit.rs:230`

也就是说，最终统计不是直接看临时 checkpoint，而是看 commit 对应的 AI note。

### 3.3 最终统计读的是 note，不是 hook 过程本身

`stats_for_commit_stats()` 会去读 `refs/notes/ai`，而不是直接重放 hook。

关键代码：

- `src/authorship/stats.rs`
- `src/git/refs.rs`

因此只要 note 没有及时生成，`ai-code` 读到的结果就会偏低。

---

## 4. 为什么会漏记：六个直接原因

## 4.1 bash 工具链路很容易在慢机上直接 fallback

Claude 的文件编辑和 bash 调用会进入 `checkpoint_agent/bash_tool.rs` 这套快照机制。

这里的预算非常紧：

- `WALK_TIMEOUT_MS = 1500`
- `HOOK_TIMEOUT_MS = 4000`

关键代码：

- `src/commands/checkpoint_agent/bash_tool.rs:37`
- `src/commands/checkpoint_agent/bash_tool.rs:41`

在慢虚拟机和低磁盘写入下，扫描工作区、构造快照、读写缓存文件，很容易超过这两个阈值。

一旦超时，代码会直接进入：

- `BashCheckpointAction::Fallback`

而 Claude preset 对这个结果的处理是：

- 直接把 `edited_filepaths` 记为 `None`
- 不报硬错误

关键代码：

- `src/commands/checkpoint_agent/agent_presets.rs:320`
- `src/commands/checkpoint_agent/agent_presets.rs:324`

这意味着：

- 本来应该是“精准文件范围 checkpoint”
- 一旦 fallback，就退化成“没有明确文件路径”
- 后续只能走更重、更慢、也更容易失败的全量路径

这正是慢机下后台越来越容易僵死的第一层原因。

## 4.2 预快照丢了以后，代码明确选择“不补救”

bash 路径依赖 pre/post snapshot 对比。

但当前实现里，如果 post hook 找不到 pre-snapshot，日志会写：

`Pre-snapshot not found ... returning fallback (no git status)`

关键代码：

- `src/commands/checkpoint_agent/bash_tool.rs:1671`

同时，这个文件里其实已经实现了 `git_status_fallback()`：

- `src/commands/checkpoint_agent/bash_tool.rs:917`

但这里没有真正调用。

这意味着一旦：

- 后台挂起
- 进程被重启
- 快照被消费后处理失败
- 快照过期

那么本来可以通过 `git status` 做一次降级恢复的场景，被代码直接放弃了。

这会直接造成一次 AI 改动从统计链路中消失。

## 4.3 失败被静默吞掉，上层看不到真正失败

这是当前实现里最危险的一点。

`git-ai checkpoint` 在多个失败路径上都会 `std::process::exit(0)`，包括：

- preset 解析失败
- checkpoint 执行失败
- 本地 checkpoint 失败

关键代码：

- `src/commands/git_ai_handlers.rs:441`
- `src/commands/git_ai_handlers.rs:458`
- `src/commands/git_ai_handlers.rs:1251`
- `src/commands/git_ai_handlers.rs:1341`

这会导致两个后果：

1. `ai-code` 后台即使收到了失败，进程退出码仍然像“成功”
2. 上层不会把它识别成需要重试的硬错误

所以用户体感是：

- 软件后台卡了一下
- 之后继续能用
- 但统计悄悄少了一截

这不是巧合，而是代码行为决定的。

## 4.4 working log 的持久化方式对慢盘非常不友好

当前 `append_checkpoint()` 不是追加写，而是：

1. 读完整个 `checkpoints.jsonl`
2. 反序列化
3. 把新 checkpoint 加进去
4. 再把整个文件完整重写

关键代码：

- `src/git/repo_storage.rs:390`
- `src/git/repo_storage.rs:452`
- `src/git/repo_storage.rs:564`
- `src/git/repo_storage.rs:577`

也就是说，checkpoint 越多，单次写入越重。

对你描述的环境，这会带来三个放大效应：

- 小改动也会触发整文件重写
- 同一段工作流里 checkpoint 越频繁，后面越慢
- 一旦写到一半进程挂住，损坏概率更高

还有一个更糟的点：

`append_checkpoint()` 读取失败时用的是：

`read_all_checkpoints().unwrap_or_default()`

关键代码：

- `src/git/repo_storage.rs:392`

也就是：

- 如果旧文件损坏或暂时读失败
- 新流程会把它当成“原来没有 checkpoint”

然后再把新内容完整覆盖写回去。

这会让历史 checkpoint 直接消失。

## 4.5 checkpoint 本身存在明显写放大

当前 checkpoint 会做很多磁盘写入：

- 为每个文件保存 blob
- captured checkpoint 再额外写一套 blobs
- 另外写 manifest

关键代码：

- `src/commands/checkpoint.rs:1111`
- `src/commands/checkpoint.rs:1112`
- `src/commands/checkpoint.rs:1126`
- `src/commands/checkpoint.rs:1156`

同时还存在并发处理：

- 保存当前文件状态：并发 `8`
- 文件归因任务：并发 `30`

关键代码：

- `src/commands/checkpoint.rs:1544`
- `src/commands/checkpoint.rs:2016`

在高性能 SSD 上，这种设计主要是快。

但在 `300KB/s` 的盘上，这会变成反效果：

- 并发不是提速，而是放大随机 I/O
- 后台线程不是吞吐更高，而是更容易互相阻塞
- 一旦同时碰上杀软、虚拟磁盘、Windows 文件锁，卡顿会被进一步放大

## 4.6 daemon 路径在慢机上存在时序错位

release 默认开启 `async_mode`：

- `src/feature_flags.rs:58`

而 Windows 安装流程里却显式把它关掉：

- `.github/workflows/install-scripts-local.yml:183`

这说明项目本身已经意识到 Windows 异步路径风险更高。

当前 daemon 还有三个关键时序问题：

### a. checkpoint 前要等 trace ingest 追平

- `src/daemon.rs:7138`

这意味着 checkpoint 不是“收到就执行”，而是还要排队等前面的 trace 数据先消化。

### b. wrapper state 等待只有 750ms

- `src/daemon.rs:7323`
- `src/daemon.rs:7345`

慢机上很容易超时，退回内部状态估算。

### c. commit 后默认只等 500ms 看 authorship note 是否出现

- `src/commands/git_handlers.rs:762`

如果 `ai-code` 在这之前就去读统计，那么它看到的就是旧值。

所以在当前实现里，完全可能发生下面这个序列：

1. 用户完成 AI 改动
2. checkpoint 还在后台排队
3. commit 已经返回
4. `ai-code` 立刻读统计
5. note 还没生成
6. 本次 AI 改动暂时不在统计里

如果之后后台又挂起，这个“暂时没有”就会变成“永久漏记”。

---

## 5. 为什么用户会感觉是“ai-code 后台挂起”

从源码看，用户感知成“后台挂起/僵死”并不奇怪。

原因有三层：

### 5.1 git 代理本身在 Windows 上就允许长时间等待

关键代码：

- `src/commands/git_handlers.rs:48`
- `src/commands/git_handlers.rs:50`
- `src/commands/git_handlers.rs:1064`

默认：

- 超时 `20s`
- 重试 `1` 次

也就是最糟情况下，一个 git 代理调用会卡很久。

### 5.2 daemon 启动依赖 Windows `Start-Process`

关键代码：

- `src/commands/daemon.rs:280`

在慢虚拟机里，启动后台进程、建立 named pipe、等待 socket ready，这些都可能抖动。

### 5.3 named pipe 控制通道有严格超时预算

关键代码：

- `src/daemon.rs:88`
- `src/daemon.rs:89`
- `src/daemon.rs:8040`

控制请求和 checkpoint 请求虽然有预算，但这些预算不是慢机友好型设计。

结果就是：

- 前台感知是“卡住”
- 后台感知是“还没处理完”
- 统计侧感知是“这部分改动还没被确认”

---

## 6. 为什么有时漏、有时不漏

这不是随机现象，而是当前实现里本来就有一个时间窗。

pre-commit / commit replay 路径会尝试读取仍然存活的 bash inflight snapshot，把本次 commit 归因成 AI。

关键代码：

- `src/authorship/pre_commit.rs:29`
- `src/daemon.rs:2551`

但 snapshot 的保鲜时间只有：

- `SNAPSHOT_STALE_SECS = 300`

关键代码：

- `src/commands/checkpoint_agent/bash_tool.rs:44`

所以：

- 如果后台只是短暂抖动，commit 时快照还在，可能还能补回来
- 如果后台挂太久，快照过期了，这条补救链路就断了

这正好对应你描述的现象：

- 不是每次都漏
- 但一旦后台僵死时间长一点，就明显漏

---

## 7. 最终根因归纳

把上面的代码事实合并起来，根因可以归纳为四条：

### 根因一：采集链路过长

AI 改动不是立刻进入最终统计，而是必须依次经过：

- hook
- checkpoint
- working log
- post-commit
- note
- stats

链路越长，慢机上失败点越多。

### 根因二：慢盘上写放大过重

当前设计里有大量：

- blob 写入
- manifest 写入
- `checkpoints.jsonl` 全量重写
- SQLite WAL 写入

这些在快盘上可接受，在你这类机器上风险极高。

### 根因三：超时和回退策略偏“保交互”，不保统计完整性

比如：

- bash hook 超时后直接 fallback
- pre-snapshot 丢失后不调用 `git_status_fallback`
- wrapper state 750ms 就超时
- commit 后只等 500ms 看 note

这类策略对“前台尽快返回”友好，但对“统计一定别漏”不友好。

### 根因四：错误传播太弱

失败经常被吞掉并以 `exit(0)` 结束，导致上层无法知道需要重试。

这是“漏记会长期存在”的决定性原因。

---

## 8. 详细解决办法

下面给出一个按优先级排列的解决方案，分为：

- 立即止血
- 代码修复
- 慢机专项优化
- 验证方案

## 8.1 立即止血方案

目标：先把“漏记概率”压下去，不追求最漂亮的架构。

### 方案 A：在这台机器上关闭 async_mode

执行：

```powershell
git-ai config set feature_flags.async_mode false
```

原因不是“异步一定不好”，而是当前 Windows 慢机环境下，daemon 排队和 note 可见性延迟会放大漏记概率。

这是最直接的止血手段。

### 方案 B：打开慢机兜底环境变量

执行：

```powershell
setx GIT_AI_SLOW_VM 1
setx GIT_AI_GIT_PROXY_TIMEOUT_MS 60000
setx GIT_AI_GIT_PROXY_RETRY_COUNT 0
```

其中：

- `GIT_AI_SLOW_VM=1` 当前至少会放大 checkpoint 锁预算
- git proxy timeout 增加到 60s，减少误杀
- retry 设为 0，避免“超时后再重试一次”把总卡顿继续放大

### 方案 C：给仓库补 `.git-ai-ignore`

至少排除：

- `target/`
- `node_modules/`
- `dist/`
- `build/`
- `.next/`
- `coverage/`
- 大型缓存目录
- 编译输出目录

这样能直接减轻：

- bash snapshot walk
- checkpoint 文件扫描
- git status / repo status 范围

### 方案 D：操作规范上禁止“挂住后直接 commit”

如果用户发现 Claude / ai-code 后台已经明显挂住，不要直接提交。

应先：

1. 恢复或重启后台
2. 等本次 checkpoint 完成
3. 再执行 commit

否则超过 300 秒后，快照可能过期，commit 兜底也会失效。

---

## 8.2 必须做的代码修复

这些属于 P0，不修的话问题会反复出现。

### 修复 1：checkpoint 真实失败必须返回非 0

当前所有“真正影响统计完整性”的失败，都不应继续 `exit(0)`。

至少下面两类要改：

- preset 解析失败
- checkpoint 执行失败

否则上层永远无法可靠重试。

### 修复 2：pre-snapshot 不要在读取后立刻删除

当前 `load_and_consume_snapshot()` 是：

1. 读文件
2. 立刻删文件
3. 再继续处理

关键代码：

- `src/commands/checkpoint_agent/bash_tool.rs:852`

更安全的做法应该是：

1. 读文件
2. 尝试处理
3. 处理成功后再删除
4. 处理失败时保留快照供 retry / commit replay 使用

这能显著降低“消费成功但后续处理失败，结果快照也丢了”的问题。

### 修复 3：post-hook 丢失快照时必须启用 `git_status_fallback()`

当前已经有实现，但没接上。

应改成：

- `pre-snapshot` 丢失时，先尝试 `git_status_fallback()`
- 只有 fallback 也失败时，才返回空结果

这是最重要的降级恢复点之一。

### 修复 4：去掉 `append_checkpoint()` 的 `unwrap_or_default()`

当前旧 checkpoint 读失败会被当成空数组。

这会造成：

- 文件损坏后继续覆盖写
- 旧 checkpoint 丢失

应改为：

- 读失败直接报错
- 保留损坏文件
- 写入 `.corrupt` 或 `.bak`
- 阻止继续覆盖原文件

### 修复 5：`write_all_checkpoints()` 改成原子写

当前是直接 `fs::write()` 覆盖。

应改成：

1. 写 `checkpoints.jsonl.tmp`
2. flush
3. fsync
4. rename 覆盖原文件

这是 Windows 慢盘环境下最基本的完整性保护。

---

## 8.3 慢机专项优化

这些属于 P1，会明显降低挂起概率。

### 优化 1：降低 checkpoint 并发

当前并发：

- 保存 blob：`8`
- 文件归因：`30`

对慢盘应提供配置化降并发，比如：

- blob 保存并发：`1~2`
- 文件归因并发：`2~4`

否则大量小文件 I/O 会互相争抢。

### 优化 2：`checkpoints.jsonl` 改成 append-only

现状是每次全量重写。

更合理的方式是：

- 正常路径只 append
- 定期 compact
- compact 放后台

这能显著减少 checkpoint 写放大。

### 优化 3：把 `GIT_AI_SLOW_VM` 扩展到更多超时点

当前只看到 checkpoint lock 对它有响应：

- `src/commands/checkpoint.rs:142`

应把它扩展到：

- bash hook timeout
- walk timeout
- wrapper state timeout
- post-commit note wait timeout
- daemon startup timeout

否则“慢机模式”实际上只覆盖了一小段链路。

### 优化 4：让 post-commit note 可见性等待可配置并默认更宽松

当前只等 500ms。

对慢 Windows VM，建议默认提升到至少：

- `3000ms ~ 5000ms`

或者在 `GIT_AI_SLOW_VM=1` 时自动放宽。

---

## 8.4 验证方案

修复后不能只看 UI，需要用可复现流程验证。

### 验证场景 1：Claude 普通文件编辑

步骤：

1. Claude 用 `Edit/Write` 改一个文件
2. 执行 commit
3. 验证 `refs/notes/ai` 已生成
4. 验证统计包含本次 AI 改动

### 验证场景 2：Claude Bash 改文件

步骤：

1. Claude 通过 bash 改文件
2. 期间故意制造慢盘环境
3. 执行 commit
4. 验证即使 bash snapshot 丢失，`git_status_fallback()` 也能补救

### 验证场景 3：后台挂住 6 分钟后再提交

这是最关键的回归测试。

步骤：

1. Claude bash 改文件
2. 模拟后台挂起超过 `300s`
3. 恢复后执行 commit
4. 验证仍然不会把这次 AI 改动完全漏掉

### 验证场景 4：损坏 `checkpoints.jsonl`

步骤：

1. 人为打断一次 checkpoint 写入
2. 再触发新 checkpoint
3. 验证不会把旧 checkpoint 静默抹掉

---

## 9. 最终建议

如果目标是“先把统计做准”，建议按这个顺序执行：

1. 立即关闭 `async_mode`
2. 打开 `GIT_AI_SLOW_VM=1`
3. 给仓库补 `.git-ai-ignore`
4. 修复 `exit(0)` 吞错问题
5. 修复 pre-snapshot 丢失后不做 `git_status_fallback()` 的问题
6. 修复 `checkpoints.jsonl` 的非原子整文件重写

如果只允许先做两件事，优先级必须是：

1. **失败返回非 0**
2. **pre-snapshot 丢失时启用 `git_status_fallback()`**

因为这两项直接决定：

- 上层能不能知道失败
- 失败后还有没有补救机会

---

## 10. 一句话总结

当前 `git-ai` 在你描述的 Windows 慢虚拟机场景下，最大问题不是“不会统计”，而是：

**它会在 checkpoint 采集阶段因为超时、回退、静默失败和重写放大，导致一部分 AI 改动根本没有稳定进入最终统计链路。**

这才是 `ai-code` AI 编码占比经常偏低的根本原因。
