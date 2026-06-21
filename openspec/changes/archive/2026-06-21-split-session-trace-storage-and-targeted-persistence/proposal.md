# Proposal: Split Session Trace Storage And Expand Targeted Persistence

## Why

Pony Agent 已经通过 Priority 1 将 turn 完成前的主卡顿从秒级压到毫秒级：`record_turn_trace` 等 trace 热路径不再触发 SQLite 全量 `write_full_store`，而是改走单 session 定向 upsert。

但当前架构仍有一个长期限制：`turn_trace_history` 仍然内嵌在 `sessions.session_data` blob 中。这会带来三个问题：

- 单 session trace 增长仍会放大 JSON 序列化成本
- trace 持久化与 session metadata 写入仍然耦合
- monitor / control-plane / history restore 虽然都消费 trace，但底层仍缺少真正的 trace 级读写边界

现阶段若继续扩展 targeted persistence 而不拆分 trace 存储，后续优化会越来越受 `session_data` blob 限制。

## What Changes

- 为 live session trace 引入独立持久化单元（例如 SQLite `session_turn_traces`）
- 保持 `SessionSnapshot.turn_trace_history` 读面不变，由 backend 在加载时组装
- 为 backend 增加 trace 级读写边界，使 `record_turn_trace`、terminal annotation、hook append 与 failed-turn trace 更新不再依赖 session blob
- 在加载路径上支持受迁移状态约束的 dual-read：迁移期优先新 trace 存储并按需 fallback 到旧 `session_data.turn_trace_history`，稳定后切到新表单源
- 明确 `HistoryNode.turn_trace_history` 继续作为历史快照真相源，而 `session_turn_traces` 只承载当前 live session / 当前分支的可变 materialization
- 明确 rollout 顺序固定为 `Off -> DualWrite -> WriteSeparate`，禁止从 `Off` 直接切到 `WriteSeparate`

## Impact

- trace 高频更新将进一步从 session blob 脱耦，降低长期序列化成本
- session metadata 与 trace persistence 的职责边界更清晰
- 为后续 session-local targeted persistence 扩展和异步持久化打下更稳定的基础
- 历史图、恢复、monitor 与 control-plane 在上层 contract 上保持兼容，但 backend assembly 逻辑会变得更明确

## Tracking

- Task card: `PA-058`
- OpenSpec Change: `split-session-trace-storage-and-targeted-persistence`
