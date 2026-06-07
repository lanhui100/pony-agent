# PA-033 建立 agent hooks pipeline foundation

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-agent-hooks-pipeline-foundation](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-agent-hooks-pipeline-foundation)

## Delta Spec
- [agent-hooks-pipeline-foundation/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-agent-hooks-pipeline-foundation/specs/agent-hooks-pipeline-foundation/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 hooks pipeline canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
在 lifecycle 与 recovery contract 稳定之后，为 Pony Agent 建立可工业化扩展的 hooks foundation，使 hooks 成为受控扩展面，而不是新的隐式调度层。

## 输出
- hook 分类与权限边界
- lifecycle boundary hook points
- hook failure policy / timeout / recovery mode
- hook registry / executor / trace adapter 的最小骨架目标
- hooks traceability / audit / persistence 原则
- `PA-033` 与 `PA-022` 的范围关系与后续扩展归属矩阵

## 验收标准
- hooks 只允许挂在 canonical lifecycle boundary 上
- hooks 通过结构化结果参与决策，不直接任意改内部 store
- hook 的失败语义与恢复语义有正式合同
- hook 顺序、patch 冲突裁决与持久化证据要求有可执行合同
- hook 执行结果可进入 trace / audit 读面
- 测试覆盖 hook 分类、边界、失败策略、恢复模式与 traceability
- `PA-033` 交付的是 foundation / no-op contract / binding / traceability，不要求把 hooks 正式接入 runtime dispatch 主链

## 当前进展
- 已建立 OpenSpec change：`add-agent-hooks-pipeline-foundation`
- 已把“hooks 可拓展性”的讨论正式收口为 contract-first 范围
- 已明确本卡是对 `PA-022` 中 turn-lifecycle foundation 范围的可执行收口，而不是直接做复杂插件生态
- 已完成独立 spec 审核并采纳修订，见：
  [2026-06-03-pa031-pa032-pa033-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-03-pa031-pa032-pa033-spec-review.md)
- 已完成第一轮 no-op foundation 骨架：
  - 新增 `src-tauri/src/agent/hooks.rs`
  - 已落第一版 hook contract types：`TurnHookPoint / HookClass / HookFailurePolicy / HookRecoveryMode / HookResultKind`
  - 已落 `AgentHookDescriptor / AgentHookRegistry / NoopHookExecutor`
  - 已补 descriptor 校验与 no-op 执行的最小 Rust 单测
- 已完成第二轮 foundation 收口：
  - 已将易冲突的 `HookBoundary` 收敛为 `TurnHookPoint`
  - 已新增 `HookTraceRecord`，明确 no-op foundation 的 traceability 读面模型
  - 已补执行结果到 trace record 的最小投影单测
- 已完成第三轮 foundation 收口：
  - 已把 hook 结果收敛为结构化合同：`observe / allow / deny / patch / side-effect-request`
  - 已新增 `HookDenyDecision / HookPatchOperation / HookSideEffectRequest / HookStructuredResult`
  - 已补 class -> result kind 兼容性校验，避免 descriptor 漂移成“万能 hook”
  - 已补 transform / side-effect 归一化单测，确保 traceability 不再只依赖自由文本 summary
- 已完成本轮定向验收：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests`
  - 结果：最新为 `15 passed`
- 已完成 hook trace persisted roundtrip foundation：
  - `TurnTraceRecord` 已正式承接 `hook_trace_records`
  - `TurnStreamEvent / TurnResult / SessionSnapshot / 前端 TurnTraceRecord` 已打通同一字段合同
  - `runtime store` 已新增 hook trace clone/hydration 消费路径
- 已完成 hook point canonical binding foundation：
  - `TurnHookPoint` 已补正式的 canonical `eventType + phase` 绑定表
  - 已新增 `hook_point_matches_canonical_boundary(...)`，为后续 runtime 接入提供统一匹配逻辑
  - 已补防漂移测试，确保 hooks foundation 不回退到 runtime 私有 phase 猜测
  - 已补 `turn_flow -> hooks binding` 交叉测试，验证当前 canonical event 发射结果与 hooks binding 一致
- 已开始推进 runtime 真实路径对齐：
  - `turn:completed` event payload 的 phase 已从历史 `ready` 收敛为 canonical `completed`
  - runtime failed/cancelled 路径已补 hooks binding 精确断言
  - runtime tool-hop 路径已补 hooks binding 断言，但部分精确测试在本机仍受 Windows `link.exe` 文件锁影响
- 已完成 hook monitor/read model 前后端闭环：
  - 后端 `control_plane` 已把 `hook_trace_records` 聚合为 `hookCallCount / blockedHookCount / avgHookDurationMs / totalHookDurationMs`
  - `ModelMonitorSummaryView` 已新增 `hookClasses / hooks` 两组聚合读面
  - 前端 `ModelMonitorPage` 已消费 hooks overview、session metrics、hook classes、hooks 与 selected trace hook evidence
  - 前端页面单测已覆盖 hooks summary / drilldown / trace evidence 展示
- 已推进 runtime canonical event 收口一小步：
  - `turn_flow::resolve_canonical_event_type(...)` 已收紧为：仅 `building_context` 阶段的 trace + observation 才归类为 `turn.context_built`
  - 避免后续 `calling_model` trace 因携带 `build_context_observation` 被误判为 `turn.context_built`
  - stream tool path 的事件 payload phase 已开始从历史 `calling_tool / calling_model` 收口到 canonical `executing_tool / tool_result_integrating`
  - 已新增 `turn_flow` 防漂移测试，覆盖 `ToolCallStart` 与 `trace_updated` 的归类
  - 前端 `runtime-store` 回归测试已确认 `executing_tool / tool_result_integrating` 仍稳定投影到 UI runtime phase
- 已补 tool-hop limit 失败路径 foundation：
  - runtime 已新增仅测试可用的 hop limit override，允许低成本触发多 hop failure 路径
  - 已新增 `start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_hop_limit_is_hit`
  - 该测试覆盖 `ToolCallEnd -> turn.tool_call_completed` 与 `TurnFinalizeEnd -> turn.failed` 的失败收口链路
- 已补 in-flight cancel 路径 foundation：
  - runtime 已新增 `RequestStopToolExecutor` 测试执行器，在工具执行期间请求 stop
  - 已新增 `start_turn_stream_cancels_with_canonical_finalize_boundary_when_stop_is_requested_during_tool_execution`
  - 该测试覆盖 `ToolCallStart -> turn.tool_call_started` 与 `TurnFinalizeEnd -> turn.cancelled` 的中断收口链路
- 已补 tool execution error 路径 foundation：
  - runtime 已将 `tool_result.status != ok` 从“只做 trace 统计”提升为“显式 failed finalize”
  - 避免工具执行失败后仍继续 provider follow-up，导致 hooks / trace / session control plane 观察到不一致终态
  - 已新增 `ErrorToolExecutor` 与 `start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_execution_errors`
  - 该测试覆盖 `ToolCallStart -> turn.tool_call_started`、`ToolCallEnd -> turn.tool_call_completed` 与 `TurnFinalizeEnd -> turn.failed` 的失败收口链路
- 已吸收本轮子智能体审计结论：
  - 已确认 `PA-033` 应收口为 foundation 卡，不再吸收 checkpoint runtime implementation 或正式 hook dispatch integration
  - 已确认 runtime 真正接线、稳定 boundary dispatch 与 hook trace 实时产物，应拆到独立实现卡推进
  - 已确认 `run / memory write / planner / skills / MCP` hooks 仍留在 `PA-022` 的 post-foundation 范围
- 已完成最终 acceptance audit：
  [2026-06-04-pa033-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa033-acceptance-audit.md)

## 下一步动作
- 将 runtime hook dispatch 与稳定 boundary 接线转交后续实现卡
- 保持 `PA-022` 只承接 post-foundation 扩展面，不回吞 foundation 已完成内容
- 如需归档对应 OpenSpec change，按单独归档流程执行

## 当前卡点
- 暂无；本卡已完成关闭

## 与 PA-022 的关系
- `PA-033` 是 `PA-022` 的 turn-lifecycle foundation 子集
- `PA-033` 完成不自动关闭 `PA-022`
- `PA-022` 中 `run / memory write / planner / skills / MCP` 相关 hooks 仍保留为 foundation 之后的扩展范围

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-022-build-lifecycle-hooks-pipeline.md`
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
