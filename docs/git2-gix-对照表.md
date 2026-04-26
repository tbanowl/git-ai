# 生产代码 git2/gix 迁移对照表

本文档覆盖整个 `src/` 生产代码中的 Git CLI 调用，同时标注每个调用迁移到 `git2`（libgit2 Rust 绑定）或 `gix` 的大致替换方案、预估性能收益，以及两条路线各自的迁移难度。

## 快速导航

- **优先迁 git2**：先看“推荐落地顺序”里的 **第一批** / **第一批延伸**，以及“未迁移（下一步候选）”里的 **高优先级（优先 git2）**。
- **优先迁 gix**：先看 **第三十一、哪些条目更适合 gix 而非 git2**，再回到表格中的 `gix 替换` / `gix 迁移难度` 两列。
- **保留 CLI**：先看“推荐落地顺序”里的 **保留 CLI**，以及“未迁移（下一步候选）”之后的 **不建议迁移（保留 CLI）**。

## 列定义

| 列名 | 含义 |
|---|---|
| **文件** | 源文件相对路径 |
| **函数** | 发起 CLI 调用的函数名 |
| **执行的命令** | 实际执行的 git 子命令（关键参数） |
| **git2可直接实现** | `是` = 有直接对应的 git2 API；`部分` = 能做但有语义差异或需额外工作；`否` = git2 不适合替代 |
| **git2 替换** | 推荐的 git2 API 或方案 |
| **用途** | 该调用的业务目的 |
| **Win10 VM 性能对比** | 在 Windows 10 VM 环境下的性能预估（见下方假设） |
| **git2 迁移难度** | 迁移到 git2 的工程复杂度 |
| **当前状态** | `✅ 已迁移` = 当前源码已使用 git2 进程内实现，不再走 CLI；留空 = 尚未迁移，仍在调 git CLI |
| **gix 替换** | 推荐的 gix / gix-index / gix-config API 或方案；留空 = 当前无明确、值得单列的 gix 方案 |
| **gix 迁移难度** | 迁移到 gix 的工程复杂度；留空 = 当前无明确、值得单列的 gix 迁移方案 |

> **当前状态列说明**：标记为 `✅ 已迁移` 的行表示该函数在当前代码库中已通过 `git2`/libgit2 进程内 API 实现，不再派生 `git` 子进程。该列基于 `src/git/repository.rs` 中的实际实现确认，会随代码演进过时，请以源码为准。
>
> **gix 替换 / gix 迁移难度列说明**：这两列只在有明确证据表明 `gix` / `gix_index` / `gix_config` 比 `git2` 更贴近当前需求时填写。内容风格与 `git2 替换` / `git2 迁移难度` 保持一致，写“可大致采用的方案”，不表示该项已经迁移到 gix。

## 性能假设

> **Windows 10 VM 环境下，每次原生 `git` CLI 调用（`Command::new("git")`）的固定开销通常 >500 ms。**
>
> 这个数字来自进程创建 + DLL 加载 + git 初始化在虚拟化环境中的累积延迟。在裸机 Linux/macOS 上同一调用可能只需 20-50 ms，但在 Windows VM 中会显著放大。
>
> 因此：
> - 任何**高频调用**（循环内反复起进程、遍历 commit 时逐个读取元数据）在 VM 上收益巨大
> - **低频单次调用**（如启动时 discover 仓库）收益有限
> - `git2` 通过进程内对象库访问，将固定开销从 ">500 ms/次" 降到 "<1 ms/次"
> - 下表性能对比列用 `>500ms → <1ms` 表示"从 CLI 变为 git2 的收益"

---

## 一、`src/git/repository.rs` — 对象/提交/ref 只读查询

这是 CLI 调用最密集的文件，也是 git2 迁移收益最大的模块。

### P0：强烈建议迁移（高频 + 只读 + git2 直接对应）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `revparse_single()` | `rev-parse --verify <spec>` | 是 | `Repository::revparse_single()` | 将 revspec 字符串解析为对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Object::peel_to_commit()` | `rev-parse --verify <oid>^{commit}` | 是 | `revparse_single()` + `peel_to_commit()` | 把对象 peel 成 commit | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::tree()` | `rev-parse --verify <oid>^{tree}` | 是 | `Commit::tree()` / `tree_id()` | 取 commit 的 tree OID | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::parent()` | `rev-parse <oid>^N` | 是 | `Commit::parent(n)` | 取第 N 个 parent | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::parents()` | `show -s --format=%P` | 是 | `Commit::parents()` / `parent_ids()` | 取所有 parent OID | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::summary()` | `show -s --format=%s` | 是 | `Commit::summary()` | commit 消息首行 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::body()` | `show -s --format=%b` | 是 | `Commit::body()` | commit 消息正文 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::author()` | `show -s --format=%an%n%ae%n%aI` | 是 | `Commit::author()` | commit 作者信息 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Commit::committer()` | `show -s --format=%cn%n%ce%n%cI` | 是 | `Commit::committer()` | commit 提交者信息 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Repository::merge_base()` | `merge-base A B` | 是 | `Repository::merge_base()` | 两个 commit 的 merge base | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `CommitRange::length()` | `rev-list --count A..B` | 是 | `Repository::revwalk()` + count | 范围内 commit 数量 | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `CommitRange::into_iter()` | `rev-list A..B` | 是 | `Repository::revwalk()` | 遍历范围内的 commit | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `CommitRange::is_valid()` | 多次 `merge-base --is-ancestor` | 是 | `graph_descendant_of()` / `merge_base()` | 校验 commit range 有效性 | N×>500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `parent_on_refname()` | 循环内 `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 找到在某 ref 上可达的 parent | N×>500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | ~~`Commit::first_parent_on_ref()`~~ | ~~`rev-parse --verify --symbolic-full-name` + `merge-base --is-ancestor`~~ | 是 | `find_reference()` + `graph_descendant_of()` | ~~找到在某 ref 上的 first parent~~ | N×>500ms → <1ms | 中 | ⛔ 已删除，当前源码中不存在 |

### P1：建议迁移（收益中高）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `Reference::shorthand()` | `rev-parse --abbrev-ref <ref>` | 是 | `Reference::shorthand()` | ref 的短名 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::target()` | `rev-parse <ref>` | 是 | `Reference::target()` | ref 指向的 OID | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::peel_to_blob()` | `rev-parse --verify <ref>^{blob}` | 是 | `find_reference` + peel | ref peel 到 blob | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::peel_to_commit()` | `rev-parse --verify <ref>^{commit}` | 是 | `find_reference` + peel | ref peel 到 commit | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `head()` | `symbolic-ref HEAD` | 是 | `Repository::head()` | 获取 HEAD 指向的 ref 名 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_reference()` | `show-ref --verify -s` | 是 | `Repository::find_reference()` | 查找指定 ref | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `references()` | `for-each-ref --format=%(refname)` | 是 | `Repository::references()` | 枚举所有 ref | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `new_infer_refname()` | `for-each-ref --points-at` | 部分 | `Repository::references()` + target 过滤 | 推断 refname | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `remote_head()` | `symbolic-ref refs/remotes/.../HEAD` | 部分 | `find_reference()` + symbolic target | 远程 HEAD | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `upstream_remote()` | `branch --show-current` + config | 部分 | `head()` + `branch_upstream_remote()` | 上游 remote 名 | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `object_type()` | `cat-file -t <oid>` | 是 | `find_object()` + `ObjectType` | 查对象类型 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Blob::content()` | `cat-file blob <oid>` | 是 | `find_blob()` + `Blob::content()` | 读 blob 内容 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_commit()` | `cat-file -t` 后校验 | 是 | `Repository::find_commit()` | 查找 commit 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_blob()` | `cat-file -t` 后校验 | 是 | `Repository::find_blob()` | 查找 blob 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_tree()` | `cat-file -t` 后校验 | 是 | `Repository::find_tree()` | 查找 tree 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `get_file_content()` | `show <commit>:<path>` | 是 | `find_commit` → `tree` → `get_path` → blob | 读指定 commit 的文件 | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `Tree::get_path()` | `ls-tree -z -r <tree> -- <path>` | 是 | `Tree::get_path()` | 从 tree 中找路径 | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `get_all_staged_files_content()` | 并发 `git show :<path>` | 部分 | `Repository::index()` → blob OID → blob content | 批量读 staged 文件 | N×>500ms → <1ms | 中 | ✅ 已迁移 | `gix_index::File::at()` + stage-0 entry → blob OID/内容 | 中 |
| `repository.rs` | `resolve_git_var_identity()` | `var GIT_COMMITTER_IDENT / GIT_AUTHOR_IDENT` | 是 | `git2::Config::get_string()` + `Repository::config()` | 获取 git 身份信息 | >500ms → <1ms | 低 | |
| `repository.rs` | `git_version()` | `--version` | 是 | `git2::version()` / `libgit2_version()` | 获取 git 版本号 | >500ms → <1ms | 低 | |
| `repository.rs` | `diff_index_filenames()` | `diff --name-only -z --no-renames` | 否 | — | 列 index 中变更文件名 | 不适用 | — | |
| `repository.rs` | `commit_range_on_branch()` | `rev-parse --verify --symbolic-full-name` + `log --format=%H --reverse` | 是 | `find_reference()` + `revwalk()` | 获取分支上的 commit 范围 | N×>500ms → <1ms | 中 | |

### P2：可迁移但收益一般

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `remotes()` | `remote` | 是 | `Repository::remotes()` | 列 remote 名 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `remotes_with_urls()` | `remote -v` | 部分 | `remotes()` + `find_remote()` + URL | 列 remote 及 URL | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `resolve_author_spec()` | `rev-list --all --author=` + `show` | 部分 | `revwalk()` + 手动过滤 author | 按 author 名查找 commit | >500ms → <1ms | 中高 | |
| `repository.rs` | `is_bare_repository()` | `rev-parse --is-bare-repository` | 是 | `Repository::is_bare()` | 判断是否 bare 仓库 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_repository()` | `rev-parse --git-dir --git-common-dir --show-toplevel` | 部分 | `Repository::discover()` / `open_ext()` | 发现/打开仓库 | >500ms → <1ms | 中 | ✅ 已迁移 | `gix::discover()` / `open` + worktree/git-dir 组装 | 中 |

### P3：能做但不建议第一批

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `blob()` | `hash-object -w --stdin` | 是 | `Repository::blob()` | 写入 blob 对象 | >500ms → <1ms | 中 | |
| `repository.rs` | `reference()` | `update-ref --stdin --create-reflog` | 部分 | `Repository::reference()` | 创建/更新 ref | >500ms → <1ms | 中高 | |
| `repository.rs` | `commit()` | `commit-tree` + `update-ref` | 部分 | `Repository::commit()` + refs 更新 | 创建 commit | >500ms → <1ms | 高 | |
| `repository.rs` | `fetch_branch()` | `fetch remote branch` | 部分 | `Remote::fetch()` | 拉取远程分支 | 收益不确定 | 高 | |

### 建议保留 CLI

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `list_commit_files()` | `diff-tree --name-only -r -z` | 否 | — | 列 commit 变更文件 | 不适用 | — | |
| `repository.rs` | `diff_added_lines()` | `diff -U0 --find-renames=1%` | 否 | — | 取 diff 新增行 | 不适用 | — | |
| `repository.rs` | `diff_added_lines_with_deleted_count()` | `diff -U0` | 否 | — | 取 diff 新增行及删除计数 | 不适用 | — | |
| `repository.rs` | `diff_changed_files()` | `diff --name-only -z` | 否 | — | 列 diff 变更文件 | 不适用 | — | |
| `repository.rs` | `diff_workdir_added_lines()` | `diff -U0` | 否 | — | 工作目录 diff 新增行 | 不适用 | — | |
| `repository.rs` | `diff_workdir_added_lines_with_insertions()` | `diff -U0 --no-renames` | 否 | — | 工作目录 diff 新增行+插入数 | 不适用 | — | |
| `repository.rs` | `merge_trees_favor_ours()` | `merge-tree --write-tree -X ours` | 部分 | — | 合并 tree (ours 策略) | 不适用 | — | |

---

## 二、`src/git/refs.rs` — Notes 读写与 ref 操作

notes 系统是该文件的核心。git2 对 notes 的支持有限，大部分 notes 操作需要保留 CLI。

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `refs.rs` | `batch_read_blobs_with_oids()` | `cat-file --batch` (stdin 批量) | 是 | `Repository::find_blob()` 批量 | 批量读 blob 内容 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + blob 数据读取 | 中 |
| `refs.rs` | `note_blob_oids_for_commits()` | `cat-file --batch-check` (stdin 批量) | 部分 | git2 notes API 有限，需手动查 tree | 批量查 commit 的 note blob OID | 1×>500ms → N×<1ms | 高 | | `gix refs/tree/blob` 遍历 notes 命名空间 | 高 |
| `refs.rs` | `notes_add_batch()` | `rev-parse --verify refs/notes/ai` + `fast-import` | 否 | git2 无 fast-import 等价 | 批量添加 notes | 不适用 | — | | `—` | — |
| `refs.rs` | `notes_add_blob_batch()` | `rev-parse --verify refs/notes/ai` + `fast-import` | 否 | 同上 | 批量关联已有 blob 为 notes | 不适用 | — | | `—` | — |
| `refs.rs` | `show_authorship_note()` | `notes --ref=ai show <sha>` | 部分 | 需查 notes tree 再读 blob | 读取单个 commit 的 note | >500ms → <1ms | 中 | | `gix refs/tree/blob` 查 note blob 后读取内容 | 中 |
| `refs.rs` | `ref_exists()` | `show-ref --verify --quiet <ref>` | 是 | `Repository::find_reference()` | 检查 ref 是否存在 | >500ms → <1ms | 低 | ✅ 已迁移 | `gix ref lookup` | 低 |
| `refs.rs` | `merge_notes_from_ref()` | `notes --ref=ai merge -s ours` | 否 | git2 无 notes merge | 合并 notes ref | 不适用 | — | | `—` | — |
| `refs.rs` | `fallback_merge_notes()` | `fast-import --quiet --done` | 否 | git2 无 fast-import | fallback 合并 notes | 不适用 | — | | `—` | — |
| `refs.rs` | `list_all_notes()` | `notes --ref=ai list` | 部分 | 需遍历 notes tree | 列出所有 notes | >500ms → <1ms | 高 | | `gix refs/tree/blob` 遍历 notes tree | 高 |
| `refs.rs` | `rev_parse()` | `rev-parse <rev>` | 是 | `Repository::revparse_single()` | 解析 ref 到 SHA | >500ms → <1ms | 低 | | `gix revision parse` | 低 |
| `refs.rs` | `copy_ref()` | `update-ref <dest> <source>` | 是 | `Repository::reference()` | 复制 ref | >500ms → <1ms | 低 | ✅ 已迁移 | `gix ref lookup` + ref update/write | 中 |
| `refs.rs` | `grep_ai_notes()` | `grep -nI <pattern> refs/notes/ai` | 否 | git2 无 grep notes | 搜索 notes 内容 | 不适用 | — | | `—` | — |
| `refs.rs` | ~~`get_commit_authorships()`~~ | ~~`rev-list --no-walk --pretty=format:%H%n%an%n%ae`~~ | 是 | `Repository::find_commit()` + `Commit::author()` | ~~批量取 commit 作者信息~~ | >500ms → <1ms | 中 | ⛔ 已删除，当前源码中不存在 | `gix object lookup` + commit 元数据读取 | 中 |

---

## 三、`src/git/authorship_traversal.rs` — Authorship note 批量读取

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `authorship_traversal.rs` | `batch_read_blobs_with_oids()` | `cat-file --batch` (stdin) | 是 | `Repository::find_blob()` 批量 | 批量读 note blob 内容 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + blob 数据读取 | 中 |

> `authorship_traversal.rs` 里的 `get_notes_list()` 仅在 `#[cfg(test)]` 中使用，不属于生产代码。

---

## 四、`src/git/sync_authorship.rs` — Notes 同步（fetch/push）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `sync_authorship.rs` | `fetch_missing_notes_for_commits()` | `notes --ref=refs/notes/ai list` | 部分 | 同 refs.rs notes 枚举 | 检查哪些 commit 缺少 note | >500ms → <1ms | 高 | | `gix refs/tree/blob` 遍历 notes + commit map 对比 | 高 |
| `sync_authorship.rs` | `fetch_authorship_notes()` | `fetch --no-tags ... <remote> <refspec>` | 部分 | `Remote::fetch()` | 从远端拉取 notes | 网络 IO 主导，git2 未必更快 | 高 | | `—` | — |
| `sync_authorship.rs` | `push_authorship_notes()` | `push --quiet ... <remote> <refspec>` | 部分 | `Remote::push()` | 推送 notes 到远端 | 网络 IO 主导 | 高 | | `—` | — |
| `sync_authorship.rs` | `get_local_notes_map()` | `notes --ref=ai list` | 部分 | 遍历 notes tree | 枚举本地所有 notes | >500ms → <1ms | 高 | | `gix refs/tree/blob` 遍历 notes tree | 高 |
| `sync_authorship.rs` | `get_current_branch()` | `rev-parse --abbrev-ref HEAD` | 是 | `Repository::head()` + `shorthand()` | 获取当前分支名 | >500ms → <1ms | 低 | ✅ 已迁移 | `gix head/refname` 读取 | 低 |

---

## 五、`src/git/status.rs` — 工作目录状态

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `status.rs` | `get_staged_filenames()` | `diff --cached --name-only -z --no-renames` | 部分 | `gix_index` + `git2` HEAD tree diff | 获取已 staged 的文件列表 | >500ms → <1ms | 中 | ✅ 已迁移 | `gix::Repository::status()` / `gix-status` + `gix-index` staged 视图 | 中 |
| `status.rs` | `get_staged_and_unstaged_filenames()` | `status --porcelain=v2 -z` | 部分 | `git2::StatusOptions` + `repo.statuses()` | 获取所有变更文件 | >500ms → <1ms | 中高 | ✅ 已迁移 | `gix::Repository::status()` + index/worktree 枚举 | 中高 |
| `status.rs` | `status()` | `status --porcelain=v2 -z` [+ pathspecs] | 部分 | `git2::StatusOptions` + `repo.statuses()` | 带 pathspec 过滤的完整状态查询 | >500ms → <1ms | 中高 | | `gix::Repository::status()` + 自组装 `StatusEntry` / pathspec 后过滤 | 中高 |
| `repo_state.rs` | `read_head_state_for_worktree()` | `status --porcelain=v2 -z --branch`（语义等价目标） | 部分 | `gix` / 现有 head-state 读取路径 | 读取分支名、HEAD、detached 状态 | >500ms → <1ms | 中高 | | `gix::discover()` + 现有 head-state 元数据读取路径 | 中高 |

> `get_staged_filenames()` 和 `get_staged_and_unstaged_filenames()` 已通过 `gix_index` + `git2` 进程内实现，不再走 CLI。
> `status()` 仍使用 CLI `status --porcelain=v2 -z`，强依赖 porcelain v2 输出格式（含 rename/copy/unmerged/untracked 完整解析）。迁移到 git2 status API 后，需要自行组装等价的 `StatusEntry` 结构并保留 pathspec 过滤和 NFC 归一化逻辑。
> 该模块已在 index 层使用了 `gix_index`，可优先考虑沿用/扩展 `gix` 而非继续迁到 `git2`。

---

## 六、`src/git/diff_tree_to_tree.rs` — Tree diff

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `diff_tree_to_tree.rs` | `diff_tree_to_tree()` | `rev-parse --empty-tree` | 是 | `git2` 内置 empty tree hash | 获取空 tree OID | >500ms → 硬编码常量 | 低 | |
| `diff_tree_to_tree.rs` | `diff_tree_to_tree()` | `diff --raw -z --no-abbrev <old> <new>` | 部分 | git2 Diff API + tree walk | 对比两个 tree | >500ms → <1ms | 高 | | `gix-diff` tree diff + 自定义 raw 输出映射 | 高 |

> `diff --raw` 的输出格式被自定义 parser 依赖。迁移需要重写 parser 以适配 git2 diff delta 结构。

---

## 七、`src/authorship/rebase_authorship.rs` — Rebase 后 authorship 重写

### P0：高频只读

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `rebase_authorship.rs` | `walk_commits_to_base()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 校验 ancestor 关系 | >500ms → <1ms | 低 | ✅ 已迁移 | `gix revision graph` ancestor 判断 | 中 |
| `rebase_authorship.rs` | `walk_commits_to_base()` | `rev-list --topo-order --ancestry-path` | 是 | `Repository::revwalk()` + 过滤 | 枚举范围内的 commit | >500ms → <1ms | 中 | ✅ 已迁移 | `gix revision walk` + ancestry/topology 过滤 | 中高 |
| `rebase_authorship.rs` | `is_ancestor()` | ~~`merge-base --is-ancestor`~~ | 是 | `graph_descendant_of()` | ~~校验 ancestor 关系~~ | >500ms → <1ms | 低 | ⛔ 已删除，逻辑合并进 `walk_commits_to_base()`（已迁移） | `gix revision graph` ancestor 判断 | 中 |
| `rebase_authorship.rs` | `rev_list_ancestry_path()` | ~~`rev-list --topo-order --ancestry-path`~~ | 是 | `Repository::revwalk()` | ~~枚举祖先路径上的 commit~~ | >500ms → <1ms | 中 | ⛔ 已删除，逻辑合并进 `walk_commits_to_base()`（已迁移） | `gix revision walk` + ancestry/topology 过滤 | 中高 |
| `rebase_authorship.rs` | `get_tracked_paths()` | ~~`ls-tree -r -z --name-only`~~ | 是 | `Tree::iter()` | ~~获取 tree 中所有跟踪路径~~ | >500ms → <1ms | 中 | ⛔ 已删除，当前源码中不存在 | `gix tree traversal` | 中 |

### P1：批量读取

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `rebase_authorship.rs` | `batch_read_blobs_with_oids()` | `cat-file --batch` (stdin) | 是 | `Repository::find_blob()` 批量 | 批量读 blob 内容 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + blob 数据读取 | 中 |
| `rebase_authorship.rs` | `batch_read_blobs_with_oids_concurrent()` | `cat-file --batch` (stdin, 并行分片) | 是 | `Repository::find_blob()` 批量 | 并行批量读 blob 内容 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + blob 数据读取（并行分片） | 中 |
| `rebase_authorship.rs` | `get_committed_files_content_batch()` | `cat-file --batch` (stdin: `commit:path`) | 部分 | `find_commit()` → `tree` → `get_path()` → blob | 批量读指定 commit 的文件内容 | 1×>500ms → N×<1ms | 高 | | `gix revision/object/tree lookup` + `commit:path` 解析 | 高 |
| `rebase_authorship.rs` | `get_commit_metadata_batch()` | `cat-file --batch` (stdin: commit SHAs) | 是 | `Repository::find_commit()` 批量 | 批量取 commit 元数据 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + commit 元数据读取 | 中 |

### P2：diff-tree 操作

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `rebase_authorship.rs` | `diff_tree_combined()` | `diff-tree --stdin --raw -p -U0 --no-color --no-abbrev -r` | 部分 | git2 Diff API + tree walk | 批量对比 commit tree 变更+patch | 1×>500ms → N×<1ms | 高 | | `gix-diff` + patch/raw 双路映射 | 高 |
| `rebase_authorship.rs` | `tracked_file_blob_changed_in_range()` | `diff-tree --stdin -z --name-only -r` | 部分 | git2 tree diff API | 检测范围内文件 blob 是否变更 | 1×>500ms → N×<1ms | 中 | | `gix-diff` tree delta 枚举 | 中高 |
| `rebase_authorship.rs` | `collect_tracked_paths_in_range()` | `diff-tree --stdin --name-only -r -z` | 部分 | git2 tree diff API | 收集范围内的变更文件路径 | 1×>500ms → N×<1ms | 中 | | `gix-diff` tree delta → path 收集 | 中高 |

> `rebase_authorship.rs` 中的 `find_commit()` 调用经过 `repository.rs` 间接调用 CLI，迁移 `repository.rs` 后自动收益。
> `diff_tree_combined()` 同时获取 `--raw`（文件级别变更）和 `-p -U0`（行级 patch），是 rebase_authorship 中最复杂的单点调用。

---

## 八、`src/commands/blame.rs` — AI Blame

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `blame.rs` | `resolve_blame_abbrev_shas_batched()` | `rev-parse --short=N <sha>...` (批量) | 是 | `Oid::to_string()` 取前 N 位 | 批量缩写 SHA | >500ms → <1ms | 低 | ✅ 已迁移 | `gix object id` 缩写 / prefix 校验 | 低 |
| `blame.rs` | `blame_hunks_for_ranges()` | `blame --line-porcelain [-w] [-M] [-C...] [-L ...]` | 部分 | `Repository::blame()` + `BlameOptions` | 完整 blame 输出 | >500ms → <1ms | 高 | | `—` | — |

> blame 是整个项目中对 git CLI 依赖最重的单点调用之一。git2 有 `Repository::blame()` API，但 `--line-porcelain` 的完整输出格式、`-C`/`-M` 检测、`--ignore-rev`/`--ignore-revs-file`、`--since` 过滤等参数组合需要仔细逐一校验。`resolve_blame_abbrev_shas_batched()` 则是独立的小优化点，SHA 缩写可以纯字符串截断。

---

## 九、`src/commands/search.rs` — Prompt 搜索

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `search.rs` | `search_by_commit_range()` | `rev-list start..end` | 是 | `Repository::revwalk()` | 枚举范围内的 commit | >500ms → <1ms | 低 | ✅ 已迁移 | `gix revision walk` | 中 |

> `search_by_file()` 内部走的是 blame 系统，其 CLI 调用已计入 blame.rs。

---

## 十、`src/commands/log.rs` — git ai log

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `log.rs` | `handle_log()` | `git log --notes=ai [args...]` | 否 | — | 透传 `git log` 给用户 | 不可替代（需要 pager/颜色/全部参数） | — | | `—` | — |

> `handle_log()` 是一个纯代理：把用户参数原样传给 `git log --notes=ai`。这不需要迁移。它依赖 pager、颜色输出、用户自定义 format 等全部 git log 特性，git2 无法替代。

---

## 十一、`src/commands/git_handlers.rs` — Git 代理主入口

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `git_handlers.rs` | `handle_git()` / `run_git_with_hooks()` | `git -c core.hooksPath=... <subcmd>` | 否 | — | git 代理，注入 hooks 路径后执行真实 git | 不可替代（整个项目的核心分发机制） | — | | `—` | — |

> git 代理层是 git-ai 的架构根基。它拦截 `git` 调用、注入 `core.hooksPath`、转发给真实 git、再执行 post-hook。这必须保留 CLI。

---

## 十二、`src/commands/install_hooks.rs` — 安装 hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `install_hooks.rs` | `set_global_git_config_value()` | `git config --global <key> <val>` | 部分 | `git2::Config` | 设置全局 git 配置 | >500ms → <1ms | 低 | | `gix_config::File` 读取/写回全局配置 | 低 |

> 仅在安装/升级时调用，不是热路径，收益有限。

---

## 十三、`src/commands/continue_session.rs` — 会话继续

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `continue_session.rs` | `CommitInfo::from_commit_sha()` | `log -1 --format=%H\|\|\|%an\|\|\|%ai\|\|\|%s` | 是 | `find_commit()` + `Commit::author/summary` | 获取 commit 基本信息 | >500ms → <1ms | 低 | | `gix object lookup` + commit 元数据读取 | 中 |
| `continue_session.rs` | `CommitInfo::from_commit_sha()` | `log -1 --format=%B` | 是 | `find_commit()` + `Commit::message()` | 获取 commit 完整消息 | >500ms → <1ms | 低 | | `gix object lookup` + commit message 读取 | 中 |
| `continue_session.rs` | `get_git_status_info()` | `branch --show-current` | 是 | `Repository::head()` + `shorthand()` | 获取当前分支名 | >500ms → <1ms | 低 | | `gix head/refname` 读取 | 低 |
| `continue_session.rs` | `get_git_status_info()` | `log --oneline -5` | 是 | `Repository::revwalk()` + `Commit::summary()` | 获取最近 5 条 commit | >500ms → <1ms | 低 | | `gix revision walk` + commit summary 读取 | 中 |

> `continue_session.rs` 中还有启动 AI agent 进程的 `Command::new`（非 git），不涉及 git2 迁移。

---

## 十四、`src/commands/diff.rs` — diff 展示

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `diff.rs` | `resolve_commit()` | `rev-parse <rev>` | 是 | `Repository::revparse_single()` | 解析 rev 到 SHA | >500ms → <1ms | 低 | | `gix revision parse` | 低 |
| `diff.rs` | `get_diff_text()` | `diff -U0 --find-renames=1% --no-color` | 否 | — | 获取 diff 文本 | 不适用 | — | | `—` | — |
| `diff.rs` | `get_commit_diff()` | `show --format= --stat --patch --no-color` | 否 | — | 获取 commit 的 diff 展示 | 不适用 | — | | `—` | — |
| `diff.rs` | `get_commit_metadata()` | `show -s --no-notes --format=%an%x00%ae%x00%aI%x00%s%x00%B` | 是 | `find_commit()` + `Commit::author/summary/body` | 获取 commit 元数据（含完整消息） | >500ms → <1ms | 低 | | `gix object lookup` + commit 元数据读取 | 中 |

> `get_diff_text()` 和 `get_commit_diff()` 依赖完整 diff 输出格式，建议保留 CLI。`resolve_commit()` 和 `get_commit_metadata()` 为简单只读查询，可独立迁移。

---

## 十五、`src/commands/status.rs` — status 统计

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `status.rs` | (status stats) | `diff --numstat -z <commit> <pathspecs>` | 部分 | git2 Diff API + 统计 | 获取变更行数统计 | >500ms → <1ms | 中 | | `gix-diff` + 插入/删除统计聚合 | 中高 |

> `--numstat` 的输出格式（added\tdelted\tfilename）被后续代码解析。迁移需要用 git2 diff API 获取等价的插入/删除行数。

---

## 十六、`src/commands/hooks/rebase_hooks.rs` — Rebase hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `rebase_hooks.rs` | `walk_first_parent_commits()` (原 `get_first_parent_rev_list`) | `rev-list --first-parent --topo-order --max-count=N` | 是 | `Repository::revwalk()` + `FirstParent` | 取 first-parent 链上的 commit | >500ms → <1ms | 中 | | `gix revision walk` + first-parent 过滤 | 中高 |
| `rebase_hooks.rs` | `is_ancestor()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 校验 ancestor 关系 | >500ms → <1ms | 低 | | `gix revision graph` ancestor 判断 | 中 |

---

## 十七、`src/commands/hooks/cherry_pick_hooks.rs` — Cherry-pick hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `cherry_pick_hooks.rs` | `expand_commit_range()` (原 `expand_range_to_commits`) | `rev-list --reverse <range>` | 是 | `Repository::revwalk()` + 反转 | 展开范围为 commit 列表 | >500ms → <1ms | 低 | | `gix revision walk` + reverse 收集 | 中 |
| `cherry_pick_hooks.rs` | `resolve_commit_sha()` | `rev-parse <commit_ref>` | 是 | `Repository::revparse_single()` | 解析 commit ref 到 SHA | >500ms → <1ms | 低 | | `gix revision parse` | 低 |

---

## 十八、`src/commands/hooks/stash_hooks.rs` — Stash hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `stash_hooks.rs` | `has_stash_conflict()` | `status --porcelain=v2` | 部分 | git2 status API | 检测 stash 冲突 | >500ms → <1ms | 中 | | `gix::Repository::status()` + conflict 项枚举 | 中高 |
| `stash_hooks.rs` | `save_stash_authorship_log()` | `notes --ref=ai-stash add -` (stdin) | 部分 | 需写 notes tree | 保存 stash authorship 到 note | >500ms → <1ms | 高 | | `—` | — |
| `stash_hooks.rs` | `read_stash_note()` | `notes --ref=ai-stash show <sha>` | 部分 | 查 notes tree 再读 blob | 读取 stash 的 authorship note | >500ms → <1ms | 中 | | `gix refs/tree/blob` 查 note blob 后读取 | 中 |
| `stash_hooks.rs` | `resolve_stash_to_sha()` | `rev-parse <stash_ref>` | 是 | `Repository::revparse_single()` | 解析 stash ref 到 SHA | >500ms → <1ms | 低 | | `gix revision parse` | 低 |

> `stash_hooks.rs` 使用独立的 notes 命名空间 `refs/notes/ai-stash`，与主 notes (`refs/notes/ai`) 分离。

---

## 十九、`src/commands/hooks/fetch_hooks.rs` — Fetch hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `fetch_hooks.rs` | (fetch detection) | `reflog -1 --format=%H %gs` | 部分 | git2 reflog API (`Repository::reflog()`) | 检测 fetch 操作 | >500ms → <1ms | 中 | | `gix reflog` 读取最近一条记录 | 中 |

---

## 二十、`src/commands/hooks/update_ref_hooks.rs` — Update-ref hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `update_ref_hooks.rs` | `is_ancestor()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 校验 ancestor 关系 | >500ms → <1ms | 低 | | `gix revision graph` ancestor 判断 | 中 |

---

## 二十一、`src/commands/hooks/reset_hooks.rs` — Reset hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `reset_hooks.rs` | `is_ancestor()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 校验 ancestor 关系 | >500ms → <1ms | 低 | | `gix revision graph` ancestor 判断 | 中 |

---

## 二十二、`src/commands/prompts_db.rs` — Prompts 数据库

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `prompts_db.rs` | `reachable_commits()` | `rev-list --all` | 是 | `Repository::revwalk()` | 获取所有可达 commit | >500ms → <1ms | 低 | | `gix revision walk` | 中 |
| `prompts_db.rs` | `commit_dates_for()` (原 `commit_timestamps`) | `show -s --format=%H %ct` (批量) | 是 | `find_commit()` + `Commit::time()` | 批量取 commit 时间戳 | >500ms → <1ms | 中 | | `gix object lookup` + commit time 读取 | 中 |
| `prompts_db.rs` | `get_notes_list()` | `notes --ref=ai list` | 部分 | 遍历 notes tree | 列出所有 notes | >500ms → <1ms | 高 | | `gix refs/tree/blob` 遍历 notes tree | 高 |
| `prompts_db.rs` | `batch_read_blobs()` | `cat-file --batch` (stdin) | 是 | `Repository::find_blob()` 批量 | 批量读 blob 内容 | 1×>500ms → N×<1ms | 中 | | `gix object lookup` + blob 数据读取 | 中 |

> `prompts_db.rs` 中 `get_notes_list()` 和 `batch_read_blobs()` 与 `refs.rs` 中的同名函数功能类似，迁移策略可复用。

---

## 二十三、`src/commands/checkpoint_agent/bash_tool.rs` — Bash tool (checkpoint agent)

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `bash_tool.rs` | `get_git_dir()` | `rev-parse --git-dir` | 是 | `Repository::path()` / `Repository::discover()` | 获取 git 目录路径 | >500ms → <1ms | 低 | |
| `bash_tool.rs` | `git_status_fallback()` (原 `get_changed_files`) | `status --porcelain=v2 -z --untracked-files=all` | 部分 | git2 status API | 获取所有变更文件（含 untracked） | >500ms → <1ms | 中高 | | `gix::Repository::status()` + untracked files 枚举 | 中高 |

> `git_status_fallback()` 与 `status.rs` 中的调用类似，但增加了 `--untracked-files=all`，需要更完整的状态枚举。

---

## 二十四、`src/daemon.rs` — 守护进程

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `daemon.rs` | `is_ancestor_commit()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | daemon 中校验 ancestor 关系 | >500ms → <1ms | 低 | | `gix revision graph` ancestor 判断 | 中 |
| `daemon.rs` | (rebase subject matching) | `log --format=%s -1` | 是 | `find_commit()` + `Commit::summary()` | 获取 commit subject | >500ms → <1ms | 低 | | `gix object lookup` + commit summary 读取 | 中 |

---

## 二十五、`src/daemon/git_backend.rs` — Git 后端

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `git_backend.rs` | `repo_context()` | `symbolic-ref --quiet --short HEAD` | 是 | `Repository::head()` + `shorthand()` | 获取当前分支名 | >500ms → <1ms | 低 | | `gix head/refname` 读取 | 低 |
| `git_backend.rs` | `rev_parse_head()` | `rev-parse --verify HEAD` | 是 | `Repository::head()` + `peel_to_commit()` | 获取 HEAD SHA | >500ms → <1ms | 低 | | `gix head` + peel/commit id 读取 | 中 |

---

## 二十六、`src/api/client.rs` — API 客户端

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `client.rs` | `resolve_git_identity()` | `var GIT_COMMITTER_IDENT` | 是 | `Repository::config()` + `Config::get_string()` | 获取 git 身份信息 | >500ms → <1ms | 低 | | `gix_config::File` / repo config 读取 ident | 中 |

> 与 `repository.rs` 中的 `resolve_git_var_identity()` 功能类似。

---

## 二十七、`src/authorship/range_authorship.rs` — 范围 authorship 统计

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `range_authorship.rs` | (fetch remote refspec) | `fetch remote refspec` | 部分 | `Remote::fetch()` | 拉取远程分支 | 网络 IO 主导 | 高 | | `—` | — |
| `range_authorship.rs` | `get_range_diff_stats()` | `diff --numstat start..end` | 部分 | git2 Diff API + 统计 | 获取范围内的 diff 统计 | >500ms → <1ms | 中 | | `gix-diff` + 插入/删除统计聚合 | 中高 |

> `get_range_diff_stats()` 与 `commands/status.rs` 中的 `diff --numstat` 用法类似。

---

## 二十八、`src/mdm/ensure_git_symlinks.rs` — Git 符号链接

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `ensure_git_symlinks.rs` | `ensure_git_symlinks()` | `--exec-path` | 部分 | `git2::Repository::discover()` + 路径推导 | 获取 git exec 路径 | >500ms → <1ms | 低 | | `—` | — |

> 仅在安装时调用，冷路径。`--exec-path` 返回 git 的安装路径，git2 无直接等价，但可通过 `Repository::discover()` + 约定路径推导。

---

## 二十九、`src/ci/` — CI 集成（github.rs / gitlab.rs）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `ci/github.rs` | (CI clone) | `clone --branch <ref> <url> <dir>` | 部分 | `Repository::clone()` | CI 环境克隆仓库 | 网络 IO 主导 | 高 | | `—` | — |
| `ci/github.rs` | (CI fetch PR) | `fetch <url> pull/N/head:refs/github/pr/N` | 部分 | `Remote::fetch()` | CI 拉取 PR 分支 | 网络 IO 主导 | 高 | | `—` | — |
| `ci/gitlab.rs` | (CI clone) | `clone --branch <ref> <url> <dir>` | 部分 | `Repository::clone()` | CI 环境克隆仓库 | 网络 IO 主导 | 高 | | `—` | — |
| `ci/gitlab.rs` | (CI set-url) | `remote set-url origin <url>` | 是 | `Remote::set_url()` | 设置远程 URL | >500ms → <1ms | 低 | | `gix remote config` URL 更新 | 中 |
| `ci/gitlab.rs` | (CI fetch MR) | `fetch <url> refs/merge-requests/N/head:refs/gitlab/mr/N` | 部分 | `Remote::fetch()` | CI 拉取 MR 分支 | 网络 IO 主导 | 高 | | `—` | — |

> CI 模块仅在 CI 环境运行，不是热路径，且 `clone`/`fetch` 为网络 IO 主导，迁移收益极低。

---

## 三十、`src/authorship/ignore.rs` — .gitignore 检查

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | git2 迁移难度 | 当前状态 | gix 替换 | gix 迁移难度 |
|---|---|---|---|---|---|---|---|---|---|---|
| `ignore.rs` | (test-only) | `Command::new(git_cmd)` | — | — | 仅测试代码使用 | 不适用 | — | | `—` | — |

> `ignore.rs` 的生产代码路径不直接调用 git CLI，而是通过 git2 (已有 `test-support` feature) 进行 ignore 检查。CLI 调用仅在测试中。

---

## 推荐落地顺序（按 git2 / gix / CLI 三路决策）

### ✅ 第一批：最值（高频 + 只读 + git2 完美对应）— 已迁移

> 以下项目已在当前代码库中通过 git2 为主的进程内 API 实现，不再走 CLI。

1. ✅ `repository.rs` 中所有 `rev-parse --verify` / peel / ref resolve
2. ✅ `repository.rs` 中 `Commit::summary/body/author/committer/tree/parent/parents`
3. ✅ `repository.rs` 中 `merge_base` / `CommitRange::{length, into_iter, is_valid}` / `parent_on_refname`

#### 第一批延伸：同等优先级的下一批候选（仍优先 git2）

> 以下项同样高频 + 只读，且更偏向 git2 直接对应。部分已迁移，部分仍走 CLI。

4. ✅ `rebase_authorship.rs` 中 `walk_commits_to_base()` 的 `merge-base --is-ancestor` 和 `rev-list`
5. ⛔ `rebase_authorship.rs` 中 `is_ancestor()` / `rev_list_ancestry_path()` / `get_tracked_paths()` — 已删除，逻辑合并进 `walk_commits_to_base()`
6. ✅ `search.rs` 中 `search_by_commit_range()` 的 `rev-list`
7. ✅ `blame.rs` 中 `resolve_blame_abbrev_shas_batched()` 的 `rev-parse --short`
8. `refs.rs` 中 ~~`ref_exists()`~~ (✅ 已迁移) / `rev_parse()` / ~~`copy_ref()`~~ (✅ 已迁移) / ~~`get_commit_authorships()`~~ (⛔ 已删除)
9. ✅ `sync_authorship.rs` 中 `get_current_branch()`
10. `daemon.rs` / `daemon/git_backend.rs` 中 `merge-base --is-ancestor` / `symbolic-ref` / `rev-parse HEAD`
11. `hooks/` 各文件中 `is_ancestor()` / `merge-base --is-ancestor`（rebase/cherry_pick/update_ref/reset_hooks.rs）
12. `hooks/rebase_hooks.rs` 中 `walk_first_parent_commits()` 的 `rev-list --first-parent`
13. `hooks/cherry_pick_hooks.rs` 中 `expand_commit_range()` / `resolve_commit_sha()`
14. `hooks/stash_hooks.rs` 中 `resolve_stash_to_sha()` 的 `rev-parse`
15. `continue_session.rs` 中 `CommitInfo::from_commit_sha()` / `get_git_status_info()`
16. `prompts_db.rs` 中 `reachable_commits()` / `commit_dates_for()`
17. `diff.rs` 中 `resolve_commit()` / `get_commit_metadata()`
18. `bash_tool.rs` 中 `get_git_dir()` 的 `rev-parse --git-dir`
19. `client.rs` 中 `resolve_git_identity()` 的 `var`
20. `repository.rs` 中 `resolve_git_var_identity()` / `git_version()` / `commit_range_on_branch()`

### 第二批：按后端适配度分流推进

> 这一批不再默认等同于“继续迁 git2”，而是根据表中 `git2 替换` / `gix 替换` 两列分别进入 git2 或 gix 路线。部分项目已迁移，见标注。

21. ✅ `repository.rs` 中 `find_reference/references/find_commit/find_blob/find_tree/head/object_type/Blob::content`（已迁移）
22. ✅ `repository.rs` 中 `Tree::get_path`（已迁移）；✅ `get_file_content`（已迁移）
23. `authorship_traversal.rs` 中 `batch_read_blobs_with_oids()` 改用 git2 blob 读取
24. `refs.rs` 中 `batch_read_blobs_with_oids()` 同上
25. `status.rs` — 优先走 `gix` 路线，而非继续往 `git2` 补齐
26. `diff_tree_to_tree.rs` 中 `rev-parse --empty-tree` 改为硬编码常量
27. `rebase_authorship.rs` 中 `batch_read_blobs_with_oids()` / `batch_read_blobs_with_oids_concurrent()` 改用 git2 blob 读取
28. `rebase_authorship.rs` 中 `get_commit_metadata_batch()` 改用 git2 批量 commit 读取
29. `prompts_db.rs` 中 `batch_read_blobs()` 改用 git2 blob 读取
30. `stash_hooks.rs` 中 `read_stash_note()` / `save_stash_authorship_log()`

### 最后再碰（高成本候选，按 git2 / gix 分流评估）

31. `repository.rs` 中 `commit-tree/update-ref/reference/commit`
32. `refs.rs` 中 notes 写入相关 (`fast-import`, `notes merge`)
33. `blame.rs` 中 `blame --line-porcelain`
34. `diff_tree_to_tree.rs` 中 `diff --raw` 的 parser 重写
35. `sync_authorship.rs` 中 `fetch/push` (网络操作)
36. `rebase_authorship.rs` 中 `diff_tree_combined()` / `tracked_file_blob_changed_in_range()` / `collect_tracked_paths_in_range()`（diff-tree 操作，输出格式强依赖）
37. `range_authorship.rs` 中 `fetch` 和 `diff --numstat`
38. `bash_tool.rs` 中 `git_status_fallback()` 的 `status --porcelain=v2`（可优先考虑 `gix`）
39. `fetch_hooks.rs` 中 `reflog` 读取

### 保留 CLI（长期不建议迁移到 git2 或 gix）

40. `log.rs` — `git log` 代理（需完整 CLI 体验）
41. `git_handlers.rs` — git 代理核心（架构根基）
42. `repository.rs` 中 `diff_*` 系列函数（patch 语义强依赖）
43. `refs.rs` 中 `grep_ai_notes()`（git2 无 grep）
44. `sync_authorship.rs` 中 `fast-import` 写入（git2 无等价）
45. `diff.rs` 中 `get_diff_text()` / `get_commit_diff()`（diff 展示，强依赖 CLI 格式）
46. `ci/` — CI clone/fetch（网络 IO 主导，冷路径）

---

## 迁移状态分组汇总

> 以下汇总基于前文各表 `当前状态` 列以及 `git2 替换` / `gix 替换` 两列提炼，帮助快速定位已完成项、候选项和保留 CLI 项。

### 已迁移

当前源码已通过 git2 / gix 参与的进程内 API 实现，不再走 CLI。

**repository.rs — 对象解析与 peel**

- `revparse_single()`
- `Object::peel_to_commit()`
- `Commit::tree()` / `tree_id()`
- `Commit::parent(n)` / `Commit::parents()` / `parent_ids()`

**repository.rs — commit 元数据**

- `Commit::summary()` / `body()`
- `Commit::author()` / `committer()`

**repository.rs — 拓扑与范围查询**

- `Repository::merge_base()`
- `CommitRange::length()` / `into_iter()` / `is_valid()`
- `parent_on_refname()`

**repository.rs — ref 查询**

- `Reference::shorthand()` / `target()`
- `Reference::peel_to_blob()` / `peel_to_commit()`
- `head()`
- `find_reference()` / `references()`

**repository.rs — 对象查找与读取**

- `object_type()`
- `Blob::content()`
- `find_commit()` / `find_blob()` / `find_tree()`
- `Tree::get_path()`

**repository.rs — 仓库发现**

- `is_bare_repository()`
- `find_repository()`

**status.rs — staged/unstaged 文件枚举**

- `get_staged_filenames()` — 使用 `gix_index` + `git2` HEAD tree diff
- `get_staged_and_unstaged_filenames()` — 使用 `git2::StatusOptions` + `repo.statuses()`

**repository.rs — ref 推断与远程查询**

- `new_infer_refname()` — 使用 `open_git2()` + `g2repo.references()`
- `remote_head()` — 使用 `open_git2()` + `find_reference()` + `symbolic_target()`
- `upstream_remote()` — 使用 `head()` + `shorthand()` + `config_get_str()`（全部 git2/gix）
- `remotes()` — 使用 `open_git2()` + `g2repo.remotes()`
- `remotes_with_urls()` — 使用 `open_git2()` + `g2repo.remotes()` + `find_remote()` + `url()`

**repository.rs — 文件内容读取**

- `get_file_content()` — 使用 `open_git2()` + `revparse_single` → `peel_to_commit` → `tree` → `get_path` → `find_blob` + `Blob::content`
- `get_all_staged_files_content()` — 使用 `gix_index::File::at()` + `git2::find_blob()` + `Blob::content()`

**rebase_authorship.rs — commit 遍历**

- `walk_commits_to_base()` — 使用 `git2::Repository` + `revparse_single` + `graph_descendant_of` + `revwalk` + `Sort::TOPOLOGICAL`（含原 `is_ancestor` / `rev_list_ancestry_path` 逻辑）

**其他文件**

- `blame.rs`: `resolve_blame_abbrev_shas_batched()` — 使用 `git2::Oid::from_str` + `odb.exists_prefix`（CLI 仅 fallback）
- `search.rs`: `search_by_commit_range()` — 使用 `git2::Repository` + `revwalk` + `Sort::TIME`
- `refs.rs`: `ref_exists()` — 使用 `git2::Repository` + `find_reference()`
- `refs.rs`: `copy_ref()` — 使用 `git2::Repository` + `revparse_single` + `reference()`
- `sync_authorship.rs`: `get_current_branch()` — 使用 `git2::Repository` + `head()` + `shorthand()`

---

### 未迁移（下一步候选）

仍在走 CLI，属于可继续迁移的项。这里按“优先 git2”“优先 gix”“高成本候选”三种视角组织，而不是把所有候选混成单一 backlog。

**高优先级（优先 git2）** — 只读、高频、git2 直接对应

- `refs.rs`: `rev_parse()`
- `daemon.rs`: `is_ancestor_commit()` / rebase subject matching (`log --format=%s`)
- `daemon/git_backend.rs`: `repo_context()` / `rev_parse_head()`
- `hooks/rebase_hooks.rs`: `walk_first_parent_commits()` / `is_ancestor()`
- `hooks/cherry_pick_hooks.rs`: `expand_commit_range()` / `resolve_commit_sha()`
- `hooks/update_ref_hooks.rs`: `is_ancestor()`
- `hooks/reset_hooks.rs`: `is_ancestor()`
- `hooks/stash_hooks.rs`: `resolve_stash_to_sha()`
- `continue_session.rs`: `CommitInfo::from_commit_sha()` / `get_git_status_info()`
- `prompts_db.rs`: `reachable_commits()` / `commit_dates_for()`
- `diff.rs`: `resolve_commit()` / `get_commit_metadata()`
- `bash_tool.rs`: `get_git_dir()`
- `client.rs`: `resolve_git_identity()`
- `repository.rs`: `resolve_git_var_identity()` / `git_version()` / `commit_range_on_branch()`

**中优先级（git2 / gix 混合候选）** — 部分对应或需额外组装

- `authorship_traversal.rs`: `batch_read_blobs_with_oids()`
- `rebase_authorship.rs`: `batch_read_blobs_with_oids()` / `batch_read_blobs_with_oids_concurrent()` / `get_commit_metadata_batch()` / `get_committed_files_content_batch()`
- `refs.rs`: `batch_read_blobs_with_oids()`
- `refs.rs`: `note_blob_oids_for_commits()`
- `refs.rs`: `show_authorship_note()`
- `refs.rs`: `list_all_notes()`
- `prompts_db.rs`: `get_notes_list()` / `batch_read_blobs()`
- `diff_tree_to_tree.rs`: `diff_tree_to_tree()` 中 `rev-parse --empty-tree` 改为硬编码常量
- `status.rs`: ✅ `get_staged_filenames()` / `get_staged_and_unstaged_filenames()`（已迁移）；`status()` 仍走 CLI（优先考虑 `gix` 路线）；以及 `repo_state.rs` 的 branch metadata contract
- `stash_hooks.rs`: `read_stash_note()` / `has_stash_conflict()`
- `ensure_git_symlinks.rs`: `ensure_git_symlinks()`

**低优先级（高成本候选）** — 写操作、网络 IO、复杂语义、冷路径

- `repository.rs`: `resolve_author_spec()`
- `repository.rs`: `blob()` (写对象)
- `repository.rs`: `reference()` (创建/更新 ref)
- `repository.rs`: `commit()` (创建 commit)
- `repository.rs`: `fetch_branch()`
- `rebase_authorship.rs`: `diff_tree_combined()` / `tracked_file_blob_changed_in_range()` / `collect_tracked_paths_in_range()`（diff-tree stdin 批量操作）
- `blame.rs`: `blame_hunks_for_ranges()` 的 `--line-porcelain`（需逐一校验参数兼容）
- `diff_tree_to_tree.rs`: `diff_tree_to_tree()` 的 `diff --raw`（需重写 parser）
- `sync_authorship.rs`: `fetch_authorship_notes()` / `push_authorship_notes()`（网络 IO 主导）
- `sync_authorship.rs`: `fetch_missing_notes_for_commits()` / `get_local_notes_map()`
- `range_authorship.rs`: `get_range_diff_stats()` / fetch 操作
- `stash_hooks.rs`: `save_stash_authorship_log()`（写 notes）
- `fetch_hooks.rs`: `reflog` 读取
- `bash_tool.rs`: `get_changed_files()`（可优先考虑 `gix`）
- `status.rs` (commands): `diff --numstat` 统计
- `install_hooks.rs`: `set_global_git_config_value()`（仅在安装时调用，冷路径）

---

### 不建议迁移（保留 CLI）

以下项因 git2 / gix 都缺少合适的高层等价能力、语义强依赖 CLI 输出格式、或属于架构核心代理，建议长期保留 CLI 实现。

**repository.rs — diff 系列**

- `list_commit_files()` — `diff-tree --name-only -r -z`，patch 语义强依赖
- `diff_added_lines()` — `diff -U0 --find-renames=1%`
- `diff_added_lines_with_deleted_count()` — `diff -U0`
- `diff_changed_files()` — `diff --name-only -z`
- `diff_workdir_added_lines()` — `diff -U0`
- `diff_workdir_added_lines_with_insertions()` — `diff -U0 --no-renames`
- `merge_trees_favor_ours()` — `merge-tree --write-tree -X ours`

**refs.rs — notes 写入与 grep**

- `notes_add_batch()` — `fast-import`，git2 无等价
- `notes_add_blob_batch()` — `fast-import`，同上
- `merge_notes_from_ref()` — `notes merge`，git2 无 notes merge
- `fallback_merge_notes()` — `fast-import`，同上
- `grep_ai_notes()` — `grep -nI`，git2 无 grep notes

**架构核心代理**

- `log.rs`: `handle_log()` — 透传 `git log --notes=ai`，需 pager / 颜色 / 全部参数
- `git_handlers.rs`: `handle_git()` / `run_git_with_hooks()` — git 代理核心，注入 hooks 路径后转发真实 git

**diff 展示**

- `diff.rs`: `get_diff_text()` — `diff -U0 --find-renames=1%`，patch 语义强依赖
- `diff.rs`: `get_commit_diff()` — `show --format= --stat --patch`，完整的 diff 展示格式

**CI 模块**

- `ci/github.rs`: clone / fetch PR（网络 IO 主导，CI-only 冷路径）
- `ci/gitlab.rs`: clone / fetch MR / remote set-url（同上）

---

## 三十一、哪些条目更适合 gix 而非 git2

> 本节是前文各张表里 `gix 替换` / `gix 迁移难度` 两列的判断依据汇总：只标注“`git2` 能做，但抽象层不贴近需求；而 `gix` 更贴近 Git plumbing / 现有代码结构”的条目。它不是“凡是能用 gix 都该迁”的清单。

### 明确更适合 gix

| 文件 | 函数 | CLI 命令 | 为什么 `git2` 不顺手 | 为什么 `gix` 更合适 | 建议 |
|---|---|---|---|---|---|
| `status.rs` | `get_staged_filenames()` | ~~`diff --cached --name-only -z --no-renames`~~ | `git2` 能做 index / worktree diff，但要自己拼出接近 CLI 的 staged 视图 | 这是典型的 index / worktree plumbing，`gix-status` / `gix-index` 更贴近底层模型 | ✅ **已通过 `gix_index` + `git2` 迁移** |
| `status.rs` | `get_staged_and_unstaged_filenames()` | ~~`status --porcelain=v2 -z`~~ | 对照表前文已注明：迁到 `git2` status API 后，需要自行组装等价状态结构 | `gix` 本身就更适合处理 index、worktree、dirwalk 这类状态枚举；而且仓库已在 index 层使用 `gix_index` | ✅ **已通过 `git2::StatusOptions` 迁移** |
| `status.rs` | `status()` | `status --porcelain=v2 -z` [+ pathspecs] | 需保留 pathspec 过滤、NFC 归一化、rename/copy/unmerged 解析 | `gix` 更贴近底层状态枚举模型 | **优先考虑迁到 `gix`** |
| `repo_state.rs` | `read_head_state_for_worktree()` | `status --porcelain=v2 -z --branch`（语义等价目标） | 难点不在”能不能查状态”，而在”如何稳定复刻 branch / detached-HEAD 元数据语义” | `gix` / 现有 head-state 路径更适合承接这类元数据读取，再由本项目自己生成需要的结构 | **优先考虑沿现有 head-state 路径演进** |
| `repository.rs` | `get_all_staged_files_content()` | ~~并发 `git show :<path>`~~ | 用 `git2` 可以读 index 和 blob，但实现会和现有 index plumbing 分叉 | 当前仓库已经在 `src/git/repository.rs` 中用 `gix_index::entry::Stage` 和 `gix_index::File::at(...)` 直接读取 index，继续沿着 `gix` 取 staged blob 更自然 | ✅ **已通过 `gix_index` + `git2` 迁移** |
| `bash_tool.rs` | `git_status_fallback()` (原 `get_changed_files`) | `status --porcelain=v2 -z --untracked-files=all` | 同 `status.rs`，需要自行组装等价状态结构 | 同 `status.rs`，仓库已在 index 层使用 `gix_index`，继续沿 `gix` 方向更自然 | **优先考虑迁到 `gix`** |

### 可考虑 gix，但不属于“低成本替换”

| 文件 | 函数 | CLI 命令 | 为什么 `git2` 不理想 | 为什么 `gix` 只算“可考虑” |
|---|---|---|---|---|
| `diff_tree_to_tree.rs` | `diff_tree_to_tree()` | `diff --raw -z --no-abbrev <old> <new>` | `git2` Diff API 更偏高层对象模型，不直接对应当前依赖的 `--raw` 输出 | `gix-diff` 更贴近 tree diff / rewrite tracking 这类 plumbing，但仍需要重写 parser，把 `gix` 的变更结构映射回当前自定义 raw 格式 |

### 不要误判为“换 gix 就更容易”的条目

以下条目不是 `git2` 的舒适区，但也**不能**因此直接归为“更适合 gix”：

- `refs.rs` / `sync_authorship.rs` 中的 notes 读取与枚举：`gix` 可以从 ref / tree / blob 低层遍历实现一部分读取逻辑，但没有成熟的高层 notes API，因此不能简单归类为“比 `git2` 更容易”。
- `refs.rs` 中的 notes 写入、`fast-import`、`notes merge`：这类条目依然应保留 CLI，`gix` 没有高层等价能力。
- `blame.rs` 中的 `blame --line-porcelain`：无论是 `git2` 还是 `gix`，真正麻烦的都是复刻 porcelain 输出与参数语义，而不是底层是否能做 blame。

### 结论

如果目标是从“最难受的 `git2` 迁移点”里挑出更适合 `gix` 的条目，**优先级最高的是 `status.rs` 这组 status/index 命令，其次是 `repository.rs` 中直接依赖 staged index 内容的读取逻辑**。`diff_tree_to_tree.rs` 可以考虑 `gix`，但应把它视为“重写 parser 的专项工作”，而不是低成本替换。

### 可直接开 issue 的迁移优先级 checklist

> 下面这份 checklist 刻意压缩成“可以直接拆 issue”的粒度。每一项都只覆盖一个小范围目标，避免把 status、index、diff、notes 混在同一张工单里。

- [x] **P0：用 `gix` 重做 staged 文件枚举** ✅ 已完成
  - **目标文件**：`src/git/status.rs`
  - **目标函数**：`get_staged_filenames()`
  - **完成标准**：保留当前 staged 文件列表语义；不改变 pathspec / post-filter 行为；不引入对 `--porcelain=v2` 的新依赖。

- [x] **P0：用 `gix` 重做 staged + unstaged 状态枚举** ✅ 已完成
  - **目标文件**：`src/git/status.rs`
  - **目标函数**：`get_staged_and_unstaged_filenames()`
  - **完成标准**：继续产出与当前调用方兼容的状态结构；明确记录哪些字段来自 `gix` 枚举、哪些字段仍由本项目自己组装。

- [ ] **P0：迁移 `status()` 带完整 porcelain v2 解析的方法**
  - **建议 issue 标题**：`migrate status() porcelain-v2 pathspec-filtered query to gix/git2`
  - **目标文件**：`src/git/status.rs`
  - **目标函数**：`status()`
  - **完成标准**：保留 pathspec 过滤、NFC 归一化、rename/copy/unmerged/untracked 完整解析；输出 `Vec<StatusEntry>` 与当前调用方兼容。

- [ ] **P0：沿现有 `gix_index` 路径重做 staged blob 内容读取**
  - **建议 issue 标题**：`read staged blob contents via gix_index instead of per-path git show`
  - **目标文件**：`src/git/repository.rs`
  - **目标函数**：`get_all_staged_files_content()`
  - **完成标准**：复用现有 `gix_index` 读取 index 的路径；仅读取 stage-0 / unconflicted 条目；输出仍与当前调用方兼容。

- [ ] **P1：评估并设计 `gix-diff` 版本的 tree diff raw 输出映射**
  - **建议 issue 标题**：`prototype gix-diff backend for diff_tree_to_tree raw output`
  - **目标文件**：`src/git/diff_tree_to_tree.rs`
  - **目标函数**：`diff_tree_to_tree()`
  - **完成标准**：先证明 `gix-diff` 能提供当前 parser 所需的 delta 信息；如果不能一比一映射，就明确记录缺口，不强行替换 CLI。

- [ ] **P2：不要提前立 issue 的非目标项**
  - **暂不立项**：`refs.rs` 中 notes 写入 / `fast-import` / `notes merge`
  - **暂不立项**：`blame.rs` 中 `blame --line-porcelain`
  - **原因**：这些条目的难点主要是高层语义和 CLI 输出兼容，不是简单把底层库从 `git2` 换成 `gix` 就能解决。

## 额外提醒

### 不要把"能用 git2 或 gix"误解成"就该迁"

最有价值的目标不是"全面去 CLI"，而是：

- 去掉**高频、轻量、只读**的 git 子进程
- 保留**复杂语义 / 输出格式依赖 / 网络操作**的 CLI

### `git2` / `gix` 都只是手段，不是目标

仓库已经在 index 层用了 `gix_index`。如果核心痛点是 staged files、index scan、status 相关，继续往 `gix` 方向走可能比统一迁成 `git2` 更快。

### notes 系统是迁移难点

`refs.rs` 和 `sync_authorship.rs` 中大量使用 `git notes`、`git fast-import`、`git grep`。git2 对 notes 的支持非常有限（能读 notes tree，但没有 `notes add/merge/list` 等高层 API）。notes 写入和合并操作建议长期保留 CLI。
