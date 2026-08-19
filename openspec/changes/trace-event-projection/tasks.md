# Tasks

- [ ] trace 表降级：`SeparateTraceTableMode` 状态机收尾（`WriteSeparate` 语义改为投影缓存，事件权威），行带 seq 水位。
- [ ] `record_turn_trace` 收尾：写入改为投影缓存 upsert（复用阶段 2 的 `TraceProjection`）。
- [ ] timeline 事件折叠：`step/start`/`tool/call`/`tool/result`/`assistant/chunk`/`turn/end` → `TraceTimelineEntry` 映射表（表驱动测试）+ `TurnToolActivity` 字段重建（duration/status/artifacts/capability_invocation）。
- [ ] `ProviderCallCacheRecord` 迁出：由 `MetricsProjection` 生成（wire 兼容保留字段），trace 不再独立存储；重建字段集 + 豁免清单（updated_at/event_id/sequence/emitted_at_ms）。
- [ ] 清表重建验证：trace 表清空后从事件全量重建，与存储值一致（事件权威）。
- [ ] 大字段外置：`build_context_observation` 事件引用 + `build_context_observations` 表 + `load_build_context_observation` host command（返回与原始 payload 一致断言）。
- [ ] 前端兼容验证：wire 格式不变则零改动；有差异则显式版本化迁移。
- [ ] 单元测试：timeline 折叠映射（表驱动）、缓存水位幂等、大字段引用、ProviderCallCacheRecord 重建（字段集 + 豁免清单）。
- [ ] 集成验证：真实 turn 的 trace 视图与消息视图同源。
- [ ] 回归：既有 trace 测试全绿；验收命令 `npm run cargo:test`。

## Validation Notes

- 本 change 是 ADR 0008 阶段 4，依赖阶段 2（`session-projection-layer`）的 `TraceProjection` 与阶段 1 的扩展事件字段（ToolCall/ToolResult/ProviderUsage）。
- 前端零改动优先；wire 差异走显式版本化（`event_version` 已有字段）。
- 大字段外置只针对新事件，legacy 内嵌数据保留读取兼容。
- 对抗审核（2026-08-18）已采纳：事件权威 + 豁免清单（P1-9）、tool 字段重建（P1-5）、大字段查询 API 定义（P2）、同源语义澄清（P2）、ProviderUsage 补 prefix_mutation_reasons（P1-9）。