# 生产代码 git2 迁移对照表

本文档覆盖整个 `src/` 生产代码中的 Git CLI 调用，标注每个调用能否用 `git2` (libgit2 Rust 绑定) 直接替代、预估性能收益、迁移难度。

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
| **难度** | 迁移到 git2 的工程复杂度 |
| **当前状态** | `✅ 已迁移` = 当前源码已使用 git2 进程内实现，不再走 CLI；留空 = 尚未迁移，仍在调 git CLI |

> **当前状态列说明**：标记为 `✅ 已迁移` 的行表示该函数在当前代码库中已通过 `git2`/libgit2 进程内 API 实现，不再派生 `git` 子进程。该列基于 `src/git/repository.rs` 中的实际实现确认，会随代码演进过时，请以源码为准。

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

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
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

### P1：建议迁移（收益中高）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `Reference::shorthand()` | `rev-parse --abbrev-ref <ref>` | 是 | `Reference::shorthand()` | ref 的短名 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::target()` | `rev-parse <ref>` | 是 | `Reference::target()` | ref 指向的 OID | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::peel_to_blob()` | `rev-parse --verify <ref>^{blob}` | 是 | `find_reference` + peel | ref peel 到 blob | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Reference::peel_to_commit()` | `rev-parse --verify <ref>^{commit}` | 是 | `find_reference` + peel | ref peel 到 commit | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `head()` | `symbolic-ref HEAD` | 是 | `Repository::head()` | 获取 HEAD 指向的 ref 名 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_reference()` | `show-ref --verify -s` | 是 | `Repository::find_reference()` | 查找指定 ref | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `references()` | `for-each-ref --format=%(refname)` | 是 | `Repository::references()` | 枚举所有 ref | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `new_infer_refname()` | `for-each-ref --points-at` | 部分 | `Repository::references()` + target 过滤 | 推断 refname | >500ms → <1ms | 中 | |
| `repository.rs` | `remote_head()` | `symbolic-ref refs/remotes/.../HEAD` | 部分 | `find_reference()` + symbolic target | 远程 HEAD | >500ms → <1ms | 中 | |
| `repository.rs` | `upstream_remote()` | `branch --show-current` + config | 部分 | `head()` + `branch_upstream_remote()` | 上游 remote 名 | >500ms → <1ms | 中 | |
| `repository.rs` | `object_type()` | `cat-file -t <oid>` | 是 | `find_object()` + `ObjectType` | 查对象类型 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `Blob::content()` | `cat-file blob <oid>` | 是 | `find_blob()` + `Blob::content()` | 读 blob 内容 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_commit()` | `cat-file -t` 后校验 | 是 | `Repository::find_commit()` | 查找 commit 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_blob()` | `cat-file -t` 后校验 | 是 | `Repository::find_blob()` | 查找 blob 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_tree()` | `cat-file -t` 后校验 | 是 | `Repository::find_tree()` | 查找 tree 对象 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `get_file_content()` | `show <commit>:<path>` | 是 | `find_commit` → `tree` → `get_path` → blob | 读指定 commit 的文件 | >500ms → <1ms | 中 | |
| `repository.rs` | `Tree::get_path()` | `ls-tree -z -r <tree> -- <path>` | 是 | `Tree::get_path()` | 从 tree 中找路径 | >500ms → <1ms | 中 | ✅ 已迁移 |
| `repository.rs` | `get_all_staged_files_content()` | 并发 `git show :<path>` | 部分 | `Repository::index()` → blob OID → blob content | 批量读 staged 文件 | N×>500ms → <1ms | 中 | |

### P2：可迁移但收益一般

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `remotes()` | `remote` | 是 | `Repository::remotes()` | 列 remote 名 | >500ms → <1ms | 低 | |
| `repository.rs` | `remotes_with_urls()` | `remote -v` | 部分 | `remotes()` + `find_remote()` + URL | 列 remote 及 URL | >500ms → <1ms | 中 | |
| `repository.rs` | `resolve_author_spec()` | `rev-list --all --author=` + `show` | 部分 | `revwalk()` + 手动过滤 author | 按 author 名查找 commit | >500ms → <1ms | 中高 | |
| `repository.rs` | `is_bare_repository()` | `rev-parse --is-bare-repository` | 是 | `Repository::is_bare()` | 判断是否 bare 仓库 | >500ms → <1ms | 低 | ✅ 已迁移 |
| `repository.rs` | `find_repository()` | `rev-parse --git-dir --git-common-dir --show-toplevel` | 部分 | `Repository::discover()` / `open_ext()` | 发现/打开仓库 | >500ms → <1ms | 中 | ✅ 已迁移 |

### P3：能做但不建议第一批

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `repository.rs` | `blob()` | `hash-object -w --stdin` | 是 | `Repository::blob()` | 写入 blob 对象 | >500ms → <1ms | 中 | |
| `repository.rs` | `reference()` | `update-ref --stdin --create-reflog` | 部分 | `Repository::reference()` | 创建/更新 ref | >500ms → <1ms | 中高 | |
| `repository.rs` | `commit()` | `commit-tree` + `update-ref` | 部分 | `Repository::commit()` + refs 更新 | 创建 commit | >500ms → <1ms | 高 | |
| `repository.rs` | `fetch_branch()` | `fetch remote branch` | 部分 | `Remote::fetch()` | 拉取远程分支 | 收益不确定 | 高 | |

### 建议保留 CLI

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
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

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `refs.rs` | `batch_read_blobs_with_oids()` | `cat-file --batch` (stdin 批量) | 是 | `Repository::find_blob()` 批量 | 批量读 blob 内容 | 1×>500ms → N×<1ms | 中 | |
| `refs.rs` | `note_blob_oids_for_commits()` | `cat-file --batch-check` (stdin 批量) | 部分 | git2 notes API 有限，需手动查 tree | 批量查 commit 的 note blob OID | 1×>500ms → N×<1ms | 高 | |
| `refs.rs` | `notes_add_batch()` | `rev-parse --verify refs/notes/ai` + `fast-import` | 否 | git2 无 fast-import 等价 | 批量添加 notes | 不适用 | — | |
| `refs.rs` | `notes_add_blob_batch()` | `rev-parse --verify refs/notes/ai` + `fast-import` | 否 | 同上 | 批量关联已有 blob 为 notes | 不适用 | — | |
| `refs.rs` | `show_authorship_note()` | `notes --ref=ai show <sha>` | 部分 | 需查 notes tree 再读 blob | 读取单个 commit 的 note | >500ms → <1ms | 中 | |
| `refs.rs` | `ref_exists()` | `show-ref --verify --quiet <ref>` | 是 | `Repository::find_reference()` | 检查 ref 是否存在 | >500ms → <1ms | 低 | |
| `refs.rs` | `merge_notes_from_ref()` | `notes --ref=ai merge -s ours` | 否 | git2 无 notes merge | 合并 notes ref | 不适用 | — | |
| `refs.rs` | `fallback_merge_notes()` | `fast-import --quiet --done` | 否 | git2 无 fast-import | fallback 合并 notes | 不适用 | — | |
| `refs.rs` | `list_all_notes()` | `notes --ref=ai list` | 部分 | 需遍历 notes tree | 列出所有 notes | >500ms → <1ms | 高 | |
| `refs.rs` | `rev_parse()` | `rev-parse <rev>` | 是 | `Repository::revparse_single()` | 解析 ref 到 SHA | >500ms → <1ms | 低 | |
| `refs.rs` | `copy_ref()` | `update-ref <dest> <source>` | 是 | `Repository::reference()` | 复制 ref | >500ms → <1ms | 低 | |
| `refs.rs` | `grep_ai_notes()` | `grep -nI <pattern> refs/notes/ai` | 否 | git2 无 grep notes | 搜索 notes 内容 | 不适用 | — | |

---

## 三、`src/git/authorship_traversal.rs` — Authorship note 批量读取

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `authorship_traversal.rs` | `batch_read_blobs_with_oids()` | `cat-file --batch` (stdin) | 是 | `Repository::find_blob()` 批量 | 批量读 note blob 内容 | 1×>500ms → N×<1ms | 中 | |

> `authorship_traversal.rs` 里的 `get_notes_list()` 仅在 `#[cfg(test)]` 中使用，不属于生产代码。

---

## 四、`src/git/sync_authorship.rs` — Notes 同步（fetch/push）

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `sync_authorship.rs` | `fetch_missing_notes_for_commits()` | `notes --ref=refs/notes/ai list` | 部分 | 同 refs.rs notes 枚举 | 检查哪些 commit 缺少 note | >500ms → <1ms | 高 | |
| `sync_authorship.rs` | `fetch_authorship_notes()` | `fetch --no-tags ... <remote> <refspec>` | 部分 | `Remote::fetch()` | 从远端拉取 notes | 网络 IO 主导，git2 未必更快 | 高 | |
| `sync_authorship.rs` | `push_authorship_notes()` | `push --quiet ... <remote> <refspec>` | 部分 | `Remote::push()` | 推送 notes 到远端 | 网络 IO 主导 | 高 | |
| `sync_authorship.rs` | `get_local_notes_map()` | `notes --ref=ai list` | 部分 | 遍历 notes tree | 枚举本地所有 notes | >500ms → <1ms | 高 | |
| `sync_authorship.rs` | `get_current_branch()` | `rev-parse --abbrev-ref HEAD` | 是 | `Repository::head()` + `shorthand()` | 获取当前分支名 | >500ms → <1ms | 低 | |

---

## 五、`src/git/status.rs` — 工作目录状态

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `status.rs` | `get_staged_filenames()` | `diff --cached --name-only -z --no-renames` | 部分 | `Repository::diff_index_to_workdir()` 或 index 遍历 | 获取已 staged 的文件列表 | >500ms → <1ms | 中 | |
| `status.rs` | `get_staged_and_unstaged_filenames()` | `status --porcelain=v2 -z` | 部分 | git2 status API | 获取所有变更文件 | >500ms → <1ms | 中高 | |
| `status.rs` | `get_status_with_branch()` | `status --porcelain=v2 -z --branch` | 部分 | git2 status API + branch | 同上 + 分支信息 | >500ms → <1ms | 中高 | |

> status 模块强依赖 `--porcelain=v2` 的输出格式。迁移到 git2 status API 后，需要自行组装等价的状态结构。该模块已在 index 层使用了 `gix_index`，可优先考虑沿用/扩展 `gix` 而非迁到 `git2`。

---

## 六、`src/git/diff_tree_to_tree.rs` — Tree diff

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `diff_tree_to_tree.rs` | `diff_tree_to_tree()` | `rev-parse --empty-tree` | 是 | `git2` 内置 empty tree hash | 获取空 tree OID | >500ms → 硬编码常量 | 低 | |
| `diff_tree_to_tree.rs` | `diff_tree_to_tree()` | `diff --raw -z --no-abbrev <old> <new>` | 部分 | git2 Diff API + tree walk | 对比两个 tree | >500ms → <1ms | 高 | |

> `diff --raw` 的输出格式被自定义 parser 依赖。迁移需要重写 parser 以适配 git2 diff delta 结构。

---

## 七、`src/authorship/rebase_authorship.rs` — Rebase 后 authorship 重写

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `rebase_authorship.rs` | `get_commits_between()` | `merge-base --is-ancestor` | 是 | `graph_descendant_of()` | 校验 ancestor 关系 | >500ms → <1ms | 低 | |
| `rebase_authorship.rs` | `get_commits_between()` | `rev-list --topo-order --ancestry-path` | 是 | `Repository::revwalk()` + 过滤 | 枚举范围内的 commit | >500ms → <1ms | 中 | |

> `rebase_authorship.rs` 中的 `find_commit()` 调用经过 `repository.rs` 间接调用 CLI，迁移 `repository.rs` 后自动收益。

---

## 八、`src/commands/blame.rs` — AI Blame

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `blame.rs` | `resolve_blame_abbrev_shas_batched()` | `rev-parse --short=N <sha>...` (批量) | 是 | `Oid::to_string()` 取前 N 位 | 批量缩写 SHA | >500ms → <1ms | 低 | |
| `blame.rs` | `blame_hunks_for_ranges()` | `blame --line-porcelain [-w] [-M] [-C...] [-L ...]` | 部分 | `Repository::blame()` + `BlameOptions` | 完整 blame 输出 | >500ms → <1ms | 高 | |

> blame 是整个项目中对 git CLI 依赖最重的单点调用之一。git2 有 `Repository::blame()` API，但 `--line-porcelain` 的完整输出格式、`-C`/`-M` 检测、`--ignore-rev`/`--ignore-revs-file`、`--since` 过滤等参数组合需要仔细逐一校验。`resolve_blame_abbrev_shas_batched()` 则是独立的小优化点，SHA 缩写可以纯字符串截断。

---

## 九、`src/commands/search.rs` — Prompt 搜索

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `search.rs` | `search_by_commit_range()` | `rev-list start..end` | 是 | `Repository::revwalk()` | 枚举范围内的 commit | >500ms → <1ms | 低 | |

> `search_by_file()` 内部走的是 blame 系统，其 CLI 调用已计入 blame.rs。

---

## 十、`src/commands/log.rs` — git ai log

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `log.rs` | `handle_log()` | `git log --notes=ai [args...]` | 否 | — | 透传 `git log` 给用户 | 不可替代（需要 pager/颜色/全部参数） | — | |

> `handle_log()` 是一个纯代理：把用户参数原样传给 `git log --notes=ai`。这不需要迁移。它依赖 pager、颜色输出、用户自定义 format 等全部 git log 特性，git2 无法替代。

---

## 十一、`src/commands/git_handlers.rs` — Git 代理主入口

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `git_handlers.rs` | `handle_git()` / `run_git_with_hooks()` | `git -c core.hooksPath=... <subcmd>` | 否 | — | git 代理，注入 hooks 路径后执行真实 git | 不可替代（整个项目的核心分发机制） | — | |

> git 代理层是 git-ai 的架构根基。它拦截 `git` 调用、注入 `core.hooksPath`、转发给真实 git、再执行 post-hook。这必须保留 CLI。

---

## 十二、`src/commands/install_hooks.rs` — 安装 hooks

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `install_hooks.rs` | `set_global_git_config_value()` | `git config --global <key> <val>` | 部分 | `git2::Config` | 设置全局 git 配置 | >500ms → <1ms | 低 | |

> 仅在安装/升级时调用，不是热路径，收益有限。

---

## 十三、`src/commands/continue_session.rs` — 会话继续

该文件中的 `Command::new` 调用是启动 AI agent 进程（非 git），不涉及 git2 迁移。

---

## 十四、`src/authorship/ignore.rs` — .gitignore 检查

| 文件 | 函数 | 执行的命令 | git2可直接实现 | git2 替换 | 用途 | Win10 VM 性能对比 | 难度 | 当前状态 |
|---|---|---|---|---|---|---|---|---|
| `ignore.rs` | (test-only) | `Command::new(git_cmd)` | — | — | 仅测试代码使用 | 不适用 | — | |

> `ignore.rs` 的生产代码路径不直接调用 git CLI，而是通过 git2 (已有 `test-support` feature) 进行 ignore 检查。CLI 调用仅在测试中。

---

## 推荐落地顺序

### ✅ 第一批：最值（高频 + 只读 + git2 完美对应）— 已迁移

> 以下项目已在当前代码库中通过 git2 进程内 API 实现，不再走 CLI。

1. ✅ `repository.rs` 中所有 `rev-parse --verify` / peel / ref resolve
2. ✅ `repository.rs` 中 `Commit::summary/body/author/committer/tree/parent/parents`
3. ✅ `repository.rs` 中 `merge_base` / `CommitRange::{length, into_iter, is_valid}` / `parent_on_refname`

#### 第一批延伸：同等优先级的下一批候选

> 以下项同样高频 + 只读 + git2 直接对应，但目前仍走 CLI，建议紧接第一批之后迁移。

4. `rebase_authorship.rs` 中 `get_commits_between()` 的 `merge-base --is-ancestor` 和 `rev-list`
5. `search.rs` 中 `search_by_commit_range()` 的 `rev-list`
6. `blame.rs` 中 `resolve_blame_abbrev_shas_batched()` 的 `rev-parse --short`
7. `refs.rs` 中 `ref_exists()` 和 `rev_parse()` 和 `copy_ref()`
8. `sync_authorship.rs` 中 `get_current_branch()`

### 第二批：看时间

> 部分项目已迁移，见标注。

9. ✅ `repository.rs` 中 `find_reference/references/find_commit/find_blob/find_tree/head/object_type/Blob::content`（已迁移）
10. ✅ `repository.rs` 中 `Tree::get_path`（已迁移）；`get_file_content` 待确认
11. `authorship_traversal.rs` 中 `batch_read_blobs_with_oids()` 改用 git2 blob 读取
12. `refs.rs` 中 `batch_read_blobs_with_oids()` 同上
13. `status.rs` — 可优先考虑 `gix` 而非 `git2`
14. `diff_tree_to_tree.rs` 中 `rev-parse --empty-tree` 改为硬编码常量

### 最后再碰

15. `repository.rs` 中 `commit-tree/update-ref/reference/commit`
16. `refs.rs` 中 notes 写入相关 (`fast-import`, `notes merge`)
17. `blame.rs` 中 `blame --line-porcelain`
18. `diff_tree_to_tree.rs` 中 `diff --raw` 的 parser 重写
19. `sync_authorship.rs` 中 `fetch/push` (网络操作)

### 永远不碰

20. `log.rs` — `git log` 代理（需完整 CLI 体验）
21. `git_handlers.rs` — git 代理核心（架构根基）
22. `repository.rs` 中 `diff_*` 系列函数（patch 语义强依赖）
23. `refs.rs` 中 `grep_ai_notes()`（git2 无 grep）
24. `sync_authorship.rs` 中 `fast-import` 写入（git2 无等价）

---

## 迁移状态分组汇总

> 以下汇总基于前文各表 `当前状态` 列和 `推荐落地顺序` 提炼，帮助快速定位已完成项、候选项和不迁移项。

### 已迁移

当前源码已通过 git2 / 进程内 API 实现，不再走 CLI。

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

---

### 未迁移（下一步候选）

仍在走 CLI，属于可继续迁移的项。按优先级分三档。

**高优先级** — 只读、高频、git2 直接对应

- `rebase_authorship.rs`: `get_commits_between()` 的 `merge-base --is-ancestor` + `rev-list`
- `search.rs`: `search_by_commit_range()` 的 `rev-list`
- `blame.rs`: `resolve_blame_abbrev_shas_batched()` 的 SHA 缩写
- `refs.rs`: `ref_exists()` / `rev_parse()` / `copy_ref()`
- `sync_authorship.rs`: `get_current_branch()`

**中优先级** — 部分对应或需额外组装

- `repository.rs`: `new_infer_refname()`
- `repository.rs`: `remote_head()`
- `repository.rs`: `upstream_remote()`
- `repository.rs`: `get_file_content()`
- `repository.rs`: `get_all_staged_files_content()`
- `authorship_traversal.rs`: `batch_read_blobs_with_oids()`
- `refs.rs`: `batch_read_blobs_with_oids()`
- `refs.rs`: `note_blob_oids_for_commits()`
- `refs.rs`: `show_authorship_note()`
- `refs.rs`: `list_all_notes()`
- `diff_tree_to_tree.rs`: `diff_tree_to_tree()` 中 `rev-parse --empty-tree` 改为硬编码常量
- `status.rs`: `get_staged_filenames()` / `get_staged_and_unstaged_filenames()` / `get_status_with_branch()`（可优先考虑 `gix` 而非 `git2`）

**低优先级** — 写操作、网络 IO、复杂语义、冷路径

- `repository.rs`: `remotes()` / `remotes_with_urls()`
- `repository.rs`: `resolve_author_spec()`
- `repository.rs`: `blob()` (写对象)
- `repository.rs`: `reference()` (创建/更新 ref)
- `repository.rs`: `commit()` (创建 commit)
- `repository.rs`: `fetch_branch()`
- `blame.rs`: `blame_hunks_for_ranges()` 的 `--line-porcelain`（需逐一校验参数兼容）
- `diff_tree_to_tree.rs`: `diff_tree_to_tree()` 的 `diff --raw`（需重写 parser）
- `sync_authorship.rs`: `fetch_authorship_notes()` / `push_authorship_notes()`（网络 IO 主导）
- `sync_authorship.rs`: `fetch_missing_notes_for_commits()` / `get_local_notes_map()`
- `install_hooks.rs`: `set_global_git_config_value()`（仅在安装时调用，冷路径）

---

### 不建议迁移（保留 CLI）

以下项因 git2 无等价 API、语义强依赖 CLI 输出格式、或属于架构核心代理，建议长期保留 CLI 实现。

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

---

## 额外提醒

### 不要把"能用 git2"误解成"就该迁"

最有价值的目标不是"全面去 CLI"，而是：

- 去掉**高频、轻量、只读**的 git 子进程
- 保留**复杂语义 / 输出格式依赖 / 网络操作**的 CLI

### `git2` 不是唯一答案

仓库已经在 index 层用了 `gix_index`。如果核心痛点是 staged files、index scan、status 相关，继续往 `gix` 方向走可能比统一迁成 `git2` 更快。

### notes 系统是迁移难点

`refs.rs` 和 `sync_authorship.rs` 中大量使用 `git notes`、`git fast-import`、`git grep`。git2 对 notes 的支持非常有限（能读 notes tree，但没有 `notes add/merge/list` 等高层 API）。notes 写入和合并操作建议长期保留 CLI。
