# Windows async `git-ai status` 挂起验证指南（2026-04-22）

> 本文档用于验证这样一个假设：在 Windows 10 虚拟机上，`git-ai status` 在 async mode 下更容易挂起，主因不是 `status` 业务逻辑本身，而是 async mode 额外引入的 Windows named pipe / daemon 句柄，与内部大量采用 piped stdio 的 git 子进程执行模型叠加，放大了 Git for Windows 在 pipe / TTY 句柄探测上的挂起概率。

相关背景材料：

- `docs/fixes/2026-04-21-windows-spawn-hang-analysis.md`
- `docs/fixes/windows-spawn-hang-fix.md`

## 1. 验证目标

最小化验证以下现象是否同时成立：

1. `sync` 模式基本不挂，`async` 模式明显更容易挂。
2. 挂住时，`git-ai status` 下面存在多个未退出的 `git.exe`。
3. 挂住的 `git.exe` 句柄或线程栈中能看到 `NamedPipe` / `ConDrv` / `ReadFile` / `NtQueryObject` 等 Windows 管道或控制台线索。
4. Windows Terminal 相比直接打开的 `cmd.exe` / `powershell.exe`，复现率更低。

如果 1-4 同时满足，则强烈支持“async mode 额外引入的 Windows pipe 句柄环境是主放大器”这一判断。

## 2. 实验前提

- 在同一台 Windows 10 VM 上完成全部对照。
- 使用同一个仓库、同一批未提交改动 / checkpoint 状态。
- 每组命令至少跑 3 次，避免偶然误差。
- 挂住时先不要立刻杀进程，先采样进程树、句柄和线程栈。

## 3. 快速对照实验

建议做 4 组：

### 3.1 在 `cmd.exe` 或 `powershell.exe` 中

```bat
set GIT_AI_ASYNC_MODE=false
git-ai status
```

```bat
set GIT_AI_ASYNC_MODE=true
git-ai status
```

### 3.2 在 Windows Terminal 中

```bat
set GIT_AI_ASYNC_MODE=false
git-ai status
```

```bat
set GIT_AI_ASYNC_MODE=true
git-ai status
```

### 3.3 每组记录

- 是否挂住
- 是否出现多个内部 git 进程长时间不退出
- 是否需要等 timeout 才被回收
- 后台残留 `git.exe` 的数量

## 4. Process Explorer 使用步骤

用于回答两个问题：

1. 谁挂住了？
2. 挂住的 `git.exe` 此刻持有哪些句柄、卡在哪个线程栈？

### 4.1 启动与界面准备

1. 以管理员身份运行 `procexp64.exe`。
2. 菜单中开启：
   - `View -> Show Lower Pane`
   - `View -> Lower Pane View -> Handles`
3. 在主列表中建议显示这些列：
   - `PID`
   - `Parent PID`
   - `Threads`
   - `Handles`
   - `CPU`

### 4.2 复现挂起

1. 保持 Process Explorer 打开。
2. 在更容易复现的终端（通常先用 `cmd.exe` 或 `powershell.exe`）运行：

   ```bat
   set GIT_AI_ASYNC_MODE=true
   git-ai status
   ```

3. 一旦 CLI 卡住，先不要杀进程。

### 4.3 找到挂住的 `git.exe`

1. 回到 Process Explorer。
2. 在进程树中定位：
   - `git-ai.exe`
   - 它下面的一个或多个 `git.exe`
3. 重点观察：
   - 哪些 `git.exe` 长时间存在且 CPU 很低
   - 哪些 `git.exe` 的线程数/句柄数异常

### 4.4 查看句柄

#### 方法 A：直接看下方面板

1. 选中一个挂住的 `git.exe`。
2. 在 Lower Pane 的 Handles 视图里查找这些关键词：
   - `NamedPipe`
   - `\pipe\`
   - `git-ai`
   - `ConDrv`

#### 方法 B：全局搜索

1. 按 `Ctrl+F`。
2. 依次搜索：
   - `git-ai`
   - `\pipe\`
   - `ConDrv`
   - `NamedPipe`
3. 观察结果是否落在挂住的 `git.exe` 上。

### 4.5 查看线程栈

1. 双击挂住的 `git.exe`。
2. 打开 `Threads` 标签。
3. 选一个长时间存活、CPU 接近 0 的线程。
4. 点击 `Stack`。

重点看栈里是否出现：

- `ReadFile`
- `WaitForSingleObject`
- `PeekNamedPipe`
- `NtQueryObject`
- `KERNELBASE`
- `ntdll`

### 4.6 建议保存的证据

- `git-ai.exe -> git.exe` 的进程树截图
- 挂住的 `git.exe` 的 Handles 截图
- 挂住线程的 Stack 截图

## 5. ProcMon 使用步骤

用于回答：

> 挂住前最后一批系统调用主要碰到了什么？是普通文件、pipe，还是控制台设备？

### 5.1 启动与清空现场

1. 以管理员身份运行 `Procmon.exe`。
2. 点击工具栏放大镜，先停止采集。
3. 按 `Ctrl+X` 清空已有事件。

### 5.2 第一轮过滤

打开 `Filter -> Filter...`，加入：

- `Process Name` `is` `git.exe` -> `Include`
- `Process Name` `is` `git-ai.exe` -> `Include`

可选：勾选 `Filter -> Drop Filtered Events` 降低噪音。

### 5.3 开始抓取并复现

1. 再次点击放大镜开始采集。
2. 回终端复现：

   ```bat
   set GIT_AI_ASYNC_MODE=true
   git-ai status
   ```

3. 一旦挂住，立刻回到 ProcMon。
4. 点击放大镜停止采集。

### 5.4 锁定挂住的 PID

1. 打开 `Tools -> Process Tree`。
2. 找到对应的：
   - `git-ai.exe`
   - 其下挂住的 `git.exe`
3. 记下挂住的 `git.exe` 的 PID。

### 5.5 第二轮过滤：只看这个 PID

打开 `Filter -> Filter...`，再加：

- `PID` `is` `<挂住的 git.exe PID>` -> `Include`

### 5.6 第三轮过滤：看 pipe / 控制台线索

建议二选一反复切换看：

#### 看 pipe

- `Path` `contains` `\pipe\` -> `Include`

#### 看控制台

- `Path` `contains` `ConDrv` -> `Include`

> 建议顺序：先按 PID 缩小，再临时切换 `\pipe\` 或 `ConDrv`，避免结果过宽。

### 5.7 看哪些列

重点列：

- `Time of Day`
- `Process Name`
- `PID`
- `Operation`
- `Path`
- `Result`
- `Duration`

重点操作：

- `CreateFile`
- `ReadFile`
- `WriteFile`
- `QueryInformationFile`
- `CloseFile`

### 5.8 看单条事件细节

双击事件后重点看：

- `Event` 标签
- `Stack` 标签

关注：

- 是否访问了 `\Device\NamedPipe\...`
- 是否访问了 `\Device\ConDrv\...`
- 某些 `ReadFile` / `CreateFile` 的耗时是否异常长

### 5.9 建议保存的证据

- 按挂住 PID 过滤后的最后 20 条事件截图
- 同一 PID 下 `\pipe\` 过滤结果截图
- 同一 PID 下 `ConDrv` 过滤结果截图（如果有）

## 6. 结果判读

### 6.1 强支持 async pipe 假设的信号

如果同时看到以下现象，基本就很有说服力：

- `sync` 基本不挂，`async` 明显更容易挂。
- 挂住时 `git-ai.exe` 下面挂着多个 `git.exe`。
- 挂住的 `git.exe` 的 Handles 中能看到：
  - `NamedPipe`
  - `git-ai` 相关 pipe
  - `ConDrv`
- 线程栈中出现：
  - `ReadFile`
  - `NtQueryObject`
  - `PeekNamedPipe`
- ProcMon 显示挂住前最后一批事件主要集中在：
  - `\pipe\...`
  - `\Device\NamedPipe\...`
  - `\Device\ConDrv\...`

### 6.2 三种常见结果

#### 情况 A

- `sync` 正常
- `async` 挂
- 挂住的 `git.exe` 有额外 pipe / named pipe 痕迹

解释：强支持“async mode 额外引入的 pipe 句柄环境是主放大器”。

#### 情况 B

- `sync` 和 `async` 都挂

解释：不能只归因于 async；还要转向仓库状态、Git for Windows 版本、VM 环境或更底层的 Windows / 控制台问题。

#### 情况 C

- 只有 `cmd.exe` / `powershell.exe` 容易挂
- Windows Terminal 明显不容易挂

解释：更支持“控制台宿主 / ConPTY 时序差异是放大器”，而不是 git-ai 代码本身走了不同逻辑分支。

## 7. 需要强调的局限

- ProcMon 往往更适合看“挂住前最后发生了什么”，不一定能直接显示“当前还没返回的那个内核调用”。
- Process Explorer 更适合看“此刻卡在哪”：
  - 进程树
  - 句柄
  - 线程栈
- 因此两个工具建议配合使用，而不是只依赖其中一个。

## 8. 最少需要回传的材料

如果要继续做根因判断，至少保留以下四样：

1. Process Explorer 的进程树截图
2. 挂住的 `git.exe` 的 Handles 截图
3. 挂住线程的 Stack 截图
4. ProcMon 中按挂住 PID + `\pipe\` 或 `ConDrv` 过滤后的最后一批事件截图

有了这四样，一般就足够判断问题更接近：

- async named pipe / Git for Windows pipe 探测挂起
- 还是其他 Windows / VM / Git 环境层面的阻塞
