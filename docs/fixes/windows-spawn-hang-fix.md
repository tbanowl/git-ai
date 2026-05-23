# Windows spawn hang fix 变更文档

背景
- 解决 Windows 10 虚拟机上通过 git-ai 执行 git 命令（如 rev-parse）时的间歇性挂起问题。

根本原因
- 主要原因：Pipe 句柄继承竞争条件。Windows 下 Rust 的 Command::spawn() 在 CreateProcessW 时将 bInheritHandles 设置为 TRUE。当并发达到上限（约 30 个并发 git 进程）时，子进程会继承同辈进程的全部可继承管道句柄，导致 finalize_pipe_reader() 等待 EOF，但某些同辈进程仍保持管道写端打开，进而产生死锁。
- 辅助原因：缺乏超时保护，exec_git* 系列调用 timeout 为 None，挂起无法自我恢复。
- 其他影响因素：GIT_TERMINAL_PROMPT 未在内部 git 调用中禁用、stdin 使用 stdin 管道导致额外可继承句柄等。

变更内容

## Phase A：spawn 串行化 + spawn 后句柄清理

### src/git/repository.rs
- 在 Windows spawn 路径上加入 spawn lock，避免多个 `Command::spawn()` 同时处于 `CreateProcessW(..., bInheritHandles = TRUE)` 的脆弱窗口。
- 子进程成功创建后，立即执行 post-spawn scrub，清理仅为 spawn 过程暂时标记为可继承的本地句柄，缩短句柄误继承窗口。
- 保留 `GIT_TERMINAL_PROMPT=0`，避免非交互内部 git 调用因凭据提示阻塞。
- 在没有输入数据时继续使用 `Stdio::null()` 而不是额外的 stdin pipe，减少可继承句柄面。

为什么 Phase A 有效
- 根因是并发 spawn 时可继承 pipe 句柄泄漏到同辈子进程，导致读端永远收不到 EOF。
- spawn lock 将高风险窗口串行化；post-spawn scrub 则把窗口长度压到最短，二者结合可显著降低 Windows 挂起概率。

## Phase B：超时驱动的 `CancelIoEx` 清理

### src/git/repository.rs
- 保留 `DEFAULT_EXEC_GIT_TIMEOUT: Duration = Duration::from_secs(60)` 作为内部 git 调用的默认超时。
- 继续让以下调用使用 `timeout: Some(DEFAULT_EXEC_GIT_TIMEOUT)`：
  - `exec_git_allow_nonzero_with_profile`
  - `exec_git_with_profile`
  - `exec_git_stdin_with_profile`
  - `exec_git_stdin_with_env_with_profile`
- 当 Windows 上的读写等待命中超时时，执行以 `CancelIoEx` 为核心的 cleanup 路径，主动打断挂住的 I/O，再回收子进程与 pipe 资源。

为什么 Phase B 有效
- 即使 Phase A 未完全避免极端条件下的继承/阻塞问题，超时与 `CancelIoEx` 清理也能把“永久挂起”降级为“可恢复失败”，避免 CLI 无限卡死。

## Deferred Phase C：恢复 Windows 并发上限

- 当前仍仅把“Windows 并发从 4 恢复到 30”作为后续 rollout 文档项记录，本次不修改任何 `MAX_CONCURRENT` 常量。
- 恢复前提是：Phase A/B 在真实 Windows 环境中稳定，通过针对 `git_exec_retry`、`internal_spawn_safety` 以及更高并发压力场景的验证后，再单独推进。
- Deferred Phase C 的目标是移除临时保守限流，而不是本次任务的一部分。

验证与 rollout 备注
- 本文档将 Phase A / Phase B 视为当前已落地基线，将 Deferred Phase C 视为后续单独放量项。
- 当前已验证：格式化、`git_exec_retry`、`internal_spawn_safety` 等可在当前主机直接执行的检查。
- 若当前主机已安装对应的 Windows target / toolchain，则可额外尝试 Windows target `cargo check`；若未安装，则跳过该项。最终发布信心仍以真实 Windows 环境验证为准。
- Phase C 仍待验证：在真实 Windows 环境完成更高并发压力验证后，才可考虑将 Windows 并发从 4 恢复到 30。

长期建议
- 根本解决方案应使用 CreateProcessW 的 PROCESS_HANDLE_LIST 属性，显式枚举需要继承的三个 stdio 句柄，并将 bInheritHandles 设置为 FALSE 以禁用其他句柄的继承。该方向在 rust-lang/rust#146407 跟踪中存在，待稳定后再替换 Windows 的 spawn 实现路径（在 build_git_command() 中的实现）。
