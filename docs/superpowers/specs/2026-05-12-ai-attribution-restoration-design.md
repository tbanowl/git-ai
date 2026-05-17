# AI 归属恢复设计

## 问题

当 AI 生成的代码被人为删除后，如果之后又由人手动恢复，只要恢复后的代码在实质上仍然是同一段代码，就应该继续保留 AI 归属。当前普通的 checkpoint → working log → authorship note 流程主要依赖位置关系：一旦某个带 AI 归属的范围被删除，标准流程里就没有持久记录说明这段被删除文本原本由 AI 生成。因此，后续由人插入的相同代码会被标为人类编写，即使它实际上是在恢复之前的 AI 代码。

## 目标

- 在同一个 working log 生命周期内，人为恢复此前带 AI 归属的代码时，继续保留 AI 归属。
- 支持轻微格式差异：换行符归一化、缩进变化、行尾空白变化。
- 避免常见片段或歧义片段产生误归因；匹配不清楚时优先标为人类归属。
- 第一版聚焦普通 checkpoint 流程，不引入跨 commit 的历史追溯查询。

## 非目标

- 第一版不支持跨 commit 恢复。如果 AI 代码被删除并提交，然后在之后的 commit 中恢复，这需要单独设计基于历史的方案。
- 不把实质性改写后的代码归为 AI。如果非空白 token 发生变化，恢复后的行或代码块应保持人类归属。
- 不引入全仓库范围的内容匹配。匹配范围应保持保守，避免把常见代码片段错误归为 AI。

## 推荐方案

在 working log 中增加轻量的“已删除 AI 片段记忆”，建模为 tombstone。当 human 或 known_human checkpoint 处理到一次非移动场景下的 AI 归属文本删除时，记录一个 tombstone，包含：

- 文件路径
- 原始 AI author id 和时间戳
- 被删除的原始文本
- 用于恢复匹配的归一化文本
- 删除位置附近的小范围上下文
- checkpoint 时间戳或顺序信息

后续 human 或 known_human checkpoint 处理插入文本时，流程如下：

1. 继续优先使用现有的同 checkpoint move detection 来保留归属。
2. 对剩余的人类插入内容做归一化，并与同文件的活跃 tombstone 比对。
3. 如果只有一个安全的 tombstone 匹配成功，则把恢复后的范围归给原 AI author。
4. 如果多个 tombstone 都匹配，或内容过于通用、无法安全区分，则保持人类归属。
5. AI checkpoint 不消费 tombstone，因为 AI 插入内容已经会通过当前 AI checkpoint 逻辑声明归属。

## 匹配语义

第一版应支持用户确认的“轻微格式差异”行为：

- 归一化 CRLF 和 LF 换行符。
- 忽略仅缩进变化。
- 忽略行尾空白变化。
- 要求非空白 token 序列保持一致。
- 多行删除/恢复优先使用 block 级匹配。
- 单行匹配只在该行足够具体，且在活跃 tombstone 中没有歧义时允许。

这样可以让格式化变体仍然被识别为同一段 AI 代码，同时避免实质性 token 变化错误继承 AI 归属。

## 数据流

这个改动应放在 checkpoint/working-log 层，而不是 blame 输出层。`src/authorship/attribution_tracker.rs` 已经负责决定旧归属如何转换为新的字符级和行级归属；`src/authorship/working_log.rs` 与 `src/git/repo_storage.rs` 负责持久化当前 base commit 的 checkpoint 状态。tombstone 应与这些 working-log 状态放在一起，使删除记忆在当前 working log 被提交或丢弃前一直可用。

`src/authorship/post_commit.rs` 中的 commit note 生成逻辑应继续像现在一样消费最终行级归属。只要恢复匹配在 checkpoint 处理期间把原 AI author 分配给恢复后的范围，现有 post-commit authorship note 流程就应该自然序列化这些恢复后的 AI 行范围。

## 歧义与安全规则

- 第一版只支持同文件匹配。跨文件恢复不在初始范围内。
- 同一个 checkpoint 中的 delete + insert 仍由现有 move detection 优先处理。
- tombstone 在一次高置信匹配后应被消费或标为 inactive，避免重复误用。
- 很短或很常见的片段不应匹配，除非它属于更大的 block，或具备强局部上下文。
- 如果无法得到唯一最佳匹配，插入内容保持人类归属。

## 测试

在 `tests/integration/simple_additions.rs` 中增加聚焦回归测试。由于这个场景依赖精确 checkpoint 顺序，测试应使用显式 `fs::write`，配合 `mock_ai` 和 `mock_known_human` checkpoint，而不是高级 `set_contents` 辅助 API。

必测场景：

1. AI 添加一行，人类删除该行，人类以完全相同文本恢复该行，恢复后的行仍为 AI。
2. AI 添加多行代码块，人类删除该块，人类用缩进或行尾空白轻微变化恢复该块，恢复后的块仍为 AI。
3. 人类恢复相似文本，但有实质性 token 变化，恢复后的文本保持人类归属。
4. 存在多个相同或歧义的已删除 AI 片段时，除非能唯一识别匹配，否则恢复内容保持人类归属。
5. AI 添加多行，人类删除其中几行，AI 再添加几行，人类以完全相同文本恢复该行，恢复后的行仍为 AI。

实现后先运行新增回归测试，再运行 `task test TEST_FILTER=<new_test_name>`，随后根据本 Rust 项目要求运行 `task build`、`task lint` 和 `task fmt`。

## 风险

主要风险是误归因：常见代码片段可能因为匹配到之前删除的片段而被错误标为 AI。该设计通过限制 tombstone 生命周期为当前 working log、优先 block 匹配、要求 token 等价，并在匹配有歧义时回退到人类归属来降低风险。

第二个风险是持久化复杂度。第一版应复用现有 working-log 存储模式，避免引入单独数据库或全局索引。
