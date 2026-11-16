# Agent Note: 会话历史按 turn_id 幂等——同一轮不得写出两条 user（及两条 assistant）

Status: implemented

## Problem

用户报告同一句话在会话里出现两次（`snapshot.history = [user, user, assistant, …]`）。现场取证（`~/.local/share/PonyAgent/sessions.db`，只读）：

- 某会话该轮的 `turn_events` 只有 **1 条** `user/message`（seq=2），`normalized_messages` 也只有 **1 行**（去重键 `{turn_id}-{role}`，见 `agent/sqlite_session.rs:1137-1140`）；
- 但 `normalized_history_nodes.snapshot_json.history` 是 `[user(turn), user(turn), assistant(turn), …]`；
- 全库 58 个节点快照存在连续重复 user 行。

根因有两条，且互相独立：

1. **写路径不幂等**：`SessionStore::append_turn_fallible_with_kind`（`agent/session/store.rs:660`）直接 `staged_session.history.push(...)`；当事务被重放（同一 `turn_id` 二次 append，例如事件物化回退到 caller text 后又被上层重试）时，整轮（user + assistant）被再写一遍。索引层靠 `INSERT OR REPLACE` 把重复折叠了，所以只有内存历史/节点快照固化下来——`sqlite_session.rs` 的日志 `append_turn: no turn-scoped events … falling back to caller text` 正是回退分支的脚印。
2. **重建路径不幂等**：`history_from_turn_events` / `history_from_branch_events`（`agent/session/history.rs`）把事件流折叠成历史时，对同一 `turn_id` 的重复 `user/message` 事件无条件各 push 一条；投影状态（`HistoryProjectionState`）没有任何去重状态。

## Decision

把「同一 `turn_id` 只写一轮」提升为写路径与重建路径**共用同一身份口径**：

1. 新增 `pub(crate) fn history_contains_turn(history, turn_id) -> bool`（`session/history.rs`）：只认 `turn_id`，`turn_id` 缺失（旧直连用法、摘要行）一律视为「未写过」，老行为不变。
2. 写路径：`append_turn_fallible_with_kind` 在写入前用该判据检查「本会话历史是否已有该 turn」，命中即**幂等重放**——不追加、不覆盖首次写入（首次写入是真实事实，重放是重复事实）。
3. 重建路径：新增 `dedupe_user_messages_by_turn`，只对 `role == "user"` 按 `turn_id` 保留**第一条**；assistant 行是模型真实产出，不参与去重（去重不得连带吞掉回复）。`history_from_turn_events` 与 `history_from_branch_events` 共用这一层，不留第二套判据。
4. 判据**只认轮次身份、不按内容**：不同轮次即使文本完全相同也各留一条（按内容去重会把用户合法的重复提问误合并）。

## Alternatives considered

- **A. 按文本内容去重**：落选。用户重复提同一句问题（追问/复述）是合法行为；内容去重会静默吞掉真实发言。
- **B. 只修重建路径（`history_from_turn_events`）**：落选。现场重复是在**写路径**产生的（事件层只有 1 条 user，节点快照却有 2 条）；只修读路径，重启后再次落盘仍会复发。
- **C. 用索引层 `normalized_messages` 的去重键 `{turn_id}-{role}` 作为唯一真源**：落选。索引层是派生视图，内存历史与节点快照是另一个写入面；让读路径去信任派生层，等于把「谁是事实」倒置。
- **D. 在 `append_turn` 里检测重复后「覆盖」本轮**：落选。覆盖会让重放覆盖掉首次真实写入（首条可能带真实 attachments / transcript），且「覆盖 vs 忽略」在多线程重试下不确定；保留首次是确定性语义。
- **E. 顺手把 assistant 也纳入重建去重**：落选。重建时同一 turn 的两条 assistant 可能来自不同 step/hop（流式/工具的合法分段），按 turn 去重会误删真实产出；assistant 的重复只由写路径重放造成，故只在写路径拦。

## Consequences

- 同一 `turn_id` 重放不再产生重复 user/assistant 行；不同轮次不受影响（含文本相同的合法重复提问）。
- 事件层与索引层无需迁移：去重发生在「写进内存历史/节点快照」和「事件重建」两个入口，历史数据保持原样，旧会话按现有内容读取（已被污染的 58 个节点不做自动改写，需要时按既有折叠/重建路径重算）。
- `turn_id == None` 的旧直连用法行为逐字不变（无身份可判 ⇒ 不幂等），这是已知边界：那类调用本来就在一条「无轮次身份」的路径上。
- 验收：`tests/b222_history_rebuild_idempotent.rs`（6 例，含 store 级重复 append 与「不同轮次不得合并」守护）。

## 契约变更登记（ADR 0123 T-A1b 迁移）

`tests/adr0123_supplement_r2_test.rs::t_a1b_same_turn_id_append_semantics_and_dedupe_key` 原先断言「同 turn_id 两次 append ⇒ 四条并列，store 层不去重，幂等由运行时 gate 的历史腿保证」。这条契约的前提被现场证伪：`normalized_history_nodes` 有 58 个节点快照含重复 user 行，说明运行时 gate 并未兜住所有重放路径（`sqlite_session_commit.rs` 的重建日志与 `store.rs` 的回退日志同时出现）。因此本卡把幂等责任从「调用方自觉」下移到存储写入口，并按 owner 批准显式迁移该用例断言（保留首次写入、同 turn 两条、去重键两条）。迁移不是测试放水：新断言的强度更高（覆盖重放文案不同时仍保留首写），且与验收测试 `b222_store_append_same_turn_id_is_idempotent` 同口径。

