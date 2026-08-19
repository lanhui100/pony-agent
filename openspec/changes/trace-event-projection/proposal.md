# trace-event-projection

## Background

阶段 2（`session-projection-layer`）已把 `TurnTraceRecord` 的组装迁移到 `TraceProjection`，但 trace 表仍是独立权威存储，前端 trace 视图与消息视图仍是两套数据路径：

- `trace_timeline`（`TraceTimelineEntry`）由 `record_turn_trace` 组装，与消息历史无同源关系；
- 前端 `src/lib/runtime/trace.ts` / `trace-projection.ts` 是独立于消息渲染的消费路径；
- `build_context_observation` 等大字段（50-100KB）在 timeline 条目内嵌，前端克隆时被剥离（`trace.ts:57-62`），存在重复存储与传输浪费。

本 change 是 ADR 0008 **阶段 4：trace 事件化**——trace 表降级为 `TraceProjection` 的持久化缓存，前端 trace 视图与消息视图从同一份事件派生。

## Goals

- `TurnTraceRecord` 从独立权威表变为 `TraceProjection` 的缓存（阶段 2 已实现折叠，本 change 完成降级收尾）。
- `trace_timeline` 从事件折叠：`step/start` → `call_model` 条目、`tool/call` → `call_tool` 条目、`tool/result` → `return_result` 条目（tool 字段从阶段 1 扩展的事件字段重建）。
- 前端 trace 视图与消息视图从同一份事件派生，不再两套数据路径。
- `build_context_observation` 等大字段事件外置（事件里存引用，按需加载）。
- `ProviderCallCacheRecord` 迁出 trace：由 `MetricsProjection` 生成（wire 兼容保留字段）。

## Non-goals

- 不做 trace 视图 UI 重构（`trace-panel-virtual-scroll` / `trace-render-snapshot-projection` 已归档的既有方案保持）。
- 不做 trace 的跨会话聚合查询（后续迭代）。
- 不改 `TurnStreamEvent` 推送协议（前端实时渲染路径不动）。

## Scope

- `crates/pony-agent-core/src/agent/`：
  - `turn_persist.rs`：`record_turn_trace` 收尾（trace 表写入改为投影缓存 upsert，带 seq 水位）。
  - `telemetry.rs`：`TurnTraceRecord` / `TraceTimelineEntry` 结构适配（若折叠需要）。
  - `sqlite_session.rs`：trace 表从权威降级为缓存（`SeparateTraceTableMode` 迁移状态机收尾）。
- 前端：
  - `src/lib/runtime/trace.ts` / `trace-projection.ts`：trace 数据来源适配（若 wire 格式不变则零改动）。
  - 大字段按需加载：`build_context_observation` 从事件引用读取。

## Risks

- **trace 表降级的兼容**：`SeparateTraceTableMode` 的 `WriteSeparate`（trace 表权威）语义反转——表变缓存后，清表可重建。缓解：保留迁移状态机字段，语义文档化。
- **重建断言的双标准**：存储记录权威还是事件权威？缓解：**事件权威**——存储记录是缓存，重建后与事件折叠结果一致为准；时钟/序列字段（`updated_at`、`event_id`、`sequence`、`emitted_at_ms`）显式豁免。
- **前端零改动假设**：若 wire 格式（`TurnTraceRecord` JSON）不变，前端零改动；若折叠产生字段差异，走显式版本化迁移（`event_version` 字段已有）。
- **大字段按需加载**：`build_context_observation` 外置后，前端读取路径变化（事件引用 → 按需查询）。缓解：新增 host command（`load_build_context_observation`），查询返回与原始 payload 一致。

## Validation

- 单元测试：timeline 事件折叠映射（逐事件映射表）、trace 缓存水位幂等、大字段引用、ProviderCallCacheRecord 重建（字段集 + 豁免清单）。
- 集成验证：真实 turn 的 trace 视图与消息视图同源（同一事件派生）。
- 回归：既有 trace 测试全绿；验收命令 `npm run cargo:test`。