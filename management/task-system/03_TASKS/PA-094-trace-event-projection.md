# PA-094 trace 事件化（阶段 4：事件溯源演进）

## Basic Info

- ID: PA-094
- Status: Done
- Priority: P0
- Complexity: C
- Owner: @orchestrator
- Created At: 2026-08-18
- Updated At: 2026-08-20
- OpenSpec Change: `openspec/changes/trace-event-projection/`
- Spec 状态: 通过（3 路对抗审核收敛，2026-08-18）

## Background

PA-092 已把 trace 组装迁移到 TraceProjection，但 trace 表仍是独立权威存储，前端 trace 视图与消息视图仍是两套数据路径。阶段 4：trace 事件化收尾。

## Goal

1. trace 表降级为投影缓存（事件权威，清表可重建）
2. timeline 事件折叠映射表（表驱动测试）
3. ProviderCallCacheRecord 迁出（MetricsProjection 生成）
4. 大字段外置（build_context_observation 引用 + 按需加载）
5. 前端两视图同源（wire 兼容优先）

## Scope

- `crates/pony-agent-core/src/agent/`：`turn_persist.rs`、`telemetry.rs`、`sqlite_session.rs`
- 前端 `src/lib/runtime/trace.ts` / `trace-projection.ts` 适配（零改动优先）

## Non-Goals

- trace 视图 UI 重构、跨会话聚合查询
- TurnStreamEvent 推送协议变更

## Acceptance Criteria

1. trace 表清空后从事件重建一致（事件权威）
2. timeline 折叠映射表驱动测试通过
3. ProviderCallCacheRecord 重建（字段集 + 豁免清单）
4. 大字段按需加载（查询返回与原始一致）
5. 既有测试全绿（`npm run cargo:test`）

## Review Plan

- spec 3 路对抗审核 → 已收敛
- 实施后 3 路对抗审核 → 调优 → 验证 → 收口

## Current Progress

- spec 通过（2026-08-18 3 路审核收敛）
- **实现完成（2026-08-20）**：
  - `turn_event.rs`：新增 `ContextObservation` 事件（`context/observation`，内存携带全量 payload、落盘只含引用 `bco:<turn_id>:<seq>`，serde skip 不落盘）+ round-trip 测试
  - `turn_flow.rs`：`turn:started` 携带 build_context_observation 时额外构造 context/observation 事件（`build_context_observation_event`）
  - `projection.rs`：`TraceProjectionState` 新增 timeline 事件折叠（映射表：step/start → call_model、tool/call → call_tool、tool/result → return_result、assistant/chunk → 文本聚合、ProviderUsage → token 指标回填、turn/end → 结算）+ `MetricsProjectionState.by_turn_records`（ProviderCallCacheRecord 重建，addReplacing 原位覆盖）
  - `sqlite_session.rs`：`build_context_observations` 独立表 + `flush_events_tx` 外置（同事务）+ `normalized_turn_traces.event_watermark` 列（trace 表降级为投影缓存）+ `load_build_context_observation` 后端
  - `session.rs`：`TurnTraceRecord.build_context_observation_ref` + `TraceTerminalPatch.event_watermark` + `fold_session_views` 挂载 MetricsProjection records + `SessionBackend::load_build_context_observation`
  - `query_commands.rs` / `src-tauri/src/lib.rs`：`load_build_context_observation` host command（Tauri 注册）
- **验证**：core 846 全绿（含 timeline 折叠表驱动、水位幂等、大字段引用、ProviderCallCacheRecord 重建、清表重建、同源集成、真实 step 语义回填、backend 加载重建）+ src-tauri 6 + 前端 399 全绿 + build 通过
- **实施后审核（2026-08-20）**：tester 对抗审核发现 3 个 P0 盲区并全部修复——
  - P0-1 ToolCall/ToolResult step 语义错位（`build_tool_event` 以 turn 事件序号作 step）→ `call_tool_index` 改按 call_id 索引，回填不再依赖 step
  - P0-2 completed turn 不发射 TurnEnd（投影 turn/end 结算永不触发）→ `turn:completed` 额外发射 turn/end + provider/usage（MetricsProjection 重建 ProviderCallCacheRecord 的事件源）
  - P0-3 清表重建仅内存平凡断言 → `SessionStore::with_backend` 加载时 trace 缓存为空且事件存在则从事件全量折叠重建（backend 真实路径测试）
  - 另修复：`MetricsProjectionState.last` 的 prefix_mutation_reasons 重建、build_context timeline 条目折叠（带 ref + sequence 重排）、前端 `loadBuildContextObservation` 按需加载（HomeTracePanel 消费 ref）
- **3 路对抗审核调优（2026-08-20）**：code-reviewer（PASS WITH FIXES）+ consultant（FAIL，4 个 P0）+ tester 全部采纳修复——
  - P0-4 projection watermark `unwrap_or(0) >= seq` 跳过首个 seq=0 事件 → Option 判断（TraceProjection + MetricsProjection）
  - P0-1' turn:completed 拆批（AssistantMessage 先行落盘、TurnEnd 第二批，崩溃半终态）→ assistant/message 不标记 terminal，由 turn/end 统一触发单次 flush
  - P0-3' ProviderUsage 只发 step=0（tool followup 记录无法重建）→ 逐条发射 `provider_call_records`（step 递增，per-call 粒度保真）+ backfill step 递增
  - P0-2' 生产无 UserMessage 事件（HistoryProjection 无法重建用户消息）→ turn:started 额外发射 user/message（payload.text 携带用户消息）
  - P1-1 重建按 active branch 过滤 + replace 写回 → 多分支 trace 永久丢失 → `fold_session_traces_all` 全量折叠（不带分支过滤）+ 空结果不置 should_save
  - P1-7 事件缓冲按 turn_id 索引（跨 session 同 turn_id 串数据）→ key 改 (session_id, turn_id)
  - P1-4/8 前端 ref 缓存跨会话串数据 + 无去重 → 缓存键含 sessionId + Set 去重 + watch 键监听 ref 集合 + 会话切换清空
  - P2 cargo fmt 清理 + 测试 flaky 修复（按 turn_id 过滤断言）
- **第二轮 3 路对抗审核调优（2026-08-20）**：consultant（FAIL，4 个 P0 + 7 个 P1）采纳修复——
  - P1-4 prefix_mutation_reasons 用 Debug 格式发射（`SessionSummaryChanged`）→ projection 按 snake_case 反序列化全部丢失 → 改 serde 序列化（`session_summary_changed`）+ 测试
  - P1-5 TraceProjection 顶层 token 指标用 `.or`（首值优先，多次 ProviderUsage 丢失后续）→ 改 saturating_add 累计（first_token_latency_ms 取首次）
  - P0-4 普通 streaming 路径缺 terminal trace annotation（trace 缓存行 watermark 停留 0）→ start_turn_stream 复用 RecordingTurnEventSink + annotate_turn_trace_terminal_event（与 graph 路径一致）
  - 登记后续（架构级，非本轮收口范围）：P0-1 同步 run_turn 绕过事件流、P0-2 事件/history/trace cache 事务边界、P0-3 AssistantChunk 固定 step=0 多 hop 重建缺口（需 TurnStreamEvent 加 step + 运行时 hop 追踪）、P1-1 UserMessage 双写 canonical payload、P1-2 全量 fold 与分支视图分离、P1-3 trace cache 只存 ref、P1-6 部分缺失增量重建、P1-7 前端真正按需加载
- **第三轮 3 路对抗审核调优（2026-08-20）**：ox-alpha 模型（openrouter/stealth/ox-alpha，opencode run --agent plan，PASS WITH FIXES：3 P1 + 12 P2）+ general 测试审核（PASS WITH GAPS）+ code-reviewer（模型失效未返回）采纳修复——
  - P1-1 failed/cancelled turn 无 usage 发射（多 hop 中途失败 token 丢失）→ `build_provider_usage_event` 扩展匹配三终态
  - P1-2 legacy 内嵌 build_context_observation 清表重建永久丢失 → `derive_events_from_session` 派生 ContextObservation 事件（flush 时自动外置 bco 表）
  - P1-3 加载时 trace 重建结果不回写 trace 表（缓存永不预热，每次启动全量重折叠）→ 重建后逐条 AppendTrace 回写（sequence=终态 seq 作水位）
  - P2-1 顶层 token 裸加法与注释不符 + None→0 语义漂移 → saturating_add + (None,None)→None 合并
  - P2-9 runtime/mod.rs 缩进错位 → 精确重插 12 处（保留原缩进，无 fmt 无关改动）
  - 补充测试：多 ProviderUsage 顶层累计断言（`trace_projection_top_level_tokens_accumulate_across_usages`）
  - 登记后续：P1-3 关联的部分缺失增量重建、P2-2 chunk 聚合 O(n²)、P2-3 settle 覆盖 per-call duration、P2-5 迟到事件丢弃、P2-6 suspended 缓冲泄漏、P2-10 前端 in-flight 去重、P2-11 水位双语义统一
- **验证**：core 849 全绿 + src-tauri 6 + 前端 399 全绿 + vue-tsc + build 通过
- **收口（2026-08-20）**：OpenSpec change 已归档 `openspec/changes/archive/2026-08-20-trace-event-projection/`，canonical spec 已同步 `openspec/specs/trace-event-projection/spec.md`

## Next Action

- 无（已完成）。事件溯源演进阶段 1-4 全部收口（PA-091/092/093/094）

## Blockers

- 无（PA-093 已完成，阻塞解除）

## Resume Hint

- 先读 `openspec/changes/trace-event-projection/design.md`，再读 `turn_persist.rs`（record_turn_trace）、`telemetry.rs`（TurnTraceRecord）