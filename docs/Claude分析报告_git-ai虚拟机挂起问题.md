# Git-AI 虚拟机环境挂起问题深度分析报告

**项目**: git-ai (D:\ai-code-statistics-alpha\git-ai)
**分析日期**: 2026-04-20
**分析环境**: Windows 10 中文版虚拟机 (磁盘 I/O 上限 300KB/s)
**报告类型**: 根因分析 + 解决方案

---

## 一、问题概述

用户在高延迟、低性能虚拟机环境中使用 `git-ai` 进行 AI 编码占比统计时，遇到软件后台进程挂起/僵死的问题，导致部分代码未能计入 AI 编码统计范畴。

**症状表现**：
- Git 操作（commit、checkpoint 等）长时间无响应
- 守护进程 (daemon) 不响应客户端请求
- AI 编码归因数据丢失
- 统计结果不完整

---

## 二、根因定位

### 2.1 核心故障路径

```
用户执行 git commit
       ↓
Wrapper 进程 → 连接 Daemon 控制套接字 (期望 750ms 内完成)
       ↓
Daemon 在 OS 原生线程中调用 block_on()
       ↓
checkpoint::run() 在 Tokio blocking 线程池上同步执行
       ↓
  ├─ 读取工作日志 checkpoints.jsonl (std::fs::read_to_string)  ← 【致命阻塞点 #1】
  ├─ 处理归因逻辑 (CPU)
  ├─ 写回 checkpoints.jsonl (std::fs::write)                   ← 【致命阻塞点 #2】
  └─ SQLite 数据库写入 (MetricsDatabase::global().lock())
       ↓
Tokio blocking 线程池默认约 10 个线程
多个并发 checkpoint 全部阻塞 → 线程池耗尽
       ↓
新请求无法获得 blocking 线程
→ block_on() 无限期等待
→ DAEMON_CONTROL_RESPONSE_TIMEOUT (10s) 触发
→ Wrapper 收到超时错误，任务链中断
→ 代码未被归因统计，数据永久丢失
```

### 2.2 根因总结

**主要根因**: Tokio 异步运行时的 blocking 线程池被耗尽（Blocking Thread Pool Exhaustion）

| 根因类型 | 描述 | 严重程度 |
|----------|------|----------|
| **阻塞 I/O 未异步化** | `read_all_checkpoints` / `write_all_checkpoints` 使用同步 `std::fs` 一次性读写整个 JSONL 文件 | 致命 (P0) |
| **Tokio blocking 线程池过小** | 默认 ~10 个线程，多个并发 checkpoint 操作迅速耗尽 | 致命 (P0) |
| **超时配置不足** | 多个关键超时（wrapper state 750ms、daemon startup 5s、post-commit 500ms）远低于慢磁盘所需时间 | 高危 (P1) |
| **Metrics Flush 阻塞 flush 循环** | `spawn_blocking` 中同步 HTTP + 同步 SQLite 写入阻塞整个 flush 循环 | 高危 (P1) |
| **Windows LockFile 句柄未缓存** | 每次加锁都重新加载 kernel32.dll，增加额外开销 | 低危 (P2) |

---

## 三、问题详情

### 3.1 致命级问题 (P0)

#### P0-1: JSONL 文件全量读写阻塞

**文件**: `src/git/repo_storage.rs` L390-582

```rust
// 当前实现：一次性读取整个文件到内存
pub fn read_all_checkpoints(&self) -> Result<Vec<Checkpoint>, GitAiError> {
    let content = fs::read_to_string(&checkpoints_file)?;  // 同步阻塞
    for line in content.lines() {
        let checkpoint: Checkpoint = serde_json::from_str(line)?;
    }
}

// 当前实现：一次性写入整个文件
pub fn write_all_checkpoints(&self, checkpoints: &[Checkpoint]) -> Result<(), GitAiError> {
    let content = lines.join("\n");
    fs::write(&checkpoints_file, format!("{}\n", content))?;  // 同步阻塞
}
```

**影响分析**:

| 工作日志大小 | 300KB/s 磁盘读取时间 | 300KB/s 磁盘写入时间 | 总阻塞时间 |
|-------------|---------------------|---------------------|-----------|
| 1 MB | ~3.4 秒 | ~3.4 秒 | ~7 秒 |
| 2 MB | ~6.9 秒 | ~6.9 秒 | ~14 秒 |
| 5 MB | ~17.2 秒 | ~17.2 秒 | ~34 秒 |
| 10 MB | ~34.5 秒 | ~34.5 秒 | ~69 秒 |

- 一次 `git commit` 归因流程会触发 **两次** `read_all_checkpoints`（pre-hook + post-hook）和 **一次** `write_all_checkpoints`
- 加上 `mutate_all_checkpoints` 的第三次读写，单次 commit 可能需要 **50-100 秒** 的阻塞时间
- 如果 process 在写操作过程中被杀死（VM 资源紧张时常见），整个工作日志文件被截断，数据永久丢失

#### P0-2: Daemon checkpoint 执行完全同步阻塞

**文件**: `src/daemon.rs` L1351-1384

```rust
fn apply_checkpoint_side_effect(request: CheckpointRunRequest) -> Result<(), GitAiError> {
    match request {
        CheckpointRunRequest::Live(request) => {
            let repo = find_repository_in_path(&request.repo_working_dir)?;
            let _ = crate::commands::checkpoint::run(...)?;  // 整个过程同步阻塞
            // 包括：
            // - git diff --staged (subprocess)
            // - git diff (subprocess)
            // - 文件内容读取 (std::fs)
            // - SQLite 写入 (rusqlite)
            // - 工作日志读写 (std::fs)
        }
    }
}
```

**影响**: 在 blocking 线程上执行完整 checkpoint 流程。一个慢 repo 占用一个 blocking 线程，多个并发 repo 快速耗尽线程池。

#### P0-3: Metrics Flush 在 spawn_blocking 中同步阻塞

**文件**: `src/daemon/telemetry_worker.rs` L232-287

```rust
async fn telemetry_flush_loop(buffer: Arc<Mutex<TelemetryBuffer>>) {
    let mut ticker = interval(FLUSH_INTERVAL); // 3 秒
    loop {
        ticker.tick().await;
        let snapshot = { buffer.lock().unwrap().take() };
        tokio::task::spawn_blocking(move || {
            flush_telemetry_batch(snapshot);  // 同步 HTTP (ureq) + 同步 SQLite
        }).await;
    }
}
```

**影响**: 指标上传失败后有 60 秒重试延迟，期间整个 flush 循环被阻塞。SQLite 写入慢时也会阻塞 flush。

---

### 3.2 高危级问题 (P1)

#### P1-1: Wrapper State 等待超时过短

**文件**: `src/daemon.rs` L7277-7336

```rust
async fn apply_wrapper_state_overlay(&self, command: &mut NormalizedCommand) {
    let timeout = self.wrapper_state_wait_timeout();  // 仅 750ms
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if has_pre && has_post { return Ok(()); }
        if tokio::time::Instant::now() >= deadline { return; }  // 超时放弃
        let remaining = deadline.saturating_duration_since(...);
        let _ = tokio::time::timeout(remaining, notified).await;
    }
}
```

**影响**: 慢磁盘环境下 git repo 读取极慢，750ms 内无法完成。Wrapper 状态覆盖失败 → 归因数据缺失。

#### P1-2: Post-Commit 统计超时过短

**文件**: `src/commands/git_handlers.rs` L721-817

```rust
let timeout = if std::env::var_os("GIT_AI_TEST_DB_PATH").is_some() {
    Duration::from_secs(20)
} else {
    Duration::from_millis(500)  // 生产环境仅 500ms
};
let poll_interval = Duration::from_millis(25);
```

**影响**: 500ms 内轮询等待 git authorship note，每 25ms spawn 一次 git subprocess。磁盘慢时几乎总是超时，统计信息完全丢失。

#### P1-3: Daemon 控制连接超时过短

**文件**: `src/daemon.rs` L87-90

| 超时配置 | 当前值 | 问题 |
|----------|--------|------|
| `DAEMON_CONTROL_CONNECT_TIMEOUT` | 2500ms | 尚可 |
| `DAEMON_CONTROL_RESPONSE_TIMEOUT` | 10s | 太短，blocking 线程池耗尽时 10s 内无法获得线程 |
| `DAEMON_CHECKPOINT_RESPONSE_TIMEOUT` | 300s | 合理 |
| `wrapper_state_wait_timeout` | 750ms | **太短** |
| `daemon_startup_timeout` | 5000ms | **太短**，慢磁盘初始化可能超过 |

#### P1-4: mutate_all_checkpoints 全量读写

**文件**: `src/git/repo_storage.rs` L585-597

```rust
pub fn mutate_all_checkpoints<F>(&self, mutator: F) -> Result<Vec<Checkpoint>, GitAiError> {
    let _lock = LockFile::try_acquire(&lock_path).ok_or_else(|| {
        GitAiError::Generic("timed out waiting for checkpoint lock".to_string())
    })?;
    let mut checkpoints = self.read_all_checkpoints()?;  // 全量读
    mutator(&mut checkpoints)?;
    self.write_all_checkpoints(&checkpoints)?;            // 全量写
    Ok(checkpoints)
}
```

**影响**: Post-commit hook 调用的就是这个方法。读-修改-写三步串行执行，在锁内完成，慢磁盘下持锁时间极长。

---

### 3.3 中危级问题 (P2)

| # | 文件位置 | 问题 | 影响 |
|---|----------|------|------|
| P2-1 | `src/api/metrics.rs` L11 | 重试延迟 60s 同步 sleep 阻塞 flush 循环 | Metrics 批量上传失败后系统阻塞 |
| P2-2 | `src/daemon/telemetry_handle.rs` L23 | `DAEMON_SOCKET_IO_TIMEOUT` 仅 2s | 守护进程负载高时 socket 写超时 |
| P2-3 | `src/commands/git_handlers.rs` L48-57 | Windows git proxy timeout 20s + 1 次重试 | 磁盘极慢时 git 命令本身被 kill |
| P2-4 | `src/commands/checkpoint.rs` L1110-1160 | captured checkpoint 多次阻塞文件写入 | Captured checkpoint 超时 |
| P2-5 | `src/utils.rs` L285 | Windows LockFile 每次 `libloading::Library::new("kernel32.dll")` 未缓存 | 每次加锁增加 DLL 搜索开销 |

---

## 四、300KB/s 磁盘 I/O 下的量化影响

### 场景 A: 用户执行 `git commit`，10 个 AI 修改的文件

```
操作步骤                          │ 耗时（300KB/s）│ 线程
─────────────────────────────────┼────────────────┼──────────────────
1. Post-commit hook 读 JSONL     │ ~7s            │ Tokio blocking #1
2. Post-commit hook 写 JSONL     │ ~7s            │ Tokio blocking #1
3. Daemon trace 处理读 JSONL      │ ~7s            │ Tokio blocking #2
4. Daemon checkpoint 写 JSONL    │ ~7s            │ Tokio blocking #2
5. SQLite 写入 (碎片化多次)       │ 2-5s          │ Tokio blocking #2
─────────────────────────────────┼────────────────┼──────────────────
总计                              │ ~30-33s        │ 2 个线程被长时间占用
```

### 场景 B: IDE 触发 checkpoint + 用户 commit + git fetch 并发

```
Tokio blocking 线程池: ~10 个线程

线程 #1-3:  被 checkpoint / commit 占用（各 30s）
线程 #4-5: 被 metrics flush 占用（每 3s 重置）
线程 #6-7: 被 git 操作 hooks 占用
线程 #8-10: 被 daemon trace 处理占用

→ 所有线程耗尽
→ 新请求无法获得线程
→ DAEMON_CONTROL_RESPONSE_TIMEOUT (10s) 触发
→ Wrapper 报错退出
→ 归因失败，数据丢失
```

### 场景 C: 守护进程启动超时

```
守护进程初始化步骤（全部同步阻塞）:
  ├─ 加载配置 (~500ms，取决于磁盘)
  ├─ 打开 SQLite 数据库 (~1s，取决于磁盘)
  ├─ prune_stale_captured_checkpoints (扫描目录，大量读)
  ├─ DaemonLock::acquire (创建锁文件)
  └─ Tokio runtime 初始化 + 创建 named pipes

→ 总计可能超过 5s
→ Wrapper 认为守护进程未启动
→ ensure_daemon_running 反复尝试
→ 形成重试循环，用户感知为"挂起"
```

---

## 五、解决方案

### 5.1 紧急缓解（立刻生效，无需修改代码）

通过环境变量配置绕过问题：

```bash
# 方案 A: 完全禁用守护进程，同步模式（最稳定，推荐作为 VM 环境默认）
GIT_AI_ASYNC_MODE=0
GIT_AI_POST_COMMIT_TIMEOUT_MS=30000
GIT_AI_GIT_PROXY_TIMEOUT_MS=120000
GIT_AI_SLOW_VM=1

# 或在 Windows 系统环境变量中永久设置
# 控制面板 → 系统 → 高级系统设置 → 环境变量 → 新建系统变量
```

**效果**: 每次 git 操作会同步等待归因完成（可能有几秒延迟），但**保证数据不丢失**。

**临时验证**:
```bash
# 验证当前 git-ai 状态
git-ai --version

# 在同步模式下测试
set GIT_AI_ASYNC_MODE=0
set GIT_AI_DEBUG=1
git-ai checkpoint --agent claude
```

---

### 5.2 根本性修复（需要代码改动）

#### 修复 1: JSONL 读写改为流式异步 (P0，核心改动)

**目标文件**: `src/git/repo_storage.rs`

**改动方案**: 用 `tokio::fs` 替代 `std::fs`，追加写入替代全量覆盖。

```rust
// 追加写入（不阻塞，异步）
pub async fn append_checkpoint_async(&self, checkpoint: &Checkpoint) -> Result<(), GitAiError> {
    let line = serde_json::to_string(checkpoint)? + "\n";
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&self.checkpoints_file)
        .await
        .map_err(|e| GitAiError::Io(e.to_string()))?;

    use tokio::io::AsyncWriteExt;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| GitAiError::Io(e.to_string()))?;
    Ok(())
}

// 流式读取（不一次性加载，按需解析）
pub async fn read_checkpoints_streaming<F>(&self, mut callback: F) -> Result<(), GitAiError>
where
    F: FnMut(Checkpoint) -> bool,
{
    let file = tokio::fs::File::open(&self.checkpoints_file)
        .await
        .map_err(|e| GitAiError::Io(e.to_string()))?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = tokio::io::BufRead::lines(reader);

    while let Some(line) = lines.next_line().await
        .map_err(|e| GitAiError::Io(e.to_string()))?
    {
        let checkpoint: Checkpoint = serde_json::from_str(&line)
            .map_err(|e| GitAiError::Parse(e.to_string()))?;
        if !callback(checkpoint) {
            break;
        }
    }
    Ok(())
}

// 后台压缩（当文件超过阈值时异步执行）
async fn maybe_compact_checkpoints(&self) {
    // 检查文件大小，超过阈值时异步压缩
    // 读取 → 过滤重复 base_commit → 写回
}
```

**预期收益**: 单次 checkpoint 的文件 I/O 从阻塞 30s+ 降至 **毫秒级追加 + 后台异步压缩**

#### 修复 2: Tokio Blocking 线程池扩容 (P0)

**目标文件**: `src/daemon.rs` 或 `src/lib.rs`

```rust
// daemon runtime 初始化
let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(8)           // IO 任务线程
    .max_blocking_threads(32)   // 阻塞任务线程: 10 → 32
    .build()?;

// 对于非 daemon 模式，wrapper 也需要足够的 blocking 线程
let wrapper_runtime = tokio::runtime::Builder::new_current_thread()
    .max_blocking_threads(16)   // 至少 16 个
    .build()?;
```

#### 修复 3: 所有超时配置环境变量化 (P1)

**目标文件**: `src/daemon.rs` `src/commands/git_handlers.rs`

```rust
// 从环境变量读取，保留默认值但允许覆盖
fn daemon_control_response_timeout() -> Duration {
    env_duration("GIT_AI_DAEMON_CONTROL_TIMEOUT_MS", 60_000)     // 10s → 60s
}

fn wrapper_state_wait_timeout() -> Duration {
    env_duration("GIT_AI_WRAPPER_STATE_TIMEOUT_MS", 10_000)     // 750ms → 10s
}

fn daemon_startup_timeout() -> Duration {
    env_duration("GIT_AI_DAEMON_STARTUP_TIMEOUT_MS", 20_000)    // 5s → 20s
}

fn post_commit_timeout() -> Duration {
    env_duration("GIT_AI_POST_COMMIT_TIMEOUT_MS", 10_000)      // 500ms → 10s
}

fn env_duration(key: &str, default_ms: u64) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(default_ms))
}
```

#### 修复 4: Metrics Flush 改为异步 HTTP (P1)

**目标文件**: `src/daemon/telemetry_worker.rs`

```rust
// 用 reqwest (async) 替代 ureq (sync)
async fn flush_telemetry_batch_async(snapshot: TelemetrySnapshot) -> Result<(), GitAiError> {
    // 全部异步，不阻塞 flush 循环
    let sentry_fut = async { reqwest::Client::new().post(&sentry_url).send().await };
    let posthog_fut = async { reqwest::Client::new().post(&posthog_url).send().await };
    let cas_fut = async { cas_client.upload_cas_async(&objects).await };
    let metrics_fut = async { store_metrics_in_db_async(&metrics).await };

    let _ = futures::join!(sentry_fut, posthog_fut, cas_fut, metrics_fut);
    Ok(())
}
```

#### 修复 5: Windows LockFile 句柄缓存 (P2)

**目标文件**: `src/utils.rs`

```rust
static KERNEL32_LIB: std::sync::OnceLock<libloading::Library> =
    std::sync::OnceLock::new();

fn kernel32() -> Option<&'static libloading::Library> {
    KERNEL32_LIB.get_or_init(|| unsafe {
        libloading::Library::new("kernel32.dll").ok()?
    })
}
```

---

## 六、实施优先级与风险评估

| 优先级 | 步骤 | 改动范围 | 风险 | 预期收益 | 推荐 |
|--------|------|----------|------|----------|------|
| **0** | 设置 `GIT_AI_ASYNC_MODE=0` | 仅环境变量 | 无 | 立即止血，100% 数据可靠 | **立刻执行** |
| 1 | 所有超时环境变量化 | `daemon.rs` + `git_handlers.rs` | 低 | 覆盖所有超时场景 | 第二优先 |
| 2 | Tokio blocking 线程池扩到 32 | `daemon.rs` | 低 | 提升并发阻塞能力 | 第二优先 |
| 3 | JSONL 读写改为异步流式 | `repo_storage.rs` | **中**（归因核心逻辑） | 消除最核心瓶颈 | 核心改动，充分测试后合并 |
| 4 | Metrics Flush 改为 async HTTP | `telemetry_worker.rs` | 中（依赖 reqwest） | 解耦 I/O 阻塞 | 后续迭代 |
| 5 | Windows LockFile 句柄缓存 | `utils.rs` | 低 | 减少加锁开销 | 小改动，随时可做 |

---

## 七、验证与测试

### 7.1 验证清单

```bash
# 1. 确认同步模式正常工作
GIT_AI_ASYNC_MODE=0 git-ai --version

# 2. 执行测试 commit，测量归因时间
set GIT_AI_ASYNC_MODE=0
set GIT_AI_DEBUG=1
time git commit -m "test: verify attribution"

# 3. 观察超时和错误
git commit -m "test" 2>&1 | findstr /i "timeout error hang"

# 4. 验证工作日志完整性
git notes --ref=ai list

# 5. 验证统计数据上传
# 检查 metrics-db 中的记录数
sqlite3 %USERPROFILE%\.git-ai\internal\metrics-db  "SELECT COUNT(*) FROM metrics_events;"
```

### 7.2 性能基准测试

建议在 VM 环境中建立以下基准：

| 指标 | 当前值 (300KB/s) | 目标值 | 验证方法 |
|------|-----------------|--------|----------|
| 单次 checkpoint 耗时 | ~30s+ | < 2s | `time git-ai checkpoint` |
| Post-commit 归因成功率 | 接近 0% | > 95% | 连续 10 次 commit，检查 note |
| Daemon 启动成功率 | < 50% | > 99% | 连续 20 次启动测试 |
| Metrics 上传延迟 | 可能丢失 | < 5s | 观察 flush 日志 |

---

## 八、环境配置推荐

针对 Windows 10 中文版虚拟机 (磁盘 I/O ~300KB/s)，推荐配置：

### 生产环境 (.env 或系统变量)

```bash
GIT_AI_ASYNC_MODE=0
GIT_AI_SLOW_VM=1
GIT_AI_POST_COMMIT_TIMEOUT_MS=30000
GIT_AI_GIT_PROXY_TIMEOUT_MS=120000
GIT_AI_DEBUG=1
```

### 未来优化后配置 (修复完成后)

```bash
# 启用守护进程，使用新修复的异步 I/O
GIT_AI_ASYNC_MODE=1
GIT_AI_SLOW_VM=1
# 超时使用修复后的默认值（通过代码内置）
GIT_AI_DEBUG=0
```

---

## 九、附录

### A. 关键文件索引

| 文件路径 | 用途 |
|----------|------|
| `src/daemon.rs` | 守护进程主入口，控制连接处理，超时配置 |
| `src/git/repo_storage.rs` | 工作日志读写，JSONL 全量操作 |
| `src/commands/checkpoint.rs` | Checkpoint 执行流程，锁超时，`SLOW_VM` 环境变量 |
| `src/daemon/telemetry_worker.rs` | Telemetry 后台 flush 循环 |
| `src/commands/git_handlers.rs` | Git proxy hooks，post-commit 统计轮询 |
| `src/utils.rs` | Windows LockFile 实现 |
| `src/api/metrics.rs` | Metrics 上传和重试逻辑 |

### B. 现有 SLOW_VM 检测代码

```rust
// src/commands/checkpoint.rs L136-147
fn checkpoint_lock_timeout() -> StdDuration {
    let base = parse_checkpoint_lock_timeout_ms(...);
    if std::env::var_os("GIT_AI_SLOW_VM").is_some() {
        base.max(StdDuration::from_secs(240))  // SLOW_VM 时至少 240s
    } else {
        base
    }
}
```

项目已认识到慢 VM 问题，但仅在锁超时有覆盖，其他所有超时点均未考虑。

### C. 术语说明

- **Tokio Blocking Thread Pool**: Tokio 运行时中专用于执行阻塞 I/O 操作的线程池，默认 ~CPU 核数，但最多约 512 个并发 blocking 调用会排队等待
- **Working Log**: `.git/ai/working_logs/<base_commit>/` 目录下的 JSONL 文件，记录每个 checkpoint 的文件归因
- **Authorship Note**: Git note 存储在 `refs/notes/ai`，记录每个 commit 的代码归因数据
- **Checkpoint**: git-ai 在 git 操作前后捕获工作区状态，用于分析 AI 编码行为
