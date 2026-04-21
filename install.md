# 安装

执行 `install.ps1` 脚本，脚本执行后需要重新打开的终端程序，重新加载环境变量。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command ./install.ps1
```

## 非管理员权限

如果是非管理员权限，在执行安装脚本后，需要手动配置一下，使用长鑫安装助手中软件管理中的**修改系统环境变量**，将 `git-ai` 的路径配置到系统环境变量 `PATH` 的最前面。

`git-ai` 的安装路径在当前用户目录下的 `.git-ai` 目录。


![install-pic1](assets/docs/install-pic1.png)

![install-pic2](assets/docs/install-pic2.jpeg)

![install-pic3](assets/docs/install-pic3.jpeg)

环境变量配置后，重新打开 `cmd` 或 `powershell`，打印输出 `PATH` 环境变量，查看 `git-ai` 的路径是否在 `git` 路径的前面。

```cmd
echo %PATH%
```

```powershell
$env:PATH
```

输出示例：

```
...;C:\Users\vendor.mrdit.dk03\.git-ai\bin;...C:\Program Files\Git\cmd;...
```

## 验证安装

### git-ai 程序验证

执行 `git-ai -v` 验证 git-ai 是否安装成功，如果正常输出版本号则安装成功。

命令：

```cmd
git-ai -v
```

输出示例：

```
1.1.22
```

执行 `where.exe git` 命令验证是否使用的是 `git-ai` 的 `git` 程序，如果第一行输出的是 `git-ai` 路径下的 `git` 就代表没问题。

命令：

```cmd
where.exe git
```

输出示例：

```
C:\Users\vendor.mrdit.dk03\.git-ai\bin\git.exe
C:\Program Files\Git\cmd\git.exe

```

### Bash 配置

查看当前用户目录下 `.basrc` 文件中是否到配置 `git-ai` 的环境变量，因为 **claude code cli** 在 **Windows** 系统中使用的终端程序是 `git-bash`，会加载 `bash` 相关的配置，需要保证 `git-ai` 的安装路径在 `PATH` 的最前面，保证执行的是 `git-ai` 的 `git` 程序

```powhershell
cat ~\.bashrc
```

命令执行后查看输出中是否有以下内容

```bashrc
export PATH="$HOME/.git-ai/bin:$PATH"
```

## 使用

# AI Blame

使用 `git-ai blame <file>` 可以查看文件中哪些是 AI 生成，哪些是人为写入的。 

```bash
git-ai blame /src/log_fmt/authorship_log.rs
```

```bash
cb832b7 (Aidan Cunniffe 2025-12-13 08:16:29 -0500  133) pub fn execute_diff(
cb832b7 (Aidan Cunniffe 2025-12-13 08:16:29 -0500  134)     repo: &Repository,
cb832b7 (Aidan Cunniffe 2025-12-13 08:16:29 -0500  135)     spec: DiffSpec,
cb832b7 (Aidan Cunniffe 2025-12-13 08:16:29 -0500  136)     format: DiffFormat,
cb832b7 (Aidan Cunniffe 2025-12-13 08:16:29 -0500  137) ) -> Result<String, GitAiError> {
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  138)     // Resolve commits to get from/to SHAs
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  139)     let (from_commit, to_commit) = match spec {
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  140)         DiffSpec::TwoCommit(start, end) => {
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  141)             // Resolve both commits
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  142)             let from = resolve_commit(repo, &start)?;
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  143)             let to = resolve_commit(repo, &end)?;
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  144)             (from, to)
fe2c4c8 (claude         2025-12-02 19:25:13 -0500  145)         }
```