# Design

## Decision Summary

1. trace 表降级为 `TraceProjection` 持久化缓存：行带 seq 水位，清表可重建（**事件权威**：存储记录是缓存，重建与事件折叠一致为准）。
2. `trace_timeline` 从事件折叠（逐事件映射表见 §2）。
3. 前端 wire 格式保持兼容（`TurnTraceRecord` JSON 不变），前端零改动优先。
4. `build_context_observation` 事件外置：事件存引用，按需加载（新增 host command）。
5. `ProviderCallCacheRecord` 迁出 trace：由 `MetricsProjection`（阶段 2）生成，`TurnTraceRecord.provider_call_records` 降级为投影视图产物（wire 兼容保留字段）。

## Chosen Direction

### 1. trace 表降级

- `SeparateTraceTableMode` 状态机收尾：`WriteSeparate` 语义从"trace 表权威"改为"trace 表 = 投影缓存"。
- 表结构：`(session_id, turn_id, ver, seq, val)`，`seq` 为折叠水位；`val` 为 `TurnTraceRecord` JSON。
- 写入路径：`record_turn_trace` → 折叠新事件 → upsert 缓存行（`seq` 单调）。
- 清表重建：`TraceProjection` 从事件全量折叠（阶段 2 已实现）。

### 2. timeline 事件折叠映射表

```
step/start      → TraceTimelineEntry { kind: "call_model", sequence: step }
tool/call       → TraceTimelineEntry { kind: "call_tool",  sequence: step,
                                        tool_activities: [activity] }   // started_at_ms 来自事件
tool/result     → TraceTimelineEntry { kind: "return_result", sequence: step }
assistant/chunk → 文本聚合到 call_model 条目（text/reasoning_content）
turn/end        → 结算 token 指标（input/output/total/first_token_latency/turn_duration）
```

- `TurnToolActivity` 字段重建：`duration_seconds` ← `ToolResult.duration_ms`；`status` ← `ToolResult.status`；`artifacts` ← `ToolResult.artifacts`；`capability_invocation` ← `ToolResult.capability_invocation`（阶段 1 已扩展事件字段）。
- 与现有 `TraceTimelineEntry` 字段对齐（`telemetry.rs:634-660`）；折叠产生的字段差异走显式映射表。

### 3. 前端同源

- 优先保持 wire 兼容（前端零改动）；若折叠产生字段差异，走显式版本化迁移（`event_version` 字段已有）。
- 前端 `trace-projection.ts` 的投影逻辑保留（它是渲染层投影，与后端事件折叠不冲突——"同源"指数据源同一份事件派生出的 wire 数据，不是删除前端渲染投影）。

### 4. 大字段外置

- `build_context_observation`：事件 payload 存 `{ observation_ref: "bco:<turn_id>:<seq>" }`，全量存独立表（`build_context_observations`）。
- 前端按需加载：新增 host command `load_build_context_observation(ref)`，返回与原始 payload 一致（断言测试）。
- blob 内嵌字段退役：新事件不再重复存储；legacy 内嵌数据保留读取兼容（wire 字段保留，仅新数据走引用）。

### 5. ProviderCallCacheRecord 迁出

- `TurnTraceRecord.provider_call_records` 由 `MetricsProjection` 从 `ProviderUsage` 事件生成（阶段 2 已实现折叠）。
- 重建字段集：request_kind、usage 四桶、cache_hit/cache_miss、prefix_mutation_reasons、first_token_latency_ms、turn_duration_ms、latency_kind、provider/model。
- 豁免字段：`updated_at`、`event_id`、`sequence`、`emitted_at_ms`（时钟/序列语义，显式豁免清单）。

## Verification Strategy

- 单元：timeline 折叠映射（表驱动）、缓存水位幂等、大字段引用（查询返回一致）、ProviderCallCacheRecord 重建（字段集 + 豁免清单）。
- 集成：真实 turn 的 trace 与消息同源验证。
- 回归：既有 trace 测试全绿；验收命令 `npm run cargo:test`。