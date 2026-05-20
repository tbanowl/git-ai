# Daemon 模式下 `git commit` 完整流程

本文记录 `git-ai` 在 daemon / async mode 下处理 `git commit` 的端到端流程，重点说明前台 wrapper、Git Trace2、daemon side-effect pipeline、synthetic pre-commit checkpoint replay、post-commit authorship note 生成之间的关系。

## 核心结论

daemon 模式下，前台 wrapper 不走普通同步模式的 `commit_pre_command_hook()`。真正等价的 pre-commit checkpoint 是 daemon 在 commit 已经落地后，基于 Trace2、wrapper pre/post state、committed diff 做的 synthetic replay。

因此需要区分两种“pre-commit”：

- **同步 wrapper 模式**：前台执行 `commit_pre_command_hook()` → `pre_commit::pre_commit()` → `checkpoint::run()`。
- **daemon / async 模式**：daemon 执行 `sync_pre_commit_checkpoint_for_daemon_commit()`，用 commit final snapshot 回放等价 checkpoint。

## 1. 入口分发

入口在 `src/main.rs`：

- `argv[0] == "git-ai"` → `commands::git_ai_handlers::handle_git_ai()`
- `argv[0] == "git"` 或 debug 下 `GIT_AI=git` → `commands::git_handlers::handle_git()`

核心 daemon 分支在 `src/commands/git_handlers.rs` 的 `handle_git()` 中：

```rust
if config::Config::get().feature_flags().async_mode {
    ...
    proxy_to_git(args, false, None, Some(&invocation_id));
    ...
    maybe_show_async_post_commit_stats(...);
    exit_with_status(exit_status);
}
```

`async_mode=true` 时这里会直接处理并退出，不会进入后面的普通 hook path：

```rust
run_pre_command_hooks(...);
proxy_to_git(...);
run_post_command_hooks(...);
```

所以 daemon 模式下，前台 wrapper 不调用：

- `run_pre_command_hooks()`
- `commit_hooks::commit_pre_command_hook()`
- `pre_commit::pre_commit()`
- `commit_hooks::commit_post_command_hook()`

这些是同步 wrapper 模式的路径。

## 2. async wrapper 前台职责

在 `src/commands/git_handlers.rs` 的 async 分支中，前台 wrapper 做的是低延迟代理和状态采样：

1. 解析 git 参数。
2. 跳过只读命令，例如 `status` / `log` 等。
3. 跳过 repo-creating 命令，例如 `clone` / `init`。
4. 初始化 daemon telemetry/control socket。
5. 找 repo/worktree。
6. 读取 commit 前 HEAD state。
7. 生成 `invocation_id`。
8. 发送 `WrapperPreState` 给 daemon。
9. 调真实 git。
10. git 返回后读取 commit 后 HEAD state。
11. 发送 `WrapperPostState` 给 daemon。
12. 如果是成功的 `git commit`，可选等待 daemon 生成 authorship note 并显示 stats。

关键函数：

- `send_wrapper_pre_state_to_daemon()`：`src/commands/git_handlers.rs`
- `send_wrapper_post_state_to_daemon()`：`src/commands/git_handlers.rs`
- `proxy_to_git()`：`src/commands/git_handlers.rs`
- `maybe_show_async_post_commit_stats()`：`src/commands/git_handlers.rs`

`proxy_to_git()` 会给真实 git 注入：

```text
GIT_AI_WRAPPER_INVOCATION_ID=<uuid>
GIT_TRACE2_ENV_VARS=GIT_AI_WRAPPER_INVOCATION_ID
```

这样 Git Trace2 事件可以带上 wrapper invocation id，daemon 后续能把 Trace2 command 与 wrapper pre/post state 对齐。

## 3. 旧 Git core hook 已 sunset

`src/main.rs` 中检测到 git hook binary name 时只打印迁移提示并退出 0：

```rust
if commands::git_hook_handlers::is_git_hook_binary_name(&binary_name) {
    eprintln!("git-ai: the git core hooks feature has been sunset...");
    std::process::exit(0);
}
```

因此如果旧的 `.git/hooks/pre-commit` / `post-commit` symlink 仍指向 git-ai，它不会执行业务逻辑。

daemon 模式主要依赖：

- git wrapper
- Git Trace2
- daemon control socket
- daemon side-effect pipeline

而不是传统 `.git/hooks/pre-commit`。

## 4. 真实 `git commit` 前台执行

前台 wrapper 调用真实 git：

```rust
proxy_to_git(args, false, None, Some(&invocation_id));
```

`proxy_to_git()` 负责：

- `Command::new(config::Config::get().git_cmd())`
- 传入原始 git args
- 设置 `GIT_AI_WRAPPER_INVOCATION_ID`
- 设置 `GIT_TRACE2_ENV_VARS`
- 移除 `GIT_AI_ASYNC_MODE`，避免泄漏到 git alias/script
- Unix 下做 signal forwarding
- 阻塞等待真实 git 子进程退出

这一步是真正的 `git commit`，会创建 commit、更新 HEAD、写 reflog。

## 5. daemon 接收 Trace2 并归一化命令

daemon 核心在 `src/daemon.rs`。

Trace2 payload 进入：

```rust
ingest_trace_payload_fast()
```

随后进入：

```rust
apply_trace_payload_to_state()
```

这里会调用：

```rust
normalizer.ingest_payload(&payload)
```

`src/daemon/trace_normalizer.rs` 会把原始 Git Trace2 stream 聚合成一个完整的 `NormalizedCommand`。terminal event（例如 `exit` / `atexit`）到达后，normalizer 才会 emit command。

daemon 还会把 wrapper state overlay 上去：

```rust
if applied.command.wrapper_invocation_id.is_some() {
    self.apply_wrapper_state_overlay(&mut applied.command).await;
}
```

这一步把 wrapper 发送的精确 pre/post HEAD 覆盖到 Trace2 推断结果上。daemon 最多等待一小段时间获取 wrapper state；超时则使用 daemon 内部 state。

## 6. Commit semantic event 到 RewriteLogEvent

daemon 处理完整命令后，进入：

```rust
maybe_apply_side_effects_for_applied_command()
```

这里拿到 `applied.analysis.events`，再调用：

```rust
rewrite_events_from_semantic_events()
```

对 commit 来说，关键分支是：

```rust
SemanticEvent::CommitCreated { base, new_head } => {
    out.push(RewriteLogEvent::commit(base.clone(), new_head.clone()));
}
```

对于 amend：

```rust
SemanticEvent::CommitAmended { old_head, new_head } => {
    out.push(RewriteLogEvent::commit_amend(old_head.clone(), new_head.clone()));
}
```

之后 daemon 调：

```rust
apply_rewrite_side_effect(...)
```

## 7. daemon 中的 synthetic pre-commit checkpoint replay

这是 daemon 模式下等价于 pre-commit 的核心逻辑。

入口：

```rust
sync_pre_commit_checkpoint_for_daemon_commit()
```

它由 `apply_rewrite_side_effect()` 调用：

```rust
sync_pre_commit_checkpoint_for_daemon_commit(
    &repo,
    &rewrite_event,
    &author,
    normalized_carryover_snapshot_ref,
)?;
```

流程：

1. 从 `RewriteLogEvent::Commit` / `CommitAmend` 中解析：
   - `base_commit`
   - `target_commit`

2. 获取 committed diff 的文件快照：

   ```rust
   committed_file_snapshot_between_commits(repo, committed_diff_base, &target_commit)
   ```

3. 过滤出需要 replay 的文件：

   ```rust
   filter_commit_replay_files(...)
   ```

4. 检测当前是否有 active bash AI context：

   ```rust
   checkpoint_context_from_active_bash(repo_root, &repo_workdir)
   ```

5. 根据是否有 AI context 构造 replay checkpoint：

   - 有 AI bash context：构造 `CheckpointKind::AiAgent` 类型的 replay checkpoint，归因给 AI agent。
   - 没有 AI bash context：构造 synthetic Human replay checkpoint。

6. 执行 checkpoint replay：

   ```rust
   checkpoint::run_with_base_commit_override_with_policy(
       repo,
       author,
       checkpoint_kind,
       true,
       Some(replay_agent_result),
       base_commit != "initial",
       Some(base_commit.as_str()),
       BaseOverrideResolutionPolicy::RequireExplicitSnapshot,
   )
   ```

这里的 `base_commit_override` 和 `RequireExplicitSnapshot` 很重要：daemon 不依赖当前工作区猜测，而是用 committed diff / carryover snapshot 作为精确输入。

## 8. daemon 中的 post-commit authorship note 生成

pre-commit replay 完成后，`apply_rewrite_side_effect()` 继续处理 authorship。

普通 commit 走：

```rust
post_commit_with_final_state(
    &repo,
    commit.base_commit.clone(),
    commit.commit_sha.clone(),
    author.clone(),
    true,
    final_state_override,
)?;
```

amend 走：

```rust
rewrite_authorship_after_commit_amend_with_snapshot(...)
```

`post_commit_with_final_state()` 在 `src/authorship/post_commit.rs`，主要做：

1. 根据 parent/base commit 打开 working log。
2. 更新 prompts/transcripts 到最新。
3. 从 checkpoint entries 和 INITIAL 组装 pathspecs。
4. 构造 `VirtualAttributions`：
   - 有 final snapshot 时用 `from_working_log_snapshot()`。
   - 无 snapshot 时用 `from_just_working_log()`。
   - 对 whitespace-only uncheckpointed path 可能走 blame-backed path。
5. 调 `to_authorship_log_and_initial_working_log()`。
6. 生成 `AuthorshipLog`。
7. 根据 prompt storage mode 处理 prompt：local、notes、default/CAS。
8. 写 Git Note 到 `refs/notes/ai`。
9. 写 stats/metrics。
10. 写新的 INITIAL / 清理旧 working log。

## 9. rewrite log append 时机

daemon 有意在 authorship 处理成功后才 append rewrite log：

```rust
repo.storage.append_rewrite_event(rewrite_event.clone())?;
```

这样做可以避免失败的 rewrite 被永久标记为 processed。

也就是说：

- authorship note 成功写入后，才记录 rewrite event 已处理。
- 如果中途失败，daemon 可以在后续 cycle/retry 中重新处理。

## 10. 前台 stats 等待逻辑

commit 前台进程不会负责生成 note，但可能短暂等待 daemon 结果显示 stats。

入口：

```rust
maybe_show_async_post_commit_stats()
```

行为：

- dry-run 跳过。
- `--porcelain` / `--quiet` / `-q` / `--no-status` 跳过。
- 非 TTY 跳过。
- 默认如果 `GIT_AI_WAIT_COMMIT_STATS != 1`，直接提示跳过 stats。
- 如果启用等待：
  - 每 25ms poll `refs/notes/ai`。
  - 默认最多 500ms。
  - 测试环境最多 20s。
  - 找到 note 后计算并显示 stats。

因此 daemon 模式下用户可能看到：

- commit 已经成功返回。
- authorship note 还在后台生成。
- 稍后 `git-ai stats` / `git-ai blame` 才能看到结果。

## 11. family sequencer 保证同 repo 顺序

daemon 内部用 family sequencer 保证同一个 repo family 的命令和 checkpoint 按顺序执行。

核心结构在 `src/daemon.rs`：

```rust
enum FamilySequencerEntry {
    PendingRoot,
    ReadyCommand(Box<NormalizedCommand>),
    Checkpoint { ... },
    Canceled,
}
```

处理函数：

```rust
drain_ready_family_sequencer_entries_locked()
```

它会：

- 遇到 `PendingRoot` 就停，避免乱序。
- 对 `ReadyCommand`：route command，再 apply side effects。
- 对 `Checkpoint`：apply checkpoint side effect，更新 watermarks，删除 captured checkpoint 文件。

这就是 daemon 模式下 command event 和 checkpoint request 能按 repo 顺序串行化的原因。

## 12. captured checkpoint / live checkpoint 与 commit 的关系

除了 commit replay，daemon 还处理 AI 工具的 checkpoint 请求。

控制协议包含：

- `CheckpointRunRequest::Live`
- `CheckpointRunRequest::Captured`

入口：

```rust
ingest_checkpoint_payload()
```

实际执行：

```rust
apply_checkpoint_side_effect()
```

两种路径：

### Live checkpoint

```rust
checkpoint::run(...)
```

daemon 在自己进程里实时读工作区执行 checkpoint。通常 `wait=true`，调用方阻塞等结果。

### Captured checkpoint

```rust
checkpoint::execute_captured_checkpoint(...)
```

调用方已经把文件快照落盘到 async checkpoint blob 目录，daemon 只 replay 这个 snapshot。captured checkpoint 不依赖当前工作区状态，所以更适合 fire-and-forget。

相关函数在 `src/commands/checkpoint.rs`：

- `prepare_captured_checkpoint()`
- `execute_captured_checkpoint()`

## 13. 同步模式与 daemon 模式对比

### 同步 wrapper 模式

```text
git wrapper
  → run_pre_command_hooks()
    → commit_pre_command_hook()
      → pre_commit::pre_commit()
        → checkpoint::run()
  → real git commit
  → run_post_command_hooks()
    → commit_post_command_hook()
      → repository.handle_rewrite_log_event()
        → post_commit / rewrite authorship
```

### daemon / async 模式

```text
git wrapper
  → read pre HEAD
  → send WrapperPreState
  → real git commit with Trace2 invocation id
  → read post HEAD
  → send WrapperPostState
  → optionally wait for stats/note

daemon
  → receive Trace2
  → normalize command
  → overlay wrapper pre/post state
  → SemanticEvent::CommitCreated
  → RewriteLogEvent::Commit
  → sync_pre_commit_checkpoint_for_daemon_commit()
  → post_commit_with_final_state()
  → write refs/notes/ai
  → append rewrite_log
```

## 14. 前台与后台职责总览

| 操作 | 位置 | 模式 |
| --- | --- | --- |
| argv[0] 分发 | `src/main.rs` | 前台/同步 |
| async_mode 判断 | `src/commands/git_handlers.rs` | 前台/同步 |
| 读取 pre-state | `src/commands/git_handlers.rs` | 前台/同步 |
| 发送 `WrapperPreState` | daemon control socket | 前台/同步 socket 写入 |
| proxy 到真实 `git commit` | `src/commands/git_handlers.rs` | 前台/同步阻塞 |
| 读取 post-state | `src/commands/git_handlers.rs` | 前台/同步 |
| 发送 `WrapperPostState` | daemon control socket | 前台/同步 socket 写入 |
| 可选轮询 note/stats | `maybe_show_async_post_commit_stats()` | 前台/同步 |
| 接收 Trace2 | daemon trace listener | 后台/异步 |
| Trace2 归一化 | `src/daemon/trace_normalizer.rs` | 后台/异步 |
| overlay wrapper state | `src/daemon.rs` | 后台/异步 |
| 生成 `SemanticEvent` / `RewriteLogEvent` | daemon analyzer / `rewrite_events_from_semantic_events()` | 后台/daemon 内同步 |
| synthetic pre-commit replay | `sync_pre_commit_checkpoint_for_daemon_commit()` | 后台/daemon 内同步 |
| post-commit attribution | `post_commit_with_final_state()` | 后台/daemon 内同步 |
| 写 `refs/notes/ai` | `src/authorship/post_commit.rs` | 后台/daemon 内同步 |
| append rewrite log | `src/daemon.rs` | 后台/daemon 内同步 |

## 15. 关键边界条件

- daemon 模式下，foreground wrapper 不执行 `commit_pre_command_hook()`；如果有人说“pre-commit 会执行”，应理解为 daemon 的 synthetic pre-commit checkpoint replay。
- old core hook binary 已 sunset；旧 hook symlink 只打印提示并退出 0。
- active bash AI context 会影响 replay attribution：有上下文时归因给 AI agent；没有上下文时归为 synthetic Human checkpoint。
- wrapper state overlay 有等待超时；超时后 daemon 使用内部 state。
- authorship note 生成是异步的；commit 成功返回时 note 可能尚未写入。
- read-only invocation 会直接 proxy，并且可能 suppress Trace2。
- `clone` / `init` 不发送 wrapper invocation id，因为目标 repo 状态在命令前不存在或不可靠。
