# git-ai 问题专项分析报告

## 1. 报告范围

本报告**仅基于当前 `git-ai` 目录下源码进行分析**，不对外部系统做臆测。

分析对象是之前出现的专项问题：

- 用户运行环境是虚拟机
- 操作系统是 Windows 10 中文版
- 机器整体性能较差
- 磁盘写入 I/O 最高约 `300KB/s`
- 用户使用 Claude 进行 AI 编程
- 使用 `ai-code` 软件统计 AI 编码占比
- 实际执行时经常出现 `ai-code` 后台挂起、僵死、统计遗漏
- 结果是部分 AI 写的代码**没有被计入 AI 编码统计**

本报告关注的不是“异步模式本身好不好”，而是：

**为什么在上述环境里，`git-ai` 会成为 `ai-code` 后台挂起和 AI 统计遗漏的关键风险点。**

---

## 2. 结论摘要

基于源码分析，可以直接得出以下结论：

1. `git-ai` 当前实现对“先 checkpoint，再 commit，再生成 notes/statistics”这条链路依赖很强。
2. 这条链路在 Windows 慢虚拟机环境下，受到 daemon、文件锁、磁盘写入、trace ingest、checkpoint 重写、SQLite WAL 等多重因素影响，容易变慢、积压、超时、异常。
3. 一旦 `git-ai` 的 checkpoint 或 post-commit note 没有及时落盘，`ai-code` 读取到的统计结果就会偏低，直接体现为 AI 编码占比丢失。
4. 这个问题不是单点故障，而是**架构层“最终一致性 + 高 I/O + Windows 兼容性不足 + 超时预算偏紧”叠加形成的系统性问题**。
5. 对你描述的这类机器环境，当前实现更适合“以正确性优先的保守模式”运行；如果继续追求前台速度，必须补齐 barrier、健康检查、daemon 自动恢复、写放大削减等机制。

---

## 3. 与当前问题直接相关的源码事实

### 3.1 Release 默认开启 async/daemon 模式

`src/feature_flags.rs:58`

```rust
async_mode: async_mode, debug = false, release = true,
```

这意味着正式环境默认走异步/daemon 链路，而不是完全同步落盘。

### 3.2 项目自己的 Windows 安装流程会主动关闭 async

`.github/workflows/install-scripts-local.yml:183-184`

```yaml
# Windows does not support Unix-domain sockets for the async daemon.
GIT_AI_ASYNC_MODE: "false"
```

无论这条注释是否完全跟当前 named pipe 实现一致，至少有一个非常明确的事实：

**项目自己的 Windows 端到端流程，在主动规避 Windows async 路径。**

这说明维护者自己也知道 Windows 上的异步后台链路风险更高。

### 3.3 Claude 的 hook 数据会携带明确文件路径

`src/commands/checkpoint_agent/agent_presets.rs:244-248`

```rust
let file_path_as_vec = hook_data
    .get("tool_input")
    .and_then(|ti| ti.get("file_path"))
    .and_then(|v| v.as_str())
```

`src/commands/checkpoint_agent/agent_presets.rs:278-298`

Claude `PreToolUse` 会生成 `will_edit_filepaths`，`PostToolUse` 会生成 `edited_filepaths`。

这会让 `git-ai` 在很多真实 Claude 文件编辑场景里走“**显式文件范围 checkpoint**”路径。

### 3.4 显式文件范围的 checkpoint 会走 delegated/captured 路径

`src/commands/git_ai_handlers.rs:989-990`

```rust
let allow_captured_async =
    checkpoint_request_has_explicit_capture_scope(args, agent_run_result.as_ref());
```

`src/commands/git_ai_handlers.rs:1448-1450`

```rust
if allow_captured_async
    && crate::commands::checkpoint::explicit_capture_target_paths(...)
```

也就是说，Claude 这种“明确知道编辑了哪个文件”的路径，天然更容易进入 captured/delegated 分支。

### 3.5 这条路径在用户层会被标记为 “Checkpoint queued”

`src/commands/git_ai_handlers.rs:1078-1098`

`src/commands/git_ai_handlers.rs:1244-1246`

代码会打印：

- `Checkpoint queued`
- `Checkpoint completed`

`tests/daemon_mode.rs:1257-1262`

测试明确把这条路径称为：

```rust
"explicit-path daemon-mode checkpoint should queue asynchronously"
```

所以从产品语义和测试预期看，**显式文件 checkpoint 被当成“队列化/异步化路径”处理。**

### 3.6 但 daemon 在真正处理 checkpoint 前，还要等待 trace ingest 追平

`src/daemon.rs:7124-7139`

```rust
let ingest_high_watermark = self.trace_ingest_high_watermark();
if ingest_high_watermark > 0 {
    self.wait_for_trace_ingest_processed_through(ingest_high_watermark)
        .await?;
}
```

也就是说，checkpoint 不是简单收下就完事，它前面还要等 trace ingest 队列追平。

### 3.7 项目源码自己承认 trace ingest 在 IDE 场景里可能出现 1 分钟以上积压

`src/daemon.rs:4899`

```rust
with 120–415 trace events/sec and causing >1 min backlog
```

这不是推测，是源码注释里直接写出来的历史问题。

### 3.8 commit 后默认只等 500ms 看 note 是否出现

`src/commands/git_handlers.rs:762`

```rust
std::time::Duration::from_millis(500)
```

`src/commands/git_handlers.rs:780-781`

```rust
"[git-ai] still processing commit {}... run `git ai stats` to see stats."
```

这说明：

- commit 返回时，统计结果并不一定已经稳定
- note 可能还在后台处理中
- 如果 `ai-code` 在这一刻立即读统计，就有读到旧值的风险

### 3.9 Windows git 代理本身就允许 20 秒阻塞，并且只重试 1 次

`src/commands/git_handlers.rs:48`

```rust
const DEFAULT_GIT_PROXY_TIMEOUT: Duration = Duration::from_secs(20);
```

`src/commands/git_handlers.rs:51`

```rust
const DEFAULT_GIT_PROXY_RETRY_COUNT: usize = 1;
```

这意味着 Windows 上一次 git 代理异常，用户侧可能感知到的是：

- 先卡 20 秒
- 再重试
- 再卡一轮

从 `ai-code` 后台视角看，很容易被感知成“挂住了”。

### 3.10 daemon wrapper state 等待预算只有 750ms

`src/daemon.rs:7323-7345`

```rust
"git-ai: wrapper state timeout ..."
Duration::from_millis(750)
```

在慢虚拟机里，这个预算明显偏紧，很容易退回 fallback 路径。

### 3.11 checkpoint 路径本身存在明显的写放大

#### a. 保存当前文件状态时，会为每个文件写 blob

`src/commands/checkpoint.rs:1531`

`src/commands/checkpoint.rs:1583-1587`

```rust
std::fs::create_dir_all(&*blobs_dir)?;
std::fs::write(blob_path, content)?;
```

#### b. captured checkpoint 还会额外写一套 blobs 和 manifest

`src/commands/checkpoint.rs:1111-1112`

```rust
fs::create_dir_all(&capture_dir)?;
fs::create_dir_all(capture_dir.join("blobs"))?;
```

`src/commands/checkpoint.rs:1126`

```rust
fs::write(capture_dir.join("blobs").join(&blob_name), content)?;
```

`src/commands/checkpoint.rs:1156-1158`

```rust
async_checkpoint_manifest_path(&capture_id)?,
serde_json::to_vec(&manifest)?,
```

在你这种 `300KB/s` 写入上限环境里，这种写法非常容易积压。

### 3.12 working log 不是 append-only，而是“读全量再整文件重写”

`src/git/repo_storage.rs:390-452`

```rust
let mut checkpoints = self.read_all_checkpoints().unwrap_or_default();
checkpoints.push(storage_checkpoint);
self.write_all_checkpoints(&checkpoints)
```

`src/git/repo_storage.rs:564-579`

```rust
pub fn write_all_checkpoints(...)
fs::write(&checkpoints_file, format!("{}\n", content))?;
```

这意味着每增加一个 checkpoint，都不是轻量追加，而是：

- 读完整个 `checkpoints.jsonl`
- 重新序列化
- 整个文件重写

checkpoint 越多，慢盘上越慢。

### 3.13 SQLite 也使用 WAL

`src/authorship/internal_db.rs:336-339`

`src/metrics/db.rs:79-82`

```rust
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA temp_store=MEMORY;
```

WAL 在普通环境常常是合理优化，但在极低 I/O、虚拟磁盘、频繁小写场景下，也会加重额外写入和锁竞争。

### 3.14 commit replay 只对 bash in-flight 有 AI 恢复逻辑

`src/daemon.rs:2551-2579`

```rust
match crate::commands::checkpoint_agent::bash_tool::checkpoint_context_from_active_bash(...)
```

如果没有活跃 bash 上下文，就会走：

```rust
(CheckpointKind::Human, result)
```

这意味着：

**一旦正常的 Claude 文件编辑 checkpoint 没有成功进入 working log，commit 阶段并没有通用的“恢复 Claude AI 上下文”机制。**

结果就是：这次代码最终更容易被当成 Human，而不是 AI。

### 3.15 Windows 文件锁实现曾存在底层资源释放问题

`src/utils.rs:277`

```rust
const FILE_SHARE_NONE: u32 = 0u32;
```

Windows 锁打开时直接使用独占共享模式。

本地已修复的代码位于：

`src/utils.rs:240-263`

```rust
fn CloseHandle(hObject: isize) -> i32;
unsafe { UnlockFile(...) };
let _ = CloseHandle(self.handle);
```

这个修复的意义是：

- 避免锁对象 drop 后只解锁不关句柄
- 降低 Windows 上锁状态残留、伪死锁、daemon 启动阻塞的概率

这不是唯一问题，但确实是一个明确的底层风险点。

---

## 4. 之前问题的根因分析

结合上面的源码事实，可以把根因归纳为 5 类。

### 4.1 根因一：统计链路依赖 checkpoint 先成功落盘，缺少通用补偿机制

当前 `git-ai` 的 AI 归因链路是：

1. agent hook 产出 `edited_filepaths` / `will_edit_filepaths`
2. `git-ai checkpoint` 写入 `.git/ai/working_logs/...`
3. commit 后根据 working log 生成 authorship note
4. `git-ai stats` / `ai-code` 读取 note 或相关统计结果

问题在于：

- 只要第 2 步没完成
- 后面就容易全链路丢失这次 AI 归因

而 commit replay 的补偿机制目前主要覆盖 bash in-flight，不是通用 Claude edit 恢复。

所以这条链路的脆弱点非常明显：

**checkpoint 一丢，统计大概率跟着丢。**

### 4.2 根因二：慢虚拟机场景下 I/O 放大非常严重

当前实现至少有这些高频写操作：

- 每文件 blob 写入
- captured checkpoint 的二次 blob 写入
- manifest 写入
- checkpoints.jsonl 整文件重写
- internal DB / metrics DB WAL 写入

在 `300KB/s` 这种写入条件下，这些操作的组合足以导致：

- checkpoint 处理时间显著增长
- daemon side-effect 执行时间增长
- 队列更容易堆积
- ai-code 调用线程更容易长期等待

### 4.3 根因三：Windows 下 daemon/lock/timeout 预算不适合低性能虚拟机

目前 Windows 相关预算明显偏激进或不协调：

- git proxy 默认 20 秒超时，用户感知卡顿重
- wrapper state 只有 750ms
- post-commit note 默认只等 500ms
- checkpoint 虽然有较长锁等待，但 daemon 前置 trace ingest 还有独立等待路径

这会形成一个典型问题：

- 某些步骤预算太短，容易 fallback 或提前返回旧状态
- 某些步骤预算又太长，用户感知成“挂死”

最终 `ai-code` 看到的表现不是“稳定慢”，而是“有时卡住，有时漏算，有时恢复”。

### 4.4 根因四：`ai-code` 如果没有等待统计稳定，就会读到旧结果

`git-ai` 自己已经在 commit 后打印过：

```text
still processing commit
```

这代表它本身就是最终一致性而不是强一致性。

所以如果 `ai-code` 的统计逻辑是：

- 编辑后立刻读
- commit 后立刻读
- daemon 请求返回后立刻读

那么在慢机上就很容易出现：

- 代码已经写了
- 但 note 还没生成
- working log 还没稳定
- 最终读到的 AI 占比偏低

这不一定是 `ai-code` 自己的 bug，但它**必须为 `git-ai` 的最终一致性特性做兼容**。

### 4.5 根因五：Windows 锁资源释放问题放大了僵死概率

这类问题在好机器上可能只是偶发现象，在慢虚拟机里会被显著放大：

- daemon lock 拿不到
- flush lock、checkpoint lock、side-effect 路径被拖慢
- 后台进程残留句柄
- 新一轮请求继续阻塞

这会直接加重你观察到的“ai-code 后台挂起/僵死”的体感。

---

## 5. 为什么最终会导致 AI 编码占比漏统

把上面的机制串起来，问题就很清楚了。

### 场景链路

1. 用户用 Claude 编辑文件
2. hook 触发 `git-ai checkpoint`
3. `git-ai` 进入 daemon/scoped checkpoint 路径
4. daemon 要等待 trace ingest、锁、I/O 落盘
5. 在慢虚拟机上，这一步可能非常慢，甚至卡住或异常
6. 如果 checkpoint 没成功落入 working log，后续 commit 统计就缺失关键 AI 上下文
7. commit 后 note 又不保证立即可见
8. `ai-code` 这时去读统计，就会得到偏低结果

### 最终表现

- AI 实际写了代码
- 但 `git-ai` 没能稳定完成归因
- `ai-code` 读取时机又早于最终一致性完成
- 于是这部分代码没有计入 AI 统计

这正是你描述的问题本质。

---

## 6. 详细解决方案

解决方案要分成三层：

1. 立即止血
2. 代码级修复
3. 架构级优化

### 6.1 立即止血方案

这部分是**面向当前用户环境，优先保证统计正确性**。

#### 方案 A：在这类 Windows 慢虚拟机里，优先关闭 async

这是最现实、最稳妥的方案。

```powershell
git-ai config set feature_flags.async_mode false
git-ai daemon shutdown --hard
setx GIT_AI_SLOW_VM 1
setx GIT_AI_CHECKPOINT_LOCK_TIMEOUT_MS 600000
setx GIT_AI_GIT_PROXY_TIMEOUT_MS 120000
setx GIT_AI_GIT_PROXY_RETRY_COUNT 2
setx GIT_AI_POST_COMMIT_TIMEOUT_MS 30000
```

说明：

- `async_mode=false`：优先保证归因正确性，减少 daemon 风险
- `GIT_AI_SLOW_VM=1`：让 checkpoint 锁预算走慢机兜底逻辑
- 放大 git proxy timeout/retry：减少用户侧误判为“卡死”
- 放大 post-commit timeout：减少 commit 后立即读取旧结果

对这类机器，**正确性应该优先于前台速度**。

#### 方案 B：如果必须保留 async，`ai-code` 必须加 barrier

`ai-code` 不能再“调用完就立即读统计”，必须等待 `git-ai` 真正稳定。

建议 barrier 逻辑最少做到：

1. commit 完成后，不立刻读取 AI 占比
2. 轮询 `git notes --ref=ai show HEAD`
3. 只有 note 已存在时，才刷新统计
4. 超时则标记“统计结果暂未完成”，而不是直接记成 0

示例 PowerShell：

```powershell
function Wait-GitAiAuthorship {
    param(
        [string]$RepoPath = ".",
        [int]$TimeoutSeconds = 60
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        git -C $RepoPath notes --ref=ai show HEAD *> $null
        if ($LASTEXITCODE -eq 0) {
            return $true
        }
        Start-Sleep -Milliseconds 500
    }
    return $false
}
```

`ai-code` 应改成：

- `git commit`
- `Wait-GitAiAuthorship`
- 成功后再刷新 AI 占比

如果超时：

- 显示“git-ai 统计仍在处理中”
- 不要直接把这部分代码算成非 AI

#### 方案 C：把 daemon/internal 数据迁到更快磁盘

```powershell
setx GIT_AI_DAEMON_HOME R:\git-ai-home
```

这能减轻：

- captured checkpoint
- internal db
- metrics db
- daemon logs

的写入压力。

注意：

- `.git/ai/working_logs` 仍然在仓库目录里
- 如果仓库也在最慢的虚拟磁盘，问题仍然会存在

所以更理想的做法是：

- `GIT_AI_DAEMON_HOME` 放快盘
- 仓库工作目录也尽量放快盘

---

### 6.2 代码级修复方案

这部分是对 `git-ai` 本身的修复建议。

#### 修复 1：Windows 下显式 scoped checkpoint 强制走同步完成路径

当前 Claude 常见路径会进入 explicit capture scope。

建议规则：

- Windows + `GIT_AI_SLOW_VM=1`
- 或 Windows + 虚拟机识别
- 或 Windows + 明确文件路径 checkpoint

满足任一条件时：

- 不要 `wait: false`
- 强制 `wait: true`
- 前台拿到“真正完成”的结果再返回

这会牺牲部分速度，但能显著提升统计完整性。

#### 修复 2：为 queued checkpoint 返回可等待的 seq，并增加 barrier 命令

当前 `FamilyStatus` 已经有 `applied_seq`，`status.family` 也可查询。

建议新增：

- checkpoint 请求即使是 queued，也返回排队的 `seq`
- 增加：

```text
git-ai daemon barrier --repo <path> --seq <n>
```

语义：

- 等待 daemon family 的 `applied_seq >= n`
- 成功后再允许上层读取统计

这能从根本上解决 `ai-code` 没法判断“统计到底算完没”的问题。

#### 修复 3：让 wrapper/post-commit timeout 支持 slow-vm 配置

当前这些预算太死：

- wrapper state 750ms
- post-commit note 500ms

建议增加环境变量：

- `GIT_AI_WRAPPER_STATE_TIMEOUT_MS`
- `GIT_AI_POST_COMMIT_TIMEOUT_MS`

并在 `GIT_AI_SLOW_VM=1` 时自动放大默认值。

#### 修复 4：补齐非 bash 的 AI 上下文恢复能力

当前 commit replay 对 bash in-flight 有专门恢复。

但普通 Claude 文件编辑如果 checkpoint 失败，就容易直接掉到 Human。

建议新增：

- 最近一次 AI tool-use 上下文缓存
- 对 Claude/Codex/Cursor 等普通文件编辑都可恢复
- 当 commit 到来但 checkpoint 缺失时，允许按最近有效 AI 上下文做补偿重建

这能显著减少“checkpoint 一丢，整次 AI 归因全丢”的问题。

#### 修复 5：增加 daemon stuck watchdog

建议增加以下检测：

- control socket 可连但 family `applied_seq` 长时间不推进
- checkpoint side effect 长时间无进展
- trace ingest high watermark 长时间不下降
- lock 长时间占用

触发后自动：

1. 记录错误
2. 尝试优雅 shutdown
3. 不成功则 `taskkill /F /T`
4. 自动重启 daemon

---

### 6.3 架构级优化方案

这部分是长期方案，用来真正解决“慢盘环境下的系统性卡顿”。

#### 优化 1：working log 改为 append-only

当前 `append_checkpoint()` 是：

- 读全量
- push
- 整文件重写

建议改成：

- checkpoint 真正 append-only 写 JSONL
- 后台 compact 单独做

这样可以显著降低每次 checkpoint 的写入放大。

#### 优化 2：blob 已存在时不要重复写

当前每次都算 sha，再直接写 blob 文件。

建议改成：

- 若 `blobs/<sha>` 已存在，则跳过写入

这样对重复内容、频繁编辑会减轻大量磁盘写入。

#### 优化 3：captured checkpoint 与 working log blob 复用

当前 captured checkpoint 和正式 working log 都会各写一份 blob。

建议统一 blob 存储层，避免双份写入。

#### 优化 4：慢机模式下调低 SQLite 写入压力

建议引入 slow-vm profile：

- 降低 metrics 刷新频率
- 非关键 telemetry 可以延后
- internal DB 尽量合批写入
- 必要时为慢机切换更保守的 journal 策略

#### 优化 5：Windows 单独特化运行模式

建议不要把 Windows 与 Unix 共用同一套 async 假设。

Windows 上可以直接定义：

- daemon 仅负责可恢复的后台任务
- 关键归因任务同步完成
- 统计正确性优先于响应速度

---

## 7. 推荐执行顺序

如果现在要解决你描述的现场问题，建议按下面顺序推进。

### 第一步：先保证现场数据不再继续漏

立即执行：

```powershell
git-ai config set feature_flags.async_mode false
git-ai daemon shutdown --hard
setx GIT_AI_SLOW_VM 1
setx GIT_AI_CHECKPOINT_LOCK_TIMEOUT_MS 600000
setx GIT_AI_GIT_PROXY_TIMEOUT_MS 120000
setx GIT_AI_GIT_PROXY_RETRY_COUNT 2
setx GIT_AI_POST_COMMIT_TIMEOUT_MS 30000
```

### 第二步：修改 `ai-code` 统计刷新逻辑

必须等待：

- `git notes --ref=ai show HEAD` 可读
- 或者未来新增的 `git-ai daemon barrier`

在此之前不要直接刷新 AI 占比。

### 第三步：合入当前已确认的底层修复

已在本地确认并修改：

- `src/utils.rs`
- Windows `LockFile` drop 时增加 `CloseHandle`
- Unix `LockFile` drop 时补 `libc::close`

这项修复应该尽快合入。

### 第四步：对 Windows 慢机增加强制同步策略

优先改：

- 显式 scoped checkpoint 在 Windows 慢机上强制 `wait=true`
- 或直接在 Windows release 默认 `async_mode=false`

### 第五步：做 barrier + working log append-only 改造

这是从根上解决“卡顿 + 统计遗漏”的关键工程项。

---

## 8. 最终结论

针对之前的问题，可以给出最终判断：

1. 问题的直接表现是 `ai-code` 后台挂起、僵死，以及 AI 编码占比漏统。
2. 从 `git-ai` 源码看，根本原因是 `git-ai` 当前统计链路对 checkpoint 和 post-commit note 依赖极强，而这条链路在 Windows 慢虚拟机里存在明显的性能和稳定性缺陷。
3. 这些缺陷主要来自：
   - daemon/async 最终一致性
   - trace ingest 积压
   - 高 I/O 写放大
   - working log 整文件重写
   - SQLite WAL
   - Windows timeout 与锁问题
   - 非 bash 场景缺少 AI 上下文补偿
4. 一旦 checkpoint 或 note 没有按时完成，`ai-code` 就会读取到旧统计，导致 AI 代码没有计入 AI 占比。
5. 对当前这类 Windows 10 中文版慢虚拟机场景，最现实的方案是：
   - 先关闭 async，优先保证正确性
   - 同时让 `ai-code` 增加 barrier，禁止在统计链路未稳定时读取结果
6. 从长期看，必须对 `git-ai` 做 Windows 特化、barrier 支持、append-only working log 和写放大削减，否则这类问题会反复出现。
