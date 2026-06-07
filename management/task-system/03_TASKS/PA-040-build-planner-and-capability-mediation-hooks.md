# PA-040 构建 planner 与 capability-mediation hooks

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- [add-planner-and-capability-mediation-hooks](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-planner-and-capability-mediation-hooks)

## Delta Spec
- [planner-and-capability-mediation-hooks/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-planner-and-capability-mediation-hooks/specs/planner-and-capability-mediation-hooks/spec.md)

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 hooks 扩展到 `LocalTurnPlanner / GraphPlanner / capability bridge / skills ingress` 这些决策与能力中介层，让系统能在“不绕开 planner / capability registry 真相源”的前提下，统一观察、拦截和改写高层决策输入输出。

## 输出
- planner / capability mediation hook point 第一版
- normalized planner facts / capability mediation envelope
- guard / transform / observe 在 planner 与 capability 层的适用边界
- 与 `PA-019 / PA-020 / PA-021 / PA-033 / PA-035` 的职责分工
- planner / capability read-plane 与验收矩阵

## 验收标准
- planner hooks 只消费 normalized planner facts，不依赖 provider raw protocol 或 UI 私有状态
- capability hooks 只消费 capability bridge / skill ingress 的规范化 envelope，不直接绕开 registry
- hooks 不得把 planner/capability mediation 变成第二 scheduler 或第二 capability registry
- transform 必须对白名单字段生效：planner 与 capability 各自的允许改动面、只读字段与阻断字段都要有可测试合同
- planner evidence 至少覆盖 `planner preflight / tool selection / graph decision`，并进入 `hook_trace_records -> turn trace -> session snapshot`；capability evidence 至少覆盖 `capability resolve / skill mediation`，并可由 control-plane drilldown 读回
- 本阶段 `monitor` 投影若未完全落地，不作为本卡关闭前置；但 control-plane / session snapshot 读面必须可验证
- 测试覆盖 `planner preflight / tool selection / graph decision / capability resolve / skill mediation / failure policy`，以及 session snapshot / control-plane drilldown 的 reload/read-plane 断言

## 当前进展
- `PA-019` 已稳定 graph planner 边界
- `PA-020 / PA-021` 已稳定 capability bridge 与 skills registry 边界
- `PA-033 / PA-035` 已把 hooks foundation 与 turn stable-boundary runtime dispatch 收口
- `PA-040` 负责在这些稳定高层边界上建立 hooks mediation，而不是回到 runtime 私有分支去补丁式扩展
- 已完成一轮独立 spec 审核并采纳修订，见：
  [2026-06-04-pa038-pa039-pa040-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa038-pa039-pa040-spec-review.md)
- 已完成第二轮独立 spec 收紧审阅并采纳修订，见：
  [2026-06-05-pa038-pa039-pa040-spec-tightening-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa038-pa039-pa040-spec-tightening-review.md)
- 已完成一轮只读实现入口勘探：
  - planner hooks 首选挂点已收敛到 `Runtime::decide_graph_after_turn_with_planner(...)`、`LocalTurnPlanner::preflight_decision(...)`、`LocalTurnPlanner::select_tool_call(...)`
  - capability mediation hooks 首选挂点已收敛到 `CapabilityRegistry::resolve_tool_call(...)` 与 `CapabilityRegistry::resolve_skill_tool_actions(...)`
  - ingress/read-plane 辅助挂点收敛到 `HostControlPlane::apply_mcp_source_snapshot(...)` 与 `apply_skill_source_snapshot(...)`
- 已启动第一段 contract/scaffolding 实现：
  - `hooks.rs` 已开始定义 `PlannerHookPoint / CapabilityMediationHookPoint`
  - 已开始定义 `PlannerFactsEnvelope / CapabilityMediationEnvelope`
  - 已开始定义 planner / capability mediation 各自独立的 transform 白名单与只读字段合同
- 已完成第一条真实 evidence 接线（sync turn path）：
  - `run_turn(...)` 当前已开始记录 `planner preflight` 与 `planner tool selection` 的 hook trace
  - `execute_registered_tool_call(...)` 当前已开始记录 `capability resolve / skill mediation` 的 hook trace
  - 上述 evidence 已进入现有 `hook_trace_records -> turn trace -> session snapshot` 链路
- 已把同一批 evidence 接入 stream turn path：
  - stream tool path 现在会把 planner / capability / tool boundary evidence 归并进 terminal trace
  - stream no-tool 完成态现在会保留 planner evidence，而不是只剩 checkpoint/finalize 尾部记录
- 已把 `graph decision` evidence 接入 graph-run 主路径：
  - `control_plane` 在 graph planner 形成最终决策后，会把 `planner.graph_decision.observe` 追加到对应 turn trace
  - sync `advance_graph_run(...)` 与 stream `run_graph_turn_stream(...)` 都会把这条 evidence 写回 session truth-source
  - `GraphRunTurnResponse.turn_result.hook_trace_records` 现在也会立即携带这条 graph decision evidence
- 已补第一条 capability read-plane 验证证据：
  - `monitor_and_drilldown_read_runtime_generated_capability_hook_evidence` 已证明 `capability.resolve.observe` 可被 session runtime view、monitor session drilldown 与 monitor summary 同时读回
- 已把 capability / skill source ingress 收敛为 source drilldown 事实，而不是继续塞进 turn hook trace：
  - `CapabilitySourceView / SkillSourceView` 已新增 `last_ingress_observation`
  - `apply_mcp_source_snapshot(...)` 与 `apply_skill_source_snapshot(...)` 现在会在 registry/runtime 两侧同步写入 ingress observation
  - `apply_mcp_source_snapshot_updates_read_plane_and_runtime_registry` 与 `apply_skill_source_snapshot_updates_read_plane_and_runtime_registry` 已证明 control-plane source drilldown 与 runtime registry 能读回同一份 ingress 事实
- 已把 capability / skill source snapshot 持久化进 file-backed session store，并在 runtime/control-plane 启动期自动回填 registry：
  - `persisted_mcp_source_snapshots_roundtrip_through_store` 与 `persisted_skill_source_snapshots_roundtrip_through_store` 已证明 source snapshots 可随文件后端 roundtrip
  - `file_backed_reload_restores_persisted_capability_and_skill_source_ingress` 已证明重建 `HostControlPlane` 后，source drilldown 仍能读回 ingress observation、capability 与 skill
- 已补 monitor 页的最小 source drilldown 投影：
  - `ModelMonitorPage` 的 capability source 调试卡现在会展示 `last_ingress_observation` 的 summary / boundary / observedAt / candidate ids
  - `ModelMonitorPage.spec.ts` 已证明前端能消费并展示这条 source ingress 事实，而不会把它误并入 session trace 聚合
- 已补 monitor canonical summary 的反误报回归：
  - `monitor_summary_keeps_source_ingress_out_of_canonical_trace_aggregates` 已证明 source ingress 即使已经进入 source drilldown / runtime registry，也不会平白写进 `load_model_monitor_summary()` 的 capability / skill canonical 聚合
- 已把 capability / skill mediation 的第一段真实 hook dispatch 接入 `execute_registered_tool_call(...)`：
  - capability / skill 路径现在会先执行 capability mediation hooks，再决定最终送入 registry / tool executor 的 arguments
  - 当前已支持 capability mediation 白名单路径 `request.arguments` 的 transform patch 合并与实参改写
  - 若 hook 返回 deny 或 fail-turn 错误，当前会在 tool 执行前阻断并返回显式 blocked result，而不是继续把它伪装成普通 observe trace
- 已把 planner `preflight / tool selection` 的第一段真实 hook dispatch 接入 `plan_turn()` 与 stream planner 路径：
  - planner 路径现在会先执行 preflight / tool selection hooks，再决定最终送入工具执行的 tool call
  - 当前已支持 planner 白名单路径 `provider_tool_call / selected_tool_call` 的 transform patch，并已证明能真实改写执行输入
- 已补 capability / skill mediation 的真实执行验证：
  - `capability_mediation_hooks_can_rewrite_arguments_before_tool_execution` 已证明 capability hook 的 patch 会真正改写工具执行 arguments
  - `skill_mediation_hooks_can_rewrite_arguments_before_skill_execution` 已证明 skill mediation hook 的 patch 会真正改写 composed capability execution arguments
  - `monitor_and_drilldown_read_runtime_generated_skill_hook_evidence` 已证明 `skill.tool_actions.observe` 现在可被 control-plane runtime view 与 session drilldown 读回
- 已补 planner mediation 的真实执行与 read-plane 验证：
  - `planner_preflight_hooks_can_rewrite_tool_call_before_execution` 已证明 preflight hook 的 patch 会真正改写工具执行输入
  - `planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution` 已证明 tool-selection hook 的 patch 会真正改写最终 selected tool call
  - `monitor_and_drilldown_read_runtime_generated_planner_hook_evidence` 已证明 `planner.preflight.observe / planner.tool_selection.observe` 现在可被 control-plane runtime view 与 session drilldown 读回
- 已完成第一次 acceptance 审计，见：
  [2026-06-05-pa040-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa040-acceptance-audit.md)
- 已完成最终 closeout 审计，见：
  [2026-06-05-pa040-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-05-pa040-acceptance-audit.md)
- 当前卡的实现缺口已全部关闭：
  - `planner preflight / tool selection / graph decision`
  - `capability resolve / skill mediation`
  - `source ingress`
  均已接入真实 hooks dispatch，并补齐了对应的 read-plane / control-plane 验证

## 下一步动作
- 本卡已完成 closeout；后续若要扩 monitor 聚合维度或更细的 ingress 投影，应另开新卡承接

## 当前卡点
- 无；本卡已完成

## 断点续跑提示
继续前先看：
- `management/task-system/03_TASKS/PA-019-build-graph-planner-and-decision-policy.md`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
- `management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md`
- `management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md`
- `src-tauri/src/agent/planner.rs`
- `src-tauri/src/agent/capability_bridge.rs`
- `src-tauri/src/agent/control_plane.rs`
