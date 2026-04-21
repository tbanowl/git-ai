# Windows spawn hang 现状分析（2026-04-21）

> 本文档是对 `windows-spawn-hang-fix.md` 的补充：把根因机制完整讲透，盘点已落地的 mitigation、剩余风险点、以及相关代码位置，作为后续设计（`docs/superpowers/specs/2026-04-21-windows-spawn-hardening-design.md`）的前置材料。

## 1. 症状

Windows 10 虚拟机上执行 git-ai（典型场景：`checkpoint`、`blame`、`commit` 钩子里调 `git show :file`、`git rev-parse`、`git diff` 等）时，偶发性地"整个命令卡住，不往下执行"。表现形式：

- 主线程阻塞在 `finalize_pipe_reader` 的 `JoinHandle::join()` 上（修复前）
- 或阻塞在 `child.wait()` 之后的 reader 线程收尾（修复前）
- 修复后主线程会在 60s 超时后返回 `EXEC_GIT_TIMEOUT_STDERR` 错误，但 reader 线程仍然阻塞不退出（泄漏）

频率在 VM 环境下显著高于物理机，并且随并发度升高而放大。

## 2. 根因：Windows 管道句柄继承竞态

### 2.1 Windows 进程模型与 Unix 的差异

Unix 是 `fork()` + `exec()` 两步：

- `fork` 复制整个进程地址空间，子进程继承父进程全部 FD
- 父进程在 `fork` 前对所有 FD 设置 `FD_CLOEXEC`
- 子进程 `exec` 时，内核关闭所有带 `FD_CLOEXEC` 的 FD，只用 `dup2` 显式传递 stdio

每个子进程的 FD 集合**完全可控且互不干扰**。

Windows 只有 `CreateProcessW`，继承策略是**进程级、非句柄级**的开关：

- `bInheritHandles = FALSE` — 什么句柄都传不过去，包括 stdio
- `bInheritHandles = TRUE` — **所有**被标记为 `HANDLE_FLAG_INHERIT` 的句柄都会被子进程继承

为了让 stdio 管道能工作，必须走 `bInheritHandles = TRUE` 这条路。从 Vista 起 Windows 提供了 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`，可以在 `bInheritHandles = TRUE` 的前提下**白名单**指定只继承某几个句柄——但 Rust 标准库至今没有采用（见 rust-lang/rust#146407）。

### 2.2 Rust `std::process::Command` 在 Windows 上的实现

流程（见 `rust/library/std/src/sys/pal/windows/process.rs`）：

1. `CreatePipe` 创建 3 个匿名管道（stdin/stdout/stderr）
2. 父子两端都被标记 `HANDLE_FLAG_INHERIT`
3. 构造 `STARTUPINFO`，把子端塞进 `hStdInput/hStdOutput/hStdError`
4. 调 `CreateProcessW(bInheritHandles = TRUE, ...)`
5. 父进程关闭自己持有的子端 copy，保留父端做 I/O

第 4 步那一刻，**进程中所有标记为可继承的句柄都会被新子进程拿到**，不只是这次 spawn 的 3 个管道。

### 2.3 死锁发生的时间线

以两个并发 spawn 为例（线程 A、线程 B），**关键是 B 的管道创建发生在 A 的 `CreateProcessW` 之前**。Windows 的 handle 是按进程的，父进程的 `CloseHandle` 只释放父进程自己的索引，不影响已继承到子进程的 handle 副本。

```
T0: 线程 A  CreatePipe  ->  ours_A (父,读) + theirs_A (子,写)   INHERIT=1
T1: 线程 B  CreatePipe  ->  ours_B (父,读) + theirs_B (子,写)   INHERIT=1
            此时父进程里同时有 4 个可继承句柄: ours_A, theirs_A, ours_B, theirs_B

T2: 线程 A  CreateProcessW(bInheritHandles=TRUE)
            child_A 继承当前全部 4 个可继承句柄
            其中 theirs_B  是 pipe B 的写端

T3: 线程 A  CloseHandle(theirs_A)       父进程释放自己的 theirs_A 索引
            theirs_A 仍被 child_A 持有（独立的 handle 副本）

T4: 线程 B  CreateProcessW(bInheritHandles=TRUE)
            child_B 继承 ours_A, ours_B, theirs_B
            (theirs_A 已在 T3 从父进程释放，不参与此次继承)

T5: 线程 B  CloseHandle(theirs_B)       父进程释放自己的 theirs_B 索引
            此刻 theirs_B 的持有者: child_A (T2 继承) + child_B (T4 继承)

T6: child_B  完成自己的 stdout 输出后退出
            child_B 持有的全部 handle 被内核释放，包括 theirs_B
            此刻 theirs_B 的持有者: child_A 一个

T7: 父进程线程 B 的 reader 线程  read_to_end(ours_B)  等 EOF
            EOF 到达的条件: theirs_B 全部引用释放
            child_A 仍在跑（比如 git diff 还在处理大文件、AV 正在扫描）
            ⇒ read_to_end 永远阻塞 ⇒ 死锁
```

关键推论：

- **死锁的充分条件**：在一次 `CreateProcessW(INHERIT=TRUE)` 调用的那一瞬间，父进程里存在除了本次 spawn 目标的 stdio 之外的其他可继承管道句柄。
- **等价表述**：两个或多个线程在 Rust `Command::spawn` 内部"`CreatePipe` 完成后、`CreateProcessW` 完成前"的窗口期有时间重叠。
- **触发放大因素**：任何一个 child 进程存续的生命周期越长，它作为"沉默持有者"堵住别人 EOF 的概率越大。Windows 10 VM 上慢 I/O、AV 同步扫描、文件系统响应延迟都会拉长这个存续时间，显著放大命中概率。

### 2.4 死锁发生的位置

**死锁不在子进程**：子进程早就退出了。死锁在**父进程的 reader 线程**里，具体是 `read_pipe_in_background` 启动的 `read_to_end` 调用。

## 3. 已落地的 mitigation（commit 35cde15）

见 `docs/fixes/windows-spawn-hang-fix.md`。四项：

| 措施 | 位置 | 作用 |
|---|---|---|
| 默认 60s 超时 | `src/git/repository.rs:306` `DEFAULT_EXEC_GIT_TIMEOUT` | 把"永久挂起"变成"60s 后返回超时错误"，主线程可恢复 |
| Windows 并发降到 4 | `src/git/repository.rs:2197`、`src/commands/checkpoint.rs:2250`、`src/authorship/virtual_attribution.rs:109, 183` | 降低 spawn 时间窗口重叠概率（C(30,2)→C(4,2) ≈ 1.4%） |
| `GIT_TERMINAL_PROMPT=0` | `src/git/repository.rs:339` | 避免 git 在非交互环境请求终端凭据时阻塞 |
| 无 stdin 数据时用 `Stdio::null()` | `src/git/repository.rs:328` | 不创建多余的可继承管道 |

这些措施**缓解**而非**消除**问题。实际部署后 Windows 10 VM 上挂起频率显著下降，且挂起会在 60s 后以错误形式返回，不再是"真·死锁"。

## 4. 剩余风险

### 4.1 根因未消除

概率防御仍然会命中。只要并发 ≥ 2，spawn 时间窗口仍然可能重叠。在慢 I/O 环境下，每 10⁶ 次调用里仍会有若干次超时事件被日志记录。

### 4.2 Reader 线程泄漏

timeout 路径（`src/git/repository.rs:438-459`）的收尾：

```rust
#[cfg(windows)]
let _ = kill_process_tree_windows(child.id());
#[cfg(not(windows))]
let _ = child.kill();
let _ = child.wait();
let _ = finalize_stdin_writer(stdin_handle);
let _ = finalize_pipe_reader(stdout_handle);
let _ = finalize_pipe_reader(stderr_handle);
```

`finalize_pipe_reader` 内部是 `handle.join()`，`_ =` 把结果丢掉——**但这只是忽略 `Result`，不是让线程结束**。如果 reader 线程正阻塞在一个永远不会 EOF 的管道上，`join()` 会立刻返回吗？不会。`let _` 是对 `Result` 的丢弃，对底层 `JoinHandle` 的 drop 只是**分离**（detach）线程，让它在后台继续运行，不等它。

所以每次 Windows 超时事件都会泄漏 1-2 个阻塞中的 reader 线程。单次无感知，长运行服务/daemon 场景下累积线程资源消耗。

### 4.3 不受 `GIT_TERMINAL_PROMPT=0` 保护的交互阻塞

- **Windows Credential Manager GUI 弹窗**：git-credential-manager.exe 在需要认证时弹出 GUI 对话框，不走终端 prompt，`GIT_TERMINAL_PROMPT=0` 无法阻止
- **askpass helper**：配置了 `core.askpass` 或 `GIT_ASKPASS` 环境变量时，git 会启动外部 helper
- 这些是 **git 自己的孙子进程**，其管道不由 git-ai 管理，也不走 git-ai 的 reader 线程；表现为"git 主进程看起来在跑但实际卡在等孙子进程"

### 4.4 索引锁无限等待

`index.lock` 文件存在时，git 默认等锁释放（可由 `core.lockTimeout` 配置）。`is_retryable_git_cli_error`（`src/git/repository.rs:510-518`）能识别 "index.lock" / ".lock ... unable to create" 的 stderr 字符串，但如果 git 一直等锁不返回 stderr，识别不到，只能靠 60s 超时兜底。

### 4.5 AV/EDR 同步注入扫描

Windows Defender 等反病毒产品会在 `CreateProcessW` 返回前做同步 DLL 扫描。VM 环境下单次 spawn 可能额外耗时 100-500ms。这会放大 §2.3 的时间窗口，间接提高死锁概率。这是运行环境问题，不在 git-ai 可修复范围内，但需要在 mitigation 设计时考虑其放大效应。

## 5. 代码位置索引

| 文件 | 行号 | 内容 |
|---|---|---|
| `src/git/repository.rs` | 306 | `DEFAULT_EXEC_GIT_TIMEOUT = 60s` |
| `src/git/repository.rs` | 317-358 | `build_git_command()` — `Command` 构造 |
| `src/git/repository.rs` | 328 | `Stdio::null()` 分支（无 stdin 时） |
| `src/git/repository.rs` | 339 | `GIT_TERMINAL_PROMPT=0` 环境变量注入 |
| `src/git/repository.rs` | 341-346 | `CREATE_NO_WINDOW` 分支（非交互终端时） |
| `src/git/repository.rs` | 373-386 | `read_pipe_in_background` — reader 线程工厂 |
| `src/git/repository.rs` | 402-412 | `finalize_pipe_reader` — 收 reader 线程 |
| `src/git/repository.rs` | 414-492 | `run_git_once` — spawn + wait + timeout 主流程 |
| `src/git/repository.rs` | 424 | `cmd.spawn()` 调用点 |
| `src/git/repository.rs` | 438-459 | timeout 分支：kill_process_tree_windows / child.kill + 丢 reader 线程 |
| `src/git/repository.rs` | 494-528 | 错误可重试性判断 |
| `src/git/repository.rs` | 2197 | `MAX_CONCURRENT` — staged files 并发上限 |
| `src/commands/checkpoint.rs` | 2250 | `MAX_CONCURRENT` — checkpoint 并发上限 |
| `src/authorship/virtual_attribution.rs` | 109, 183 | `MAX_CONCURRENT` — authorship 并发上限 |
| `src/utils.rs` | 172-174 | `is_interactive_terminal()` |
| `src/utils.rs` | 336 | `CREATE_NO_WINDOW: u32 = 0x08000000` — 手写 FFI 常量风格的示例 |
| `docs/fixes/windows-spawn-hang-fix.md` | — | 第一轮修复的变更说明 |

## 6. 为什么 Unix / macOS 没有这个问题

两层保障：

1. **`FD_CLOEXEC`**：Rust `std::process::Command` 在 Unix 下，管道创建后立刻 `fcntl(fd, F_SETFD, FD_CLOEXEC)`。`execve` 时内核自动关闭所有带 CLOEXEC 的 FD。stdio 另外用 `dup2` 复制（不带 CLOEXEC），达到"只传 stdio"的效果。
2. **`fork+exec` 两段式**：父进程在 `fork` 之后、`exec` 之前有机会精确调整子进程 FD 表。

结果：Unix 下两个并发 spawn 的子进程各自**物理上**看不到对方的管道 FD。

## 7. 根治方向

三条路，按治本程度/工程量排序：

1. **Spawn 序列化 + 父端句柄 scrub `HANDLE_FLAG_INHERIT`**（本项目即将采用的方案）
   - 用进程级 mutex 包住 `cmd.spawn()`；spawn 后立刻对父端 stdio 句柄清除 INHERIT 标志
   - 工程量小（~50 行 + parking_lot 依赖），治本，可把 Windows 并发放回 30
   - 局限：依赖"进程中除 Rust Command 外无其他可继承句柄"前提（本项目成立）
   - 详见 `docs/superpowers/specs/2026-04-21-windows-spawn-hardening-design.md`
2. **自写 `CreateProcessW` + `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`**
   - 绕开 `std::process::Command`，白名单式继承
   - 工程量大（150-300 行 unsafe），需要跟着 Rust std 行为修正
   - 对第三方库也能隔离，但本项目不需要这层兼容
3. **等 Rust 标准库修复（rust-lang/rust#146407）**
   - 被动方案；时间表不可控
   - 修复后升 Rust 版本即可

此外，从业务侧消除 spawn（P1/P2 git2 migration，见 `docs/superpowers/plans/2026-04-21-p1p2-git2-migration.md`）是**长期方向**——迁移完成后 `exec_git*` 调用量大幅下降，spawn hardening 的覆盖面随之减小，但只要还有任何 `exec_git*` 路径，hardening 都有价值。

## 8. 可观测性建议

当前 `run_git_once` 已有 `trace_id` 和慢命令日志（`[git-ai:slow]`）。建议后续补：

- 超时事件单独计数（目前夹在 `GitCliError` 里，需要字符串匹配 stderr 才能识别）
- reader 线程泄漏计数（需要 Phase B 先做 cancel_io 才能识别"真挂起"与"刚好在 join 时慢了"）
- Windows 并发压力指标（semaphore 等待时间）

这些不在 hardening 设计的范围内，列为后续可选增强。
