> 2026-08-22 归档同步：实现已随 PA-091/092 提交（6395ec7^..e88536f）落地并经全量验证（实施状态记录见文末），勾选为归档前簿记同步，非本轮新实施。

# Tasks

- [x] `sqlite_session.rs`：`turn_events` 表 schema + `idx_turn_events_turn` 索引 + `turn_events_archive` 表（定案语义）+ `store_metadata` seq 计数器（`turn_event_seq:<session_id>`，与事件写入同事务）+ `event_schema_version`。
- [x] 事件类型定义：`TurnEvent` enum + payload struct（`turn_event.rs`），serde round-trip 测试（语义等价 + 字段级一致；含 `UserMessage`、`HistorySquash`、`ProviderUsage`、`ToolCall`/`ToolResult` 扩展字段、`branch_id`）。
- [x] `TurnEventSink` trait 扩展：`persist(&self, event: TurnEvent)` 默认空实现；生产 sink 缓冲 + turn 终态 flush；测试 `RecordingTurnEventSink` 收集。
- [x] `PersistCommand` 扩展：`FlushEvents { epoch, session_id, turn_id, events }`，与快照 upsert 同事务（先事件后快照）。
- [x] `write_full_store` 增加 `#[cfg(test)]` 注入开关（事件行已写、快照未写之间强制失败），验证事务回滚。
- [x] 事件名 → variant 映射表（8 种现有发射名 + 新增 emit 点：`StepStart`/`StepEnd`/`UserMessage`/`HistorySquash`）；`turn:output_end`/`turn:trace` 显式声明不落盘。
- [x] provider 结算事件化：`build_provider_call_cache_record` 调用点（`runtime/mod.rs:6407` 附近）append `ProviderUsage`（builder 保持纯函数，事件 append 在调用方）。
- [x] chunk 写时聚合：emit 时按 `(turn_id, step)` 合并到内存缓冲，每逻辑事件一行、seq 连续；不做事后合并。
- [x] 迁移回填：从 blob 反推事件（`chunk_missing`/`backfill_partial`/`turn_boundary_synthetic` 标记），per-session 水位幂等（`turn_event_backfill:<session_id>`）。
- [x] 单元测试：事件序列化、seq 连续性（0-based + 并发 Barrier 双线程各 100 条）、flush 事务原子性（注入崩溃）、回填逐 session 幂等、写时聚合、事件名映射表驱动。
- [x] 集成验证：真实 turn 后 `turn_events` 与 `TurnStreamEvent` 推送序列一致。
- [x] 回归：既有 session/trace/checkpoint 测试全绿；**验收命令 `npm run cargo:test`**（`npm run verify` 仅前端单测 + cargo check，不等于 Rust 回归通过）；既有 runtime 事件断言测试适配。

## Validation Notes

- 本 change 是 ADR 0008 阶段 1，范围严格限定"事件层落地"，投影/checkpoint/trace 演进见后续 change（`session-projection-layer`、`checkpoint-event-referencing`、`trace-event-projection`）。
- 双写是迁移期策略，阶段 2 完成后快照降级为投影缓存，双写变单写。
- 对抗审核（2026-08-18）已采纳：`user/message` 事件（P0-1）、缓冲模型 + 终态 flush（P0-2）、seq 分配单写者 + 0-based（P0-3）、`history/squash` 压缩事件（P0-4）、失败分层 fail-loud vs contained（P0-5）、验收命令修正（P0-6）、事件名映射表（P1）、写时聚合（P1-3）、双 usage 计数规则（P1-4）、ToolCall/ToolResult 扩展字段（P1-5）、归档定案（P1-7）、`TurnEventSink::persist` 注入机制（P1-8）、回填数据损失承认 + per-session 幂等（P1-11）、`event_schema_version`（P1-12）。
- **实施后对抗审核（2026-08-18，tester）采纳与降级声明**：
  - P0-1 快照同事务 → **显式降级**：FlushEvents 事务保证事件层原子性（事件行 + 计数器）；快照由既有独立事务路径写入，'事件+快照同事务' 推迟到阶段 2（投影层重构快照写入路径时统一）。spec `Terminal flush atomicity` 已加 NOTE。
  - P0-2 新 emit 点（UserMessage/StepStart/StepEnd/HistorySquash/ProviderUsage）→ **显式豁免**：阶段 1 运行时覆盖 6 类核心事件，其余 emit 点归入阶段 2；回填已覆盖 legacy 写入。spec `Emission mapping` 已加 NOTE。
  - P0-3 persist_failed 标记 → 归入重试机制（后续 change）；失败日志含 `(session_id, turn_id, event_count, first_event_type)`。spec `Persistence failure containment` 已更新。
  - P1-1 补链路测试（`emit_event_persist_channel_buffers_aggregates_and_flushes`）；P1-2 补 reason 断言；P1-4 补回填边界测试（`backfill_resumes_partially_backfilled_sessions`、`backfill_derives_tool_and_provider_events_from_trace`）；P1-3 schema 版本读侧校验随阶段 2 读路径；P2-1 并发测试对齐 100 条；P2-4 计数器缺失兜底（MAX(seq)+1）；P2-5 reasoning-only chunk 落盘。
- **实施状态**：core 811 测试通过（含新增 14 个）。实现偏离记录：ProviderUsage 运行时 emit 点豁免（见上）、`turn:tool` 的 started_at_ms 恒 None（阶段 2 补）。