# Agent Note: 会话上下文装载、撤回状态机与 C5 取消收口架构不变量

Status: implemented

## Problem

真实故障会话 `pa_01a21aa3-533e-7da3-b4af-22774f04699d`（TUI 报错终止后双击 `Esc` 撤回并继续对话）暴露出写入、撤回、装载与并发收口四层之间的结构性契约裂痕：

1. **状态枚举过载（`HistoryNodeKind::TurnCancelled`）**：`TurnCancelled` 同时表达「失败/被取消但仍在分支上的活跃轮」（应当推进分支头并允许被撤回）与「已被 `checkout_history_node` 截断废弃的后缀节点」（不得再被复活或撤回）。`ensure_history_graph` 被迫盲目将 `main_branch.head_node_id` 重置为分支末尾节点，导致撤回后的废弃节点在重载时复活；而 `rewind_turns` / `rewind_plan` 又只过滤 `TurnCommitted`，导致失败轮无法进入双 Esc 撤回菜单。
2. **双模式持久化旁路与折叠漏水**：`persist_session_and_trace_change` 仅对 `TraceTableAuthoritative` 走日志命令流，使 `DualWrite` 会话在 `checkout_history_node` 后绕过 `session.jsonl` 与 `normalized_*` 并退化为轻量索引行，重启后图与游标丢失；此外 `fold::apply_index_projection` 与 `rebuild_history_from_turn_events_when_projection_empty_with_deadline` 在撤回到Turn 0 根 `Checkpoint`（`history` 为空）时跳过节点水合或从旧 `turn_events` 重建，引发已撤回历史复活。
3. **跨层时序与写撕裂**：
   - `store_sparse` 中 `ensure_session` 新建活跃会话时未登记 `lazy.loaded`，Turn 1 进行中再次读写会触发 `ensure_session_hydrated`，把未完成的 `turn_events` 重建为 `legacy-node-1` 并造成用户消息重复；
   - `materialize_last_turn_messages` 未按 `turn_id` 作用域过滤，`seq >= watermark` 跨轮写入时可能将下一轮的 `UserMessage` 误归到上一轮；
   - 失败/取消及内联 `decision` 报错路径分离调用写历史与 `persist_turn_trace`，致使 `HistoryNode` 冻结时缺少 `turn_trace_refs`。
4. **C5 宽限监督解锁先于落盘（Option A 目标面）**：`stop_escalation` 宽限期到点后原 `SinkTerminalEmitter` 直接发射 `turn:cancelled` 解锁 UI，未先持久化取消轮的历史/trace，导致 Turn N+1 的 `prepare_turn` 与滞后 Turn N 工作线程发生读写竞态。

相关前序决策：
- `.agents/notes/implemented/feature/2026-11-10-tui-double-esc-rewind.md`
- `.agents/notes/implemented/architecture/2026-11-11-multi-turn-tool-context-pruning-dsh.md`
- `.agents/notes/implemented/bug-fix/2026-11-12-interrupted-turn-content-preservation.md`
- `.agents/notes/implemented/architecture/2026-11-13-targeted-session-load-and-lazy-store.md`
- `.agents/notes/implemented/bug-fix/2026-11-16-session-history-turn-id-idempotency.md`

## Decision

围绕四个正交架构支柱确立硬性不变量，不保留补丁式修复：

1. **撤回墓碑状态独立建模（`HistoryNodeKind::TurnRewound`）**：
   - 协议层（`pony-protocol/src/session_types.rs`、`src/types/runtime.ts`）新增 `TurnRewound`（`"turn_rewound"`）。`checkout_history_node` 将被截断的后缀节点标记为 `TurnRewound`；`resolve_main_branch_head` 保留有效的非 `TurnRewound` 主分支头，仅在缺失/悬空/指向 `TurnRewound` 时回扫最后一个非 `TurnRewound` 节点。
   - `rewind_store::is_rewindable_turn_node` 允许 `TurnCommitted | TurnCancelled` 进入双 Esc 撤回列表与计划，并拒绝 `TurnRewound`。
   - `persist_session_and_trace_change_impl`（`session/turn_commit.rs`）对所有 `supports_turn_events()` 会话统一经日志命令流持久化图投影（含 `UpdateSessionMeta`）；`fold::apply_index_projection` 与 `sqlite_session_commit` 尊重撤回至根 `Checkpoint`（`turn_count == 0`）的空视图，绝不重放已撤回后缀。
2. **稀疏活跃会话生命周期与事件作用域锁定**：
   - `store_sparse::mark_session_loaded` 与 `SessionStore::ensure_session` 将新建活跃会话立即加入 `lazy.loaded`，消除 Turn 1 期间的误水合与 `legacy-node-1` 伪造。
   - `turn_commit::materialize_turn_messages` 按 `expected_turn_id` 精确匹配事件；`append_turn_with_kind_and_trace_fallible` 在幂等判定前先执行 `ensure_session_hydrated`。
3. **历史 + Trace + 节点图单批原子提交**：
   - 新增 `SessionStore::append_turn_with_kind_and_trace(_fallible)`，在 `commit_history_node_from_live_state` 之前将准备好的 `TurnTraceRecord` 写入staged 状态并将 `PersistCommand::AppendTrace` 与消息、`HistoryNode`、`Cursor`、`Branch` 合并到同一 `persist_commands_batch`；`UpsertOne` 同步更新最新 `HistoryNode` 的 `turn_trace_refs`。
   - `turn_persist.rs`（含内联首跳失败收口 `persist_failed_inline_turn_with_trace` 与取消路径 `persist_cancelled_turn_outcome_with_trace`）统一通过原子接口落盘。
4. **C5 宽限监督持久化先于 UI 解锁屏障 + 滞后线程双闸门（Option A）**：
   - `ExecutionCheckpoint` 新增可选字段 `user_message` 与 `partial_assistant_text`（在 `begin_graph_run_stream`、`start_turn_stream_legacy` 与 `turn_stream` 执行过程中更新）。
   - `control_plane/turn_closeout.rs::SupervisorCloseoutEmitter` 在发射 `turn:cancelled` 解锁 UI 前，先于锁外 flush 缓冲事件，读取 checkpoint（缺失时回退 `turn_events` 的 `UserMessage`），构造闭合取消正文/transcript 与 `TurnTraceRecord` 原子写入 `SessionStore`（按 `turn_id` 幂等），然后才发射 `turn:cancelled` 并推进事件水位。
   - `turn_stream.rs` 完成分支在写入 `SessionStore` 前通过 `turn_has_terminal_status` 拦截滞后线程，在写终态 checkpoint 与发射 `turn:completed` 前通过 `claim_terminal_status(..., "completed")` 二次 CAS 仲裁；`update_execution_checkpoint` 禁止把终态状态覆写回 `running`。

## Alternatives considered

- **继续复用 `TurnCancelled` 并在 `load_history_graph` / UI 过滤**：无法从类型层区分「失败待继续/待撤回轮」与「被撤回墓碑」，`ensure_history_graph` 仍会在重开或 `turn_events` 回放时产生语义冲突。
- **Option B（C5 只加 UI/Turn N+1 等待锁，等滞后线程退出）**：当上游 provider HTTP 连接长阻塞直到 180s 读超时时，UI 无法真正取消解冻或 Turn N+1 会长时间卡死，违背快速中止预期。
- **仅在控制面层做去重或只补单个失败调用点**：无法解决 `turn_stream` 首跳失败、`DualWrite` 投影丢失及稀疏模式 Turn 1 半程水合等根因。

## Consequences

- 正向结果：失败轮与中断轮具备一等的历史、trace、`turn_trace_refs` 与双 Esc 撤回支持；撤回后上下文不复活、C5 超时强停后立即可续轮且零跨轮污染与重复写；冷/热与全量/稀疏重启视图严格一致。
- 约束与代价：`ExecutionCheckpoint` 增加两个可缺省字段（向后兼容旧 JSON），控制面与存储各新增单一职责子模块（`turn_commit.rs`、`turn_closeout.rs`），`store.rs`、`control_plane/mod.rs`、`tests.rs` 行数保持在棘轮基线以内。
- 校验覆盖：`pony-storage` 单元不变量测试 T1–T5（`context_loading_invariants.rs`）与真实 SQLite 端到端闭环测试（`context_loading_pa01a21aa3_e2e.rs`），配合现有 `session_storage_suite`、`runtime_suite`、`tools_stop_suite` 回归护航。
