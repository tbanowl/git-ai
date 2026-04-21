I detect **planning** intent — 你要的是可直接开工的 **第一阶段改造清单**，只覆盖最值钱的迁移点。我的做法：按函数给出 `git2` 对应调用、必须保留的现有语义、以及最容易踩的坑。

---

# 第一阶段具体改造清单（P0 + 核心 P1）

## 1) `revparse_single()`
- 位置：`2193-2203`
- 当前：
  - `git rev-parse --verify <spec>`
- 改成：
  - `Repository::revparse_single(spec)`
- 要保留的语义：
  - 失败时仍返回“spec 无效/不存在”的错误
  - 返回的是最终 OID，而不是模糊 ref 文本
- 坑：
  - `git rev-parse` 支持的某些 revspec 语法比 `git2` 更宽
  - 先确认你项目里传入的 spec 主要是：
    - SHA
    - branch/ref
    - `HEAD`
    - `foo^1`
    - `foo^{commit}` / `foo^{tree}`
  - 如果有更怪的 revspec，可能要保留 fallback CLI

---

## 2) `Object::peel_to_commit()`
- 位置：`579-585`
- 当前：
  - `rev-parse --verify <oid>^{commit}`
- 改成：
  - `repo.revparse_single(&oid)?`
  - 然后 `peel_to_commit()`
- 要保留的语义：
  - 非 commit-ish 对象必须报错
  - 输出仍是 commit OID
- 坑：
  - 别只做 lookup，不 peel；tag 指向 commit 的情况会出错

---

## 3) `Commit::tree()`
- 位置：`934-940`
- 当前：
  - `rev-parse --verify <commit>^{tree}`
- 改成：
  - `find_commit(oid)?.tree()` 或 `tree_id()`
- 要保留的语义：
  - 返回 tree OID
- 坑：
  - 不需要再经过 revparse；直接从 `Commit` 拿 tree 即可

---

## 4) `Commit::parent()` / `Commit::parents()` / `parent_count()`
- 位置：
  - `parent()` `946-954`
  - `parents()` `963-976`
  - `parent_count()` `988-990`
- 当前：
  - `parent()`：`rev-parse <oid>^N`
  - `parents()`：`show -s --format=%P`
- 改成：
  - `Commit::parent(i)`
  - `Commit::parents()` / `parent_ids()`
  - `Commit::parent_count()`
- 要保留的语义：
  - `parent(i)` 仍是 0-based 对外接口
  - merge commit 的所有 parent 顺序要保持一致
- 坑：
  - 当前代码自己处理了 “Git syntax 1-based，libgit2 0-based” 的转换；迁移后别重复偏移一次

---

## 5) `Commit::summary()` / `body()` / `author()` / `committer()` / `time()`
- 位置：
  - `summary()` `994-1002`
  - `body()` `1008-1016`
  - `author()` `1021-1039`
  - `committer()` `1045-1063`
  - `time()` `1070-1072`
- 当前：
  - 多次 `git show -s --format=...`
- 改成：
  - `find_commit()` 一次
  - 然后直接读：
    - `summary` / `message`
    - `author`
    - `committer`
    - `time`
- 要保留的语义：
  - UTF-8/空字符串处理和当前一致
  - `Signature` 和现有 `Time` 结构转换行为保持一致
- 坑：
  - `body()` 现在等价于 `%b`，不是完整 message 原文
  - 如果你用 `message()`，要自己拆 summary/body，别把行为改了
- 优先建议：
  - 这里最好顺手做一个内部 helper：
    - “先取一次 `git2::Commit`，后续字段复用”
  - 不然虽然去掉了 CLI，但还会重复 `find_commit`

---

## 6) `Repository::merge_base()`
- 位置：`1938-1944`
- 当前：
  - `git merge-base A B`
- 改成：
  - `Repository::merge_base(oid1, oid2)`
- 要保留的语义：
  - 输出 merge-base OID
- 坑：
  - 输入现在是 `String`，迁移后你需要先解析成 `Oid`

---

## 7) `CommitRange::length()`
- 位置：`757-770`
- 当前：
  - `git rev-list --count A..B`
- 改成：
  - `revwalk` + count
- 要保留的语义：
  - 只计算 `B` 可达但 `A` 不可达的 commits
- 坑：
  - `revwalk` 默认排序/遍历方式和 CLI 不是“天然完全一样”
  - 对 count 通常问题不大，但要确保正确 hide `A`

---

## 8) `CommitRange::into_iter()`
- 位置：`807-829`
- 当前：
  - `git rev-list A..B`
- 改成：
  - `Repository::revwalk()`
  - push `end_oid`
  - hide `start_oid`
- 要保留的语义：
  - 空 range / 单 commit range 的当前特殊逻辑
  - 返回 commit OID 列表行为稳定
- 坑：
  - CLI `rev-list` 的输出顺序要核对
  - 你当前 iterator 结果顺序若被上层依赖，迁移时要显式设定 sort
- 建议：
  - 在这一步不要顺带“优化顺序语义”，只求对齐现有行为

---

## 9) `CommitRange::is_valid()`
- 位置：`694-752`
- 当前：
  - 多次 `merge-base --is-ancestor`
- 改成：
  - `graph_descendant_of()` / `merge_base()` 组合
- 要保留的语义：
  - `start_oid`、`end_oid` 必须存在
  - 都必须在 `refname` 上可达
  - `start` 必须是 `end` 祖先
  - 空树 hash 特判保留
- 坑：
  - `git merge-base --is-ancestor A B` 和某些库 API 在“self ancestor”语义上可能不同
  - 一定要专门测：
    - `A == B`
    - root commit
    - empty tree hash
    - detached HEAD / unusual refname

---

## 10) `Commit::parent_on_refname()`
- 位置：`1097-1152`
- 当前：
  - 先规范化 refname
  - 再遍历 parents
  - 对每个 parent 调一次 `merge-base --is-ancestor`
- 改成：
  - refname -> reference -> target commit
  - 遍历 parents
  - 对每个 parent 用 graph API 判断是否被该 ref tip 包含
- 要保留的语义：
  - “返回第一个在该 refname 上可达的 parent”
  - refname 规范化 fallback 行为保留
- 坑：
  - 当前 fallback：
    - `refs/...` 原样保留
    - 否则补 `refs/heads/...`
  - 这个逻辑不要丢，不然一些宽松输入会变行为

---

## 11) `head()`
- 位置：`1570-1589`
- 当前：
  - `git symbolic-ref HEAD`
  - 失败则返回 `\"HEAD\"`
- 改成：
  - `Repository::head()`
- 要保留的语义：
  - detached HEAD 时返回 `\"HEAD\"`，不是错误
- 坑：
  - 这不是普通 `find_reference(\"HEAD\")` 语义，detached 行为要对齐现有实现

---

## 12) `find_reference()` / `references()` / `Reference::{shorthand,target}`
- 位置：
  - `find_reference()` `1925-1935`
  - `references()` `2287-2305`
  - `shorthand()` `1300-1306`
  - `target()` `1309-1314`
- 当前：
  - `show-ref`
  - `for-each-ref`
  - `rev-parse`
- 改成：
  - `Repository::find_reference()`
  - `Repository::references()`
  - `Reference::shorthand()`
  - `Reference::target()`
- 要保留的语义：
  - 输出 refname 列表
  - 不存在 ref 时报错
- 坑：
  - `for-each-ref --format=%(refname)` 现在天然给你字符串
  - 用 `git2` 后你要自己把 iterator 转成当前结构

---

## 13) `object_type()` / `find_commit()` / `find_blob()` / `find_tree()`
- 位置：
  - `object_type()` `1558-1564`
  - `find_commit()` `2309-2321`
  - `find_blob()` `2325-2333`
  - `find_tree()` `2337-2345`
- 当前：
  - `cat-file -t`
  - 然后手工断言类型
- 改成：
  - 直接：
    - `find_commit`
    - `find_blob`
    - `find_tree`
    - 或 `find_object` + type check
- 要保留的语义：
  - 错误信息尽量保持“对象类型不匹配”
- 坑：
  - 现在错误文案比较明确；迁移后别让它退化成模糊的底层错误

---

## 14) `Blob::content()`
- 位置：`1275-1281`
- 当前：
  - `git cat-file blob <oid>`
- 改成：
  - `Repository::find_blob(oid)?.content()`
- 要保留的语义：
  - 返回原始 bytes
- 坑：
  - 不要提前做 UTF-8 解码；当前返回的是 `Vec<u8>`

---

## 15) `get_file_content()`
- 位置：`2351-2360`
- 当前：
  - `git show <commit>:<path>`
- 改成：
  - `find_commit(commit)`
  - `commit.tree()`
  - `tree.get_path(path)`
  - `to_object(...).as_blob()`
- 要保留的语义：
  - 返回指定 commit 下该路径的文件 bytes
- 坑：
  - 当前 CLI 对 pathspec 和路径解析比较宽松
  - 迁移后要确认：
    - 路径分隔符
    - 非 UTF-8 path
    - 子目录路径
    - 文件不存在时的错误语义

---

## 16) `Tree::get_path()`
- 位置：`1199-1259`
- 当前：
  - `git ls-tree -z -r <tree> -- <path>`
  - 然后手工 parse 输出
- 改成：
  - `Tree::get_path(path)`
- 要保留的语义：
  - 返回 `oid/object_type/mode/path`
- 坑：
  - 当前代码自己保留了：
    - `mode`
    - `object_type`
    - `path`
  - `git2` 返回的 entry 结构不同，你需要自己重新组装 `TreeEntry`
- 说明：
  - 这一步虽然能迁，但不如前面的 commit/ref graph 热点更急

---

# 第一阶段实施顺序建议

## 批次 A：最稳、最值
先改这些，几乎都是低风险高收益：

1. `revparse_single`
2. `Object::peel_to_commit`
3. `Commit::tree`
4. `Commit::parent`
5. `Commit::parents`
6. `Commit::summary`
7. `Commit::body`
8. `Commit::author`
9. `Commit::committer`
10. `Repository::merge_base`

## 批次 B：图查询热点
11. `CommitRange::into_iter`
12. `CommitRange::length`
13. `CommitRange::is_valid`
14. `Commit::parent_on_refname`

## 批次 C：对象/ref 收尾
15. `head`
16. `find_reference`
17. `references`
18. `object_type` / `find_commit` / `find_blob` / `find_tree`
19. `Blob::content`
20. `get_file_content`

---

# 最容易踩坑的 6 件事

1. **revspec 兼容性**
   - `git rev-parse` 比 `git2` 某些入口更宽松
   - 先确认项目里 spec 的真实范围

2. **detached HEAD 语义**
   - 当前 `head()` 明确 fallback 到 `\"HEAD\"`

3. **ancestor 判断语义**
   - `merge-base --is-ancestor` 和 graph API 的边界条件要核实

4. **message vs summary/body**
   - 别把 `%s/%b` 偷偷变成完整 message

5. **错误文案退化**
   - 现在很多地方的错误语义是人工包装过的

6. **不要顺手重构 diff 路径**
   - 第一阶段只动只读对象/提交图/ref，不碰 `diff*`

---

# 我建议你怎么验收第一阶段

每改完一小批，就重点验证：

- empty repo
- detached HEAD
- annotated tag peel
- merge commit
- branch/refname 简写与全称
- `A == B` 的 ancestor/range
- 文件不存在 / 对象类型不匹配
- 非 ASCII 路径（至少 1 个用例）

---

如果你愿意，我下一条可以继续给你：

## **第一阶段的最小提交切分方案**
比如切成 4 个独立 PR/commit：
- PR1: commit metadata
- PR2: revparse/ref resolution
- PR3: commit graph/range
- PR4: blob/tree/file content

这样你能最低风险推进。