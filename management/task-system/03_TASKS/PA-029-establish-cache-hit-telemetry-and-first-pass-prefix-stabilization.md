# PA-029 建立缓存命中 telemetry 与第一版前缀稳定化

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在 `PA-025` 已完成 build-context 三层观测收口的基础上，完成当前阶段最实际的缓存命中工程任务：

- 把缓存命中从“整轮累计模糊指标”升级为可解释的工程指标
- 做第一版请求前缀稳定化，减少高波动字段对主请求前缀的污染
- 为后续 `PA-024` 的监控面与未来 `Phase 4` compaction 设计提供真实基线

本卡不实现完整 compaction，不提前实现 planner / executor 双会话重构，也不把子代理缓存策略提前做完。

## 交付结果
- 已完成 call-level cache telemetry contract：
  - 区分 `initial_request` 与 `tool_followup`
  - 保留 turn aggregate，同时写入 per-call `provider_call_records`
  - 支持 `cache_miss_input_tokens` 派生字段
- 已完成第一版 prefix mutation observability：
  - `PrefixMutationReason` 已落地
  - normalized-input 与 provider-native 路径均可记录 mutation reasons
  - `BuildContextObservation` 已带出 `prefixMutationReasons`
- 已完成第一版 stable prefix 收窄：
  - stable prefix 仅保留 `BASE_SYSTEM_PROMPT` 与 provider capability note
  - session summary / run goal / long-term memory / image note / truncation note 已移出最早 stable layer
  - provider fallback observation 不再把动态 system/developer 内容误归为 `stable_prefix_text`
- 已完成 session persistence 与前端 store 接线：
  - `TurnTraceRecord` 持久化 `provider_call_records`
  - 前端 `runtime.ts` / `types/runtime.ts` 保留 `providerCallRecords` 与 `prefixMutationReasons`
  - 前端不为展示扩大范围，仅做类型与 store 保真

## 验收标准
- trace 不再只显示整轮 `input / cached / output` 聚合值，而能解释缓存命中主要来自哪类请求
- 至少能回答：
  - 本轮首请求命中多少
  - 本轮 follow-up 命中多少
  - 哪些高波动字段导致前缀变化
- 第一版前缀稳定化后：
  - 不牺牲主任务输出质量
  - 不破坏 `PA-025` 已定义的 build-context 三层观测语义
- 本卡不引入自动 compaction，也不回滚 `PA-025` 已完成边界

## 当前进展
- `PA-029` 已完成开发与定向验证
- 三路子智能体已完成委派、勘察和实现支持：
  - `Mendel`：Telemetry 工作线
  - `Ohm`：Prefix Stabilization 工作线
  - `Mencius`：Verification 工作线
- 当前成果已可以作为 `PA-024` 的底层指标与 trace 语义输入

## 第一版实现冻结边界
- 本卡只做当前阶段最实际、可验证、可被 `PA-024` 消费的基础能力：
  - call-level cache telemetry contract
  - `initial_request` / `tool_followup` request kind
  - prefix mutation reasons
  - 第一版 stable prefix 收窄
- 本卡明确不做：
  - 自动 compaction
  - planner / executor 双会话拆分
  - 子代理缓存复用体系
  - 新增第四层 build-context observation
  - 以 UI 展示需求反向驱动底层 telemetry 结构

## 关键落地文件
- 后端：
  - `src-tauri/src/agent/telemetry.rs`
  - `src-tauri/src/agent/session.rs`
  - `src-tauri/src/agent/runtime.rs`
  - `src-tauri/src/agent/context.rs`
  - `src-tauri/src/agent/provider.rs`
  - `src-tauri/src/agent/turn_flow.rs`
  - `src-tauri/src/agent/control_plane.rs`
  - `src-tauri/src/agent/graph.rs`
  - `src-tauri/src/sse_adapter.rs`
- 前端：
  - `src/types/runtime.ts`
  - `src/stores/runtime.ts`
  - `tests/runtime-store.spec.ts`

## 验证结果
- Rust 定向测试通过：
  - `cargo test --manifest-path src-tauri/Cargo.toml run_turn_accumulates_token_usage_across_tool_followups`
  - `cargo test --manifest-path src-tauri/Cargo.toml start_turn_stream_accumulates_token_usage_across_tool_followups`
  - `cargo test --manifest-path src-tauri/Cargo.toml build_request_keeps_image_and_truncation_notes_out_of_stable_prefix`
  - `cargo test --manifest-path src-tauri/Cargo.toml build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix`
  - `cargo test --manifest-path src-tauri/Cargo.toml file_backend_roundtrip_restores_turn_trace_history`
- 前端定向测试通过：
  - `npm run test:unit -- --run tests/runtime-store.spec.ts`
  - `npm run test:unit -- --run tests/HomeSidebar.spec.ts`

## 已知残留
- `SyncToolTurnOutcome.trace_timeline` 当前未被读取，Rust 会产生 `dead_code` warning，但不影响功能与验证
- `cargo test` 过程中存在若干 Windows incremental 目录 `Access denied` warning，不影响本卡定向测试结论
- `prefix_mutation_reasons` 当前仍是“本次请求中存在的可变因子记录”，不是跨轮真实 diff；这符合本阶段冻结边界

## 与下游关系
- `PA-029` 已完成，且其输出已被 `PA-024` 消费进入正式 monitor read-plane
- `PA-024` 当前消费本卡落地后的：
  - `provider_call_records`
  - `requestKind`
  - `prefixMutationReasons`
  - `stablePrefixText / semiStableContextText / volatileInputText`

## OpenSpec Change
- [add-cache-hit-telemetry-and-prefix-stabilization](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-01-add-cache-hit-telemetry-and-prefix-stabilization)

## Canonical Spec
- [cache-hit-optimization/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/cache-hit-optimization/spec.md)

## Spec 状态
- Proposal: `done`
- Design: `done`
- Tasks: `completed`
- Ready for implementation: `done`
- Change archived: `done`

## 断点续跑提示
继续前优先看：

- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/provider.rs`
- `src/stores/runtime.ts`
- `tests/runtime-store.spec.ts`
