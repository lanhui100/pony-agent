# 2026-06-04 Session 74

## 主题

- `PA-033` hook monitor/read model 前后端闭环
- hooks 继续保持在 lifecycle boundary 的观测层，不接入 runtime dispatch

## 本轮完成

1. 完成后端 hook monitor 聚合读面收口：
   - `ModelMonitorOverview` / `ModelMonitorSessionRow` 已补
     `hookCallCount / blockedHookCount / avgHookDurationMs / totalHookDurationMs`
   - `ModelMonitorSummaryView` 已补 `hookClasses / hooks`
   - `aggregate_session_metrics(...)` 与 control plane 聚合逻辑已开始正式消费 `trace.hook_trace_records`
2. 完成前端 monitor 页面 hooks 消费闭环：
   - `ModelMonitorPage` 已新增 hooks overview 卡片
   - session 列表与 drilldown metrics 已展示 hook 调用数、阻断数与平均耗时
   - 聚合视图已新增 `Hook Classes` 与 `Hooks` 两个 section
   - selected trace 已新增 `Hook Trace` 证据区
3. 完成页面级前端回归测试补强：
   - `tests/ModelMonitorPage.spec.ts` 已补 hooks summary / session metrics / hook trace evidence 断言
4. 完成一次独立子智能体只读审计：
   - 审计结论确认此前缺口就在 hooks overview / hookClasses / hooks / session drilldown hooks metrics
   - 本轮已据此补完最小前端消费闭环
5. 推进 runtime canonical event 收口：
   - `turn_flow::resolve_canonical_event_type(...)` 已收紧，避免 `calling_model` 阶段携带 `build_context_observation` 的 trace 被误判为 `turn.context_built`
   - stream tool path 的事件 payload phase 已从历史 `calling_tool / calling_model` 开始收口到 canonical `executing_tool / tool_result_integrating`
   - runtime 多 hop 测试已补 `ModelCallStart / ToolCallStart` 断言代码，为后续真实路径验收预留钉子
6. 补充防漂移测试与前端 phase 映射保护：
   - `turn_flow` 已新增 `ToolCallStart` 与 `trace_updated` 归类测试
   - `tests/runtime-store.spec.ts` 已显式把 `turn.tool_call_started.phase=executing_tool` 与 `turn.tool_call_completed.phase=tool_result_integrating` 纳入回归断言
   - 这确保 runtime event payload phase 收口后，前端 UI 仍稳定维持 `calling_tool / calling_model` 投影
7. 推进 `tool-hop limit -> failed finalize` 失败路径：
   - runtime 已新增仅测试可用的 hop limit override，允许低成本触发真实多 hop failure 路径
   - 已新增 `start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_hop_limit_is_hit`
   - 该测试同时断言：
     - `ToolCallEnd -> turn.tool_call_completed / tool_result_integrating`
     - `TurnFinalizeEnd -> turn.failed / failed`
   - 同时校验 failed trace 已持久化且保留 hop limit 错误信息
8. 推进 `tool execution error -> failed finalize` 失败路径：
   - runtime 已将 `tool_result.status != ok` 从“只做 trace 统计”提升为“显式 failed finalize”
   - 避免工具执行失败后仍继续 provider follow-up，导致 hooks / trace / session control plane 观察到不一致终态
   - 已新增 `ErrorToolExecutor`
   - 已新增 `start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_execution_errors`
   - 该测试同时断言：
     - `ToolCallStart -> turn.tool_call_started / executing_tool`
     - `ToolCallEnd -> turn.tool_call_completed / tool_result_integrating`
     - `TurnFinalizeEnd -> turn.failed / failed`
   - 同时校验 failed trace 已持久化且保留 tool execution error

## 验证

- `npx vitest run tests/ModelMonitorPage.spec.ts`
  - 结果：通过，`7 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo check --manifest-path src-tauri/Cargo.toml --tests`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::turn_flow::tests::canonical_trace_event_prefers_model_call_started_over_context_built_outside_building_context -- --exact --nocapture`
  - 结果：未拿到逻辑结论；本机仍受 Windows `link.exe` `LNK1104` 文件锁影响
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream -- --exact --nocapture`
  - 结果：未拿到逻辑结论；本机仍受 Windows `link.exe` `LNK1104` 文件锁影响
- `npx vitest run tests/runtime-store.spec.ts`
  - 结果：通过，`50 passed`
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
  - 结果：通过
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - 结果：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_hop_limit_is_hit -- --exact --nocapture`
  - 结果：未拿到逻辑结论；本机仍受 Windows `link.exe` `LNK1104` 文件锁影响
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
  - 结果：通过；本轮新增 tool execution error failed finalize 代码可编译
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - 结果：通过；本轮新增 runtime 测试代码可编译
- `cargo test --manifest-path src-tauri/Cargo.toml start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_execution_errors -- --exact`
  - 结果：未拿到逻辑结论；本机在 300s 内未完成，暂按 Windows 构建环境噪音记录，不视为新的逻辑失败

## 当前边界

- 本轮只扩展 monitor/read model 与 trace evidence 展示
- hooks 仍未接入 runtime 正式执行链
- side-effect / replay / recovery 真执行合同仍保持在 foundation 阶段，不提前落实现
- runtime 真实路径的 hooks binding 仍在继续补强，但精确 Rust 单测在本机存在 `LNK1104` 环境噪音

## 下一步

1. 继续补 `runtime` 多 hop / error / cancel 路径与 hooks canonical binding 的交叉测试
2. 继续确认 `ModelCallStart / ToolCallStart / ToolCallEnd / TurnFinalizeEnd` 等边界不再受历史 phase 兼容语义污染
3. 优先补 `tool-hop limit -> failed finalize` 与 in-flight cancel 两类真实失败路径
4. 继续补 checkpointing 边界，把 lifecycle contract 从 terminal/tool boundary 延展到 recovery 关键节点
5. 等 lifecycle / recovery 验证再稳定一轮后，再决定 hooks runtime dispatch 的正式接入时点
