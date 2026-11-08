# Agent Note: 读路径切到日志真源（ADR 0099 阶段 2-B）

Status: implemented

## Problem

阶段 2-A 之后「文件是真源」只在写侧成立：`load_store()` 仍只读 SQLite 索引，日志里的事实无人消费。
两个后果：

- 索引一旦损坏或 schema 不兼容，会话就不可读——「索引可重建」只是口号；
- 日志与索引可以悄悄分叉（同一会话两边内容不同），而没有任何机制让「真源」生效。

## Decision

1. **语义分权（消除「真源」与「索引视图」的命名歧义）**
   - `load_store()` = **运行时真相**：索引原样 + 日志 fold，同 id 以日志为准；
   - `load_index_store()` = **索引原样视图**，doc 写死只允许 `rebuild_index` 双读比对与维护/诊断，
     **运行时业务路径不得调用**，并用源码调用点守卫测试钉住；
   - `new_read_only`（维护 / `--dry-run` 入口）语义不变。
2. **统一 `AppendHookRecords` 口径**：以日志为准 ⇒ **追加**。索引侧 legacy 行的「按键替换」改为
   `append_legacy_hook_records_tx`，并用「两批命令后 fold 与索引条数/顺序一致」用例钉住。
3. **读折叠预算 800ms**：超限记 `session_fold_slow` 并**整轮降级为读索引**，不阻塞启动，下轮再补。

## 过渡期的两条实现事实（不是放宽断言）

1. **定序规则**：整包/状态写（`save_store` / `commit_boundary`）不走命令流，日志会暂时落后于索引。
   两边都有内容时按 `updated_at_ms` 定序、**日志胜平局**；索引更新时保留索引投影（否则分支切换 /
   图刷新 / observation ref 回填会被读回旧态，实测会让 4 条既有 lib 用例变红）。
   ADR 阶段 2 第 4 项「停写 blob」落地后该分支自然消失，届时改「日志无条件为准」。
   阶段 2-B 的核心证明（索引无行 ⇒ 日志无条件为准）不受此规则影响。
2. **索引独有投影仍从索引行继承**：`trace_migration_state` / `turn_trace_refs` / `workspace_id`，
   并按 refs 过滤顶层 trace、按光标节点快照 hydrate 顶层 history（复用
   `store::hydrate_session_from_node`，不另造口径）——否则 fork 分支的消息/trace 会泄漏到顶层、
   `turn_count` 被算错。

## Verification

- **核心兑现点**（`--nocapture` 实物输出）：
  ```
  [C.1] index_row_exists=false runtime_session={id:pa_2b_read_after_delete, title:hello, turn_count:1, history:["hello", "world"]}
  test session_survives_index_row_deletion_because_the_log_is_the_truth ... ok
  ```
  用例内先断言索引行存在 → `DELETE FROM sessions` → 索引两条入口都读不到 → 运行时 `load_store()` 与
  `SessionStore::list_sessions()` 仍返回完整会话。
- `--test session_read_from_log` → 8 passed / 0 failed（Lead 独立复跑）
- `--test session_fold_rebuild` → 9 / 0；`session_log_wiring` → 7 / 0；`session_log_store` → 6 / 0
- `--lib` → 1393 / 0（含新增 5 条 `read_path_merge_*`）
- `storage_stage0_*` 6 套 → 11 / 0
- `cargo check` → 0 warning
- `check-file-size` → 本提交 0 新增违规（唯一红项为另一 lane 的 `runtime/turn_stream.rs` 1707 > 1695）

## Alternatives considered

1. **语义分权（`load_store` 合并 / `load_index_store` 原样）** —— 采纳。理由是两条读取意图真的不同：
   运行时要不惜代价读到真相，双读比对必须看到「索引自己说什么」，否则 `mismatched` 恒为零、比对失去意义。
   同时它让 2-A 已冻结的 allow-list 用例无需改动。
2. **字面切读：`load_store()` 无条件改读日志，2-A 用例判红由另一 lane 更新** —— 否决。会让已冻结的
   验收用例失效，且「索引视图」失去唯一入口，`rebuild_index` 的双读比对将无法自证。
3. **索引优先（只在索引缺行时才读日志）** —— 否决。这等于让索引继续当真源，
   「删了索引不丢数据」这条承诺永远无法被验证，重构目的落空。
4. **一开始就停写 blob，消除过渡期定序规则** —— 更纯，但会把「读切换」与「写模型收窄」两件事
   耦合在一次提交里，回滚面变大。按 ADR 阶段 2 第 4 项单独立项。
5. **读折叠超预算时阻塞等待** —— 否决。启动阻塞比「这轮读旧索引」代价大得多，
   且降级有日志可观测、下轮可补。
