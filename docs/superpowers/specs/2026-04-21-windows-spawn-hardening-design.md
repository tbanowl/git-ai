# Windows spawn hardening design（2026-04-21）

> 前置阅读：`docs/fixes/2026-04-21-windows-spawn-hang-analysis.md`（问题现状与根因分析）

## 1. 目标

在不改变 `exec_git*` API 签名、不影响非 Windows 平台、不引入大面积 unsafe 代码的前提下，消除 `src/git/repository.rs` 里 `cmd.spawn()` 路径上的 Windows 管道句柄继承死锁根因；并补上 timeout 分支的 reader 线程泄漏漏洞。

交付后 Windows 下 `exec_git*` 的稳定性应当恢复到与 Unix 基本一致的水平，且允许把 Windows 并发上限从 4 恢复到 30。

## 2. 非目标

- **不替换 `std::process::Command`**（由 rust-lang/rust#146407 解决）
- **不重写 `CreateProcessW` 调用路径**（即不采用方案 F：`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`）
- **不改默认 60s 超时**（保留作为 defense-in-depth）
- **不处理 git 孙子进程弹窗阻塞**（credential manager / askpass，独立 ticket）
- **不调整 git2 migration 进度**（P1/P2 迁移继续按既定节奏；本设计是过渡期的稳定补丁）
- **不保护 `exec_git*` 之外的 spawn 点**（`daemon.rs`、`install_hooks.rs`、`taskkill` 等）——如后续有需要，可把本设计的 `win_spawn` 模块抽到 `utils` 供其他 spawn 复用

## 3. 架构

分两个阶段，解耦实现，可独立 PR，次序：**#1 Phase A → #2 Phase B → #3 放开并发**。

### Phase A — spawn 序列化 + 清除父端句柄的 `HANDLE_FLAG_INHERIT`

治本。用一个进程级 `parking_lot::Mutex<()>` 包住 `cmd.spawn()` 与紧随其后的父端句柄 scrub 动作。critical section 只覆盖几毫秒的 spawn 瞬间，**不包**命令执行。

原理：继承竞态的充分条件是"spawn 时进程里同时存在多对可继承的管道句柄"。序列化保证了同一时刻只有一对管道处于可继承状态；spawn 完成后立刻 scrub 父端句柄的 INHERIT 标志，让它不会被下一次 spawn 的子进程继承。

### Phase B — reader 线程可终止

兜底。Phase A 消除根因后理论上不再发生死锁，但：

- 其他意外场景（git 孙子进程继承、AV 产品挂钩管道等）仍可能让 reader 阻塞
- 60s 超时兜住了主线程，但 reader 线程被 `let _ = finalize_pipe_reader(...)` 分离到后台继续阻塞，长运行下累积

方案：spawn 后保留 stdout/stderr 原生句柄副本（`RawHandle`，拷贝语义），timeout 分支 kill 子进程后调 `CancelIoEx(handle, NULL)`，强制唤醒阻塞的 `ReadFile`，reader 线程返回错误后正常退出。

## 4. Phase A 详细设计

### 4.1 新增模块

位置：`src/git/repository.rs` 顶部，紧邻现有 `#[cfg(windows)]` 块。

```rust
#[cfg(windows)]
mod win_spawn {
    use parking_lot::{Mutex, MutexGuard};
    use std::sync::OnceLock;
    use std::os::windows::io::AsRawHandle;

    // Win32 手写 FFI，与 utils.rs 现有 CREATE_NO_WINDOW 风格保持一致
    pub const HANDLE_FLAG_INHERIT: u32 = 0x00000001;

    type BOOL = i32;
    type DWORD = u32;
    type HANDLE = *mut core::ffi::c_void;

    unsafe extern "system" {
        fn SetHandleInformation(h: HANDLE, mask: DWORD, flags: DWORD) -> BOOL;
    }

    /// Process-wide mutex held across CreateProcessW + post-spawn handle
    /// scrubbing to prevent handle-inheritance races between concurrent spawns.
    static SPAWN_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    pub fn spawn_lock() -> MutexGuard<'static, ()> {
        SPAWN_MUTEX.get_or_init(|| Mutex::new(())).lock()
    }

    /// Clear HANDLE_FLAG_INHERIT on a child stdio pipe handle so it is not
    /// leaked into future CreateProcessW calls.
    ///
    /// Best-effort: Windows API failures are logged but not propagated. The
    /// caller's git command should not fail because of a flag-clear failure.
    pub fn scrub_inherit<H: AsRawHandle>(h: &H, label: &'static str) {
        let raw = h.as_raw_handle() as HANDLE;
        // SAFETY: `raw` is a kernel handle owned by the caller's ChildStdio
        // wrapper for the duration of this call. We only modify its flags;
        // the handle itself remains valid and owned.
        let ok = unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, 0) };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            tracing::debug!(
                "[win_spawn] SetHandleInformation({}, INHERIT=0) failed: {err}",
                label
            );
        }
    }
}
```

### 4.2 `run_git_once` 改造

对应 `src/git/repository.rs:414-492`。改动集中在 `cmd.spawn()` 前后：

```rust
fn run_git_once(request: &GitExecRequest) -> Result<Output, GitAiError> {
    let PreparedGitCommand { mut cmd, effective_args } = build_git_command(request);

    let cmd_start = Instant::now();
    let trace_id = uuid::Uuid::new_v4();
    tracing::debug!("[exec_git] {} Starting git command execution", trace_id);

    // === Windows spawn critical section (Phase A) ===
    #[cfg(windows)]
    let _spawn_guard = win_spawn::spawn_lock();

    let mut child = cmd.spawn().map_err(GitAiError::IoError)?;

    #[cfg(windows)]
    {
        if let Some(h) = child.stdout.as_ref() { win_spawn::scrub_inherit(h, "stdout"); }
        if let Some(h) = child.stderr.as_ref() { win_spawn::scrub_inherit(h, "stderr"); }
        if let Some(h) = child.stdin.as_ref()  { win_spawn::scrub_inherit(h, "stdin");  }
    }
    #[cfg(windows)]
    drop(_spawn_guard);
    // === end critical section ===

    let stdin_handle = request.stdin_data.as_deref()
        .and_then(|data| write_stdin_in_background(&mut child, data));
    let stdout_handle = read_pipe_in_background(child.stdout.take());
    let stderr_handle = read_pipe_in_background(child.stderr.take());

    // ...（后续 wait + timeout + finalize 逻辑不动）
}
```

### 4.3 Critical section 范围的证明

锁必须覆盖 **`cmd.spawn()` + 三个 scrub** 整体。

若锁只包 `cmd.spawn()`、释放后才 scrub：在"释放锁"与"scrub 完成"之间的瞬间，线程 2 可能拿到锁进入它的 spawn；此时线程 1 的父端句柄仍然 `HANDLE_FLAG_INHERIT=1`，会被线程 2 的 `CreateProcessW(bInheritHandles=TRUE)` 继承——就是我们要消除的状态。

锁覆盖范围内**没有用户代码、没有外部 I/O**，只有：

1. `cmd.spawn()` 内部走的 `CreatePipe`×3 + `CreateProcessW` + `CloseHandle`×3
2. 3 次 `SetHandleInformation`

全部是内核 syscall，不可能死锁、不可能阻塞到可感知的程度。Windows 上典型耗时 < 2ms。

### 4.4 为什么三个句柄都要 scrub

| 句柄 | 不 scrub 的后果 |
|---|---|
| stdout（读端） | 下次 spawn 的子进程继承它，造成句柄泄漏（累积占用）。不会死锁 |
| stderr（读端） | 同上 |
| **stdin（写端）** | **重建死锁条件**。下次 spawn 的子进程继承这个写端；当后来某次 spawn 的子进程尝试 `read(stdin)` 时，因为 stdin 写端有多个持有者，EOF 永远不到——子进程挂起。`git commit-tree` / `hash-object --stdin` / `update-ref --stdin` 都会走这条路 |

### 4.5 依赖变更

`Cargo.toml` 的 `[dependencies]` 下加一行：

```toml
parking_lot = "0.12"
```

`parking_lot` 已经是多个间接依赖（Cargo.lock 已存在），加为直接依赖不会扩大构建图。

**不**引入 `windows-sys` / `winapi`——采用手写 `extern "system"` 方式，与 `src/utils.rs:336` 现有 `CREATE_NO_WINDOW: u32 = 0x08000000` 的风格保持一致。

## 5. Phase B 详细设计

### 5.1 扩展 `win_spawn` 模块

加入 `CancelIoEx`：

```rust
#[cfg(windows)]
mod win_spawn {
    // ... 上面的 SetHandleInformation / HANDLE_FLAG_INHERIT / spawn_lock ...

    unsafe extern "system" {
        fn CancelIoEx(h: HANDLE, overlapped: *const core::ffi::c_void) -> BOOL;
    }

    /// Cancel any pending I/O on a pipe handle so a blocked reader thread
    /// returns immediately with ERROR_OPERATION_ABORTED.
    ///
    /// Safe to call on a handle whose owner may have already released it:
    /// the kernel returns ERROR_INVALID_HANDLE, which we log and ignore.
    pub fn cancel_io(raw: std::os::windows::io::RawHandle, label: &'static str) {
        let h = raw as HANDLE;
        // SAFETY: CancelIoEx performs its own kernel-side handle validation.
        // Passing a stale handle value returns ERROR_INVALID_HANDLE rather
        // than triggering UB.
        let ok = unsafe { CancelIoEx(h, core::ptr::null()) };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            let code = err.raw_os_error().unwrap_or(0);
            // 1168 = ERROR_NOT_FOUND (no pending I/O; reader already returned)
            // 6    = ERROR_INVALID_HANDLE (handle already closed)
            // Both are expected races; don't noise up logs with them.
            if code != 1168 && code != 6 {
                tracing::debug!("[win_spawn] CancelIoEx({}) failed: {err}", label);
            }
        }
    }
}
```

### 5.2 `run_git_once` timeout 分支改造

对应 `src/git/repository.rs:438-459`。在 spawn 后、reader 启动前保留两个 `RawHandle`：

```rust
#[cfg(windows)]
let stdout_raw: Option<std::os::windows::io::RawHandle> =
    child.stdout.as_ref().map(std::os::windows::io::AsRawHandle::as_raw_handle);
#[cfg(windows)]
let stderr_raw: Option<std::os::windows::io::RawHandle> =
    child.stderr.as_ref().map(std::os::windows::io::AsRawHandle::as_raw_handle);

let stdout_handle = read_pipe_in_background(child.stdout.take());
let stderr_handle = read_pipe_in_background(child.stderr.take());
```

timeout 触发路径改造后（保留现有的 `kill_process_tree_windows` 调用）：

```rust
if cmd_start.elapsed() >= timeout {
    tracing::debug!(
        "git command [{:?}] timed out after {}ms",
        effective_args.first(),
        timeout.as_millis()
    );

    #[cfg(windows)]
    let _ = kill_process_tree_windows(child.id());
    #[cfg(not(windows))]
    let _ = child.kill();
    let _ = child.wait();

    // === Phase B: cancel pending reads so reader threads can exit ===
    #[cfg(windows)]
    {
        if let Some(h) = stdout_raw { win_spawn::cancel_io(h, "stdout"); }
        if let Some(h) = stderr_raw { win_spawn::cancel_io(h, "stderr"); }
    }

    let _ = finalize_stdin_writer(stdin_handle);
    let _ = finalize_pipe_reader(stdout_handle);
    let _ = finalize_pipe_reader(stderr_handle);

    return Err(GitAiError::GitCliError {
        code: Some(1),
        stderr: EXEC_GIT_TIMEOUT_STDERR.to_string(),
        args: effective_args,
    });
}
```

### 5.3 关键设计决策

**为什么是 `RawHandle` 拷贝而非 owned handle**

`ChildStdout` / `ChildStderr` 是 owned，`take()` 之后所有权 move 到 reader 线程。若主线程再存一份 owned 副本会 double-free。`RawHandle = *mut c_void`，是整数拷贝语义，不拥有生命周期，只是 handle 值的副本。`CancelIoEx` 只需要 handle 值即可；即使 reader 线程已经 close 了真 handle，调用失败返回 `ERROR_INVALID_HANDLE`，我们过滤掉这个 errno 就行。**不会 UB，因为 `CancelIoEx` 自己在内核侧校验 handle 合法性。**

**为什么先 `child.kill()` 再 `cancel_io`**

先 kill：消除管道写端的合法持有者，**如果没有继承竞态**，子进程退出后写端引用计数归零，reader 自然 EOF 退出，完全不需要走 cancel 路径。这保留了快路径的正常性。

后 cancel：**如果**有继承竞态（别的还活着的子进程继承了本次的写端），kill 无法救出 reader。此时 `CancelIoEx` 强行让内核中止正在 pending 的 `ReadFile`，reader 立刻返回 `ERROR_OPERATION_ABORTED` 退出。

不在 cancel 之间加 `sleep(100ms)` 缓冲——kill 后调 cancel 没副作用（快路径下 cancel 也是 no-op），加 sleep 只拖慢清理。

**为什么不对 stdin 做 cancel**

stdin 方向是父进程写、子进程读。父进程 stdin writer 线程如果阻塞是因为 pipe 满写不进去——但子进程已经被 kill，读端被关，stdin pipe 进入 broken 状态，`write_all` 立刻返回 `BrokenPipe`，不会卡。`finalize_stdin_writer` 本来就忽略 `BrokenPipe` 错误（见 `src/git/repository.rs:388-400`）。无需 cancel。

### 5.4 非 Windows 路径

`stdout_raw` / `stderr_raw` 的定义和使用全部 `#[cfg(windows)]`。非 Windows 平台：

- 无 `AsRawHandle` trait
- `cancel_io` 不存在，timeout 分支也不调
- `finalize_pipe_reader` 在 Unix 下的风险低得多（无继承竞态），即使 reader 被 detach，靠 `child.kill()` + `SIGKILL` 后管道写端归零，EOF 自然到达，reader 线程最终退出

无需 Unix 侧兜底。

## 6. 测试计划

| 测试 | 类型 | 平台 | 目的 | 新增/修改 |
|---|---|---|---|---|
| `tests/unit/windows_spawn_handles.rs` | unit | windows | `exec_git(&["rev-parse", "--git-dir"])` 后用 `GetHandleInformation` 验证 child stdio handle 的 `HANDLE_FLAG_INHERIT=0` | 新增（Phase A） |
| `tests/unit/windows_cancel_io.rs` | unit | windows | 构造一个不自然退出的子进程（`cmd.exe /c ping -n 20 127.0.0.1`），触发 timeout，验证 reader `join()` 在 1s 内返回 | 新增（Phase B） |
| `tests/stress/concurrent_spawn.rs` | stress | all | 并发 200 次 `exec_git(&["rev-parse", "--git-dir"])`，全部成功且无 timeout 错误；对比 Phase A 前后 | 新增（Phase A） |
| `tests/integration/git_repository_comprehensive.rs` | integration | all | 不能回归 | 保持 |
| `cargo check --target x86_64-pc-windows-gnu`（或 msvc） | build | — | `#[cfg(windows)]` 代码能编过 | CI |

**关于 Windows CI**：`.github/workflows/test.yml` 已经配置了 `windows-latest` runner（见 test.yml:49/53/57），新增测试会自动在 Windows CI 上跑，无额外 workflow 改动。

## 7. 风险矩阵

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| `SetHandleInformation` 调用失败 | 极低（handle 刚来自 `cmd.spawn()`） | 等同于 Phase A 未生效，回退到当前行为 | best-effort + debug log，不传播错误 |
| `CancelIoEx` 对已关闭 handle 调用 | 偶发（reader 恰好自然返回） | 无 UB，Windows 返回 `ERROR_INVALID_HANDLE` | 过滤 errno 6/1168，不打日志 |
| spawn mutex 死锁 | 不可能 | — | critical section 内只有内核 syscall + 3 次 flag 设置，无用户代码、无阻塞 I/O |
| spawn 吞吐下降 | — | `cmd.spawn()` + 3 次 `SetHandleInformation` 在 Windows 上典型 < 2ms；200 次串行 < 400ms，远小于 git 命令本身耗时 | 可忽略 |
| 锁中毒 | 不可能 | — | `parking_lot::Mutex` 无中毒语义 |
| `RawHandle` 被 reader 线程 close 后变野 | 偶发 | 无 UB；`CancelIoEx` 返回 `ERROR_INVALID_HANDLE` | 过滤 errno 6 |
| Phase A 引入 panic 破坏 mutex 状态 | 极低 | — | `parking_lot::Mutex` panic 时自动 unlock，不中毒 |
| 非 Windows 平台被意外波及 | 不可能 | — | 所有改动在 `#[cfg(windows)]` 下 |
| `parking_lot` 版本升级不兼容 | 低 | 重写 Mutex 使用 | 锁定 0.12 minor |

## 8. 可观测性

不强制在本设计中实装，但推荐 Phase A/B 落地时**同时**加上以下 metric（可以放到 Phase B PR 里，与 cancel_io 一起）：

- `win_spawn_timeouts` counter（每次 timeout 分支触发 +1）
- `win_spawn_cancel_io_invoked` counter（每次实际调 CancelIoEx +1）
- `win_spawn_cancel_io_hit` counter（CancelIoEx 成功返回、非预期 errno 的次数）

如果 Phase A 真正治本，`win_spawn_timeouts` 应当 → 0。若仍有计数，说明有 §2.3 以外的卡住路径，提供后续调查线索。

## 9. PR 拆分与 rollout

### PR #1 — Phase A

- 加 `parking_lot = "0.12"` 到 `Cargo.toml`
- 新增 `win_spawn` 模块（`SetHandleInformation` + `spawn_lock` + `scrub_inherit`）
- 改造 `run_git_once` 加 spawn guard + post-spawn scrub
- 新增 `tests/unit/windows_spawn_handles.rs` + `tests/stress/concurrent_spawn.rs`
- **不动** `MAX_CONCURRENT`；Windows 并发保持 4
- 配套更新 `docs/fixes/windows-spawn-hang-fix.md`，追加 "Phase A" 小节记录落地情况

### PR #2 — Phase B

- `win_spawn` 模块扩展 `CancelIoEx` + `cancel_io`
- `run_git_once` 保存 stdout/stderr `RawHandle`，timeout 分支加 cancel 调用
- 新增 `tests/unit/windows_cancel_io.rs`
- 可选：实装 §8 的三个 counter

### PR #3 — 放开 Windows 并发

- **前置**：PR #1 合并后观察 1-2 周 Windows 生产运行
  - 期望：Sentry / 日志里 Windows timeout 事件趋近于 0
  - 期望：stress test 在内部 CI 上稳定
- 把以下三处 `30` 改回统一 `30`：
  - `src/git/repository.rs:2197`
  - `src/commands/checkpoint.rs:2250`
  - `src/authorship/virtual_attribution.rs:109, 183`
- 配套更新 `docs/fixes/windows-spawn-hang-fix.md` "Phase A" 小节补记并发恢复情况

### Rollback 策略

每个 PR 都是独立可回退的：

- 回退 PR #3 → Windows 并发回到 4，稳定性不变
- 回退 PR #2 → 损失 reader 线程 cancel，回退到 "主路径返回但 reader 泄漏"的修复前状态（已有 60s 超时兜底，非严重回归）
- 回退 PR #1 → 完全回到修复前状态，但 60s 超时仍在，非致命

## 10. 开放问题

以下不影响设计落地，但落地时要确认：

1. **`parking_lot` 版本兼容**：0.12 是 2024 以来的稳定线；落地时 `cargo tree -i parking_lot` 确认既有间接依赖版本是否在 0.12.x range，以免锁到两个 major。
2. **`src/commands/daemon.rs` 等其他 spawn 点**：目前设计不覆盖。若后续发现 daemon spawn 也有类似 hang，把 `win_spawn::spawn_lock()` 升级到 `utils` 模块，让 daemon 路径也 hold 同一把锁。本设计不提前做。
3. **Telemetry 上报路径**：如果 §8 的 counter 要上报到 Sentry / 日志收集系统，需要确认项目已有的 observability 通道。本设计假设复用现有 `tracing` + `crate::observability::log_error` 机制。
