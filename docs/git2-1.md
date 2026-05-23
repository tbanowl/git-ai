---
我 detect **investigation / evaluation** intent — 你要判断 `src/git/repository.rs` 里哪些 Git CLI 调用值得优先换成 `git2`，目标是 **减少 VM 里进程启动开销**，不是直接改实现。下面是综合代码扫描 + 并行研究后的结论。
---

## 结论先说

### 最适合优先替换成 `git2` 的
这些基本都是 **本地只读、对象库/提交图查询**，`git2` 覆盖度高，最能减少 subprocess 开销：

1. **`rev-parse` / object peel / ref resolve**
   - 典型位置：
     - `Object::peel_to_commit` `579-585`
     - `Commit::tree` `934-940`
     - `Commit::parent` `947-954`
     - `Reference::shorthand` `1300-1306`
     - `Reference::target` `1309-1314`
     - `revparse_single` `2193-2203`
   - 适合原因：
     - 都是对象解析、ref 解析、peel，`git2` 原生支持。
     - 现在很多地方是“拿一个 SHA / ref → 起一个 git 进程 → 输出一个 SHA”，在 VM 里很亏。
   - 评级：**强烈建议迁移**

2. **commit 元数据读取**
   - 典型位置：
     - `parents()` 里的 `%P`：`963-976`
     - `summary()`：`994-1002`
     - `body()`：`1008-1016`
     - `author()`：`1021-1039`
     - `committer()`：`1045-1063`
   - 适合原因：
     - `git2` 直接拿 `Commit` 的 parents、author、committer、message、tree。
     - 这里现在是 **每个 commit 多次 `git show -s`**，如果上层在遍历大量 commits，会非常慢。
   - 评级：**强烈建议迁移**

3. **提交图查询：`rev-list` / `merge-base` / ancestor check**
   - 典型位置：
     - `CommitRange::length` `759-768`
     - `CommitRange::into_iter` `809-823`
     - `CommitRange::is_valid` `708-749`
     - `Repository::merge_base` `1938-1944`
     - `parent_on_refname()` `1130-1145`
   - 适合原因：
     - 这些都是 commit graph 操作，`git2` 有 `revwalk`、`merge_base`、graph descendant API。
     - 现在的热点是 **循环里 repeatedly 调 `merge-base`**。
   - 评级：**强烈建议迁移**

4. **ref 枚举 / 查找 / HEAD 解析**
   - 典型位置：
     - `new_infer_refname()` `650-667`
     - `head()` `1570-1589`
     - `find_reference()` `1925-1935`
     - `references()` `2287-2305`
     - `remote_head()` `1913-1920`
   - 适合原因：
     - `for-each-ref` / `show-ref` / `symbolic-ref` 都偏 plumbing，`git2` 很适合。
   - 评级：**建议迁移**

5. **tree/blob/object 只读访问**
   - 典型位置：
     - `Tree::get_path` `1199-1259`
     - `Blob::content` `1275-1281`
     - `object_type` `1558-1564`
     - `get_file_content` `2351-2360`
   - 适合原因：
     - `git2` 对 tree/blob/object 查找支持很好。
     - 当前 `ls-tree` / `cat-file` / `show commit:path` 都是纯读操作。
   - 评级：**建议迁移**
   - 小心点：
     - `ls-tree -z -r` 当前依赖 CLI 输出格式；迁移后要改成对象遍历，不是机械替换。

---

## 可替换，但不一定先动的

6. **`remote` / `remote -v` / fetch 相关读取**
   - 位置：
     - `remotes()` `1692-1699`
     - `remotes_with_urls()` `1702-1725`
   - 适合原因：
     - remote 枚举可用 `git2`。
   - 但：
     - 收益通常不如 commit graph / metadata 热点高。
   - 评级：**可迁移，优先级中等**

7. **`hash-object -w` / `update-ref` / `commit-tree`**
   - 位置：
     - `blob()` `1877-1883`
     - `reference()` `1895-1904`
     - `commit()` `2126-2186`
   - 适合原因：
     - `git2` 理论上能做对象写入、创建 commit、更新 refs。
   - 但：
     - 这些属于“语义更重”的写操作。
     - `commit()` 这里不仅是写 commit，还包含：
       - author/committer env 语义
       - first-parent == current tip 校验
       - `update-ref` 的 CAS 风格更新
     - 迁移成本明显高于只读查询。
   - 评级：**能迁，但不建议作为第一批**

---

## 不建议优先换成 `git2` 的

8. **`diff` / `diff-tree` / patch 输出解析**
   - 位置：
     - `list_commit_files()` `2438-2490`
     - `diff_added_lines()` `2504-2545`
     - `diff_added_lines_with_deleted_count()` `2551-2568`
     - `diff_changed_files()` `2572-2596`
     - `diff_workdir_added_lines()` `2604-2645`
     - `diff_workdir_added_lines_with_insertions()` `2654-2696`
   - 原因：
     - 这些地方不只是“要 diff 结果”，而是强依赖 CLI diff 语义：
       - `-U0`
       - `--find-renames=1%`
       - `-z`
       - pathspec 处理
       - 当前已有自定义 parser：`parse_diff_added_lines*`
     - 用 `git2` 可以做 diff，但你得重写一大块“从 CLI patch 文本推导行号”的逻辑。
   - 这类迁移 **不是 API 对应替换**，而是 **算法/数据流重写**。
   - 评级：**不建议作为性能优化第一波**

9. **`merge-tree --write-tree -X ours`**
   - 位置：`1949-1964`
   - 原因：
     - `git2` 有 merge API，但和 CLI `merge-tree` 的行为、选项、输出语义并非一一对应。
   - 评级：**不建议优先迁移**

10. **`fetch` 网络操作**
   - 位置：`2699-2705`
   - 原因：
     - 外部研究显示 libgit2/git2 在 clone/fetch/checkout 这类网络或重 IO 场景，常常 **不如 Git CLI**，尤其大仓库。
   - 评级：**建议保留 CLI**

---

## 真正的热点在哪里

如果目标是“VM 里 git 命令太慢”，最大收益不是平均迁移，而是打这几个热点：

### 热点 1：遍历 commit 后，对每个 commit 再多次起进程
- `CommitRange::into_iter()` 先 `rev-list` 一次拿 OIDs：`809-823`
- 但后面每个 `Commit` 再调用：
  - `summary()`
  - `body()`
  - `author()`
  - `committer()`
  - `tree()`
  - `parents()`
- 这会形成 **1 次 range 查询 + N 次甚至 4N~6N 次 git 子进程**

### 热点 2：循环里做 ancestor check
- `parent_on_refname()` 对每个 parent 调一次 `merge-base --is-ancestor`：`1130-1145`
- `is_valid()` 也会做多次 `merge-base --is-ancestor`：`708-749`

### 热点 3：大量轻量级 `rev-parse`
- 很多 API 只是为了把 ref/spec 变成 SHA/type/tree/blob，就单独起进程。
- 这在裸机上未必严重，在 VM 上很容易放大。

### 热点 4：并发 `git show` 读取 staged files
- `get_all_staged_files_content()` 最多并发 30 个外部 git 进程：`2373-2398`
- 在慢 VM 上，可能反而因为进程调度/IO 抖动变差。
- 这一块如果改成 index/blob 直接读，收益会很明显。

---

## 推荐的迁移顺序

### 第一批：最值
1. `rev-parse` / peel / ref resolve
2. commit 元数据（author/committer/message/tree/parents）
3. `rev-list` / `merge-base` / graph walk
4. tree/blob/object 只读查找

### 第二批：看时间
5. ref 枚举 / remote 枚举
6. `hash-object` / `update-ref` / `commit-tree`

### 最后再考虑
7. `diff*`
8. `merge-tree`
9. `fetch`

---

## 一句话建议

**如果你只想先做最划算的优化：**
优先把 `repository.rs` 里所有 **只读的 commit/ref/object graph 查询** 换成 `git2`；  
把 **diff / merge-tree / fetch** 暂时留在 CLI。

这是收益最高、风险最低的切法。

如果你愿意，我下一步可以直接给你一张更具体的表：

**函数名 | 当前 git 命令 | 推荐 git2 API | 迁移难度 | 预估收益**

只针对 `repository.rs`，方便你按优先级开工。