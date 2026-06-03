# 2026-06-01 Session 57 - PA-029 交付与验证完成

## 本次目标

将 `PA-029` 从“实现边界已冻结”推进到“已交付、已验证、已回写任务系统”的完成态。

## 本次动作

1. 汇总主线程已经落地的后端、前端与持久化改动
2. 基于实际测试结果确认 `PA-029` 已满足任务卡与 OpenSpec 的完成边界
3. 回写 `01_TASK_BOARD.md`、`00_DASHBOARD.md` 与 OpenSpec `tasks.md`
4. 将 `PA-029` 正式切换到 `Done`，并确认其输出已被 `PA-024` 承接

## 交付结论

- 已完成 call-level cache telemetry contract：
  - 区分 `initial_request` 与 `tool_followup`
  - 每轮保留 turn aggregate，同时落地 per-call `provider_call_records`
  - 记录 `first_token_latency_ms` 与 `cache_miss_input_tokens`
- 已完成 prefix mutation observability：
  - `PrefixMutationReason` 已在请求构建链路中稳定落地
  - `BuildContextObservation`、`TurnTraceRecord`、`TurnResult`、`TurnStreamEvent` 已可透传相关字段
- 已完成第一版 stable prefix 收窄：
  - 最早 stable layer 不再混入 image note、truncation note 与动态 system/developer 说明
  - 继续保留 `PA-025` 的三层 build-context observation 契约不变
- 已完成前后端接线：
  - 后端持久化与 SSE/turn-flow 透传完成
  - 前端类型与 store 保留 `providerCallRecords`、`requestKind`、`prefixMutationReasons`

## 验证记录

- Rust 定向测试通过：
  - `file_backend_roundtrip_restores_turn_trace_history`
  - `run_turn_accumulates_token_usage_across_tool_followups`
  - `start_turn_stream_accumulates_token_usage_across_tool_followups`
  - `build_request_keeps_image_and_truncation_notes_out_of_stable_prefix`
  - `build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix`
- 前端定向测试通过：
  - `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - `npm run test:unit -- --run tests/HomeSidebar.spec.ts`

## 已知残留

- `SyncToolTurnOutcome.trace_timeline` 仍有 `dead_code` warning，但不影响本卡完成态
- Windows incremental 目录存在若干 `Access denied` warning，不影响本轮定向验证结论
- `prefix_mutation_reasons` 当前是本次请求内的可变因子记录，不是跨轮真实 diff；该限制符合本阶段冻结范围

## 下一步最小动作

1. 继续维持“缓存命中优先级最高，但实现节奏服从当前工程阶段”的路线约束
2. 若继续做缓存优化，基于 `PA-029` 与已完成的 `PA-024` 观测口径拆新的增量任务卡
3. 保持当前阶段不提前实现完整 auto-compaction
