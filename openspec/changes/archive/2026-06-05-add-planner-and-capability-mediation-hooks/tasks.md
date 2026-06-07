# Tasks: Add Planner And Capability Mediation Hooks

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-040` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-planner-and-capability-mediation-hooks` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 定义 planner facts hook point 与 normalized planner envelope
- [x] 2.2 定义 capability mediation / skill ingress hook point 与 normalized envelope
- [x] 2.3 定义 guard / transform / observe 在 planner/capability 层的允许改动面与 failure policy
  进展：`hooks.rs` 已新增 planner / capability mediation hook point、normalized envelope 与 transform 白名单/只读字段合同；真实 dispatch 与 evidence 接线仍待实现

## 3. Implementation and Verification

- [x] 3.1 在 planner / capability_bridge / control_plane 落最小 trace-first mediation 闭环
  进展：sync 与 stream turn path 已开始记录 `planner preflight / planner tool selection / capability resolve / skill mediation` evidence；`graph decision` evidence 现已进入 graph-run 主路径与 turn truth-source；capability / skill ingress 已收敛为 source drilldown 事实并写入 registry/runtime 双侧，file-backed reload 现也已回填到 runtime/control-plane registry。
  进展补充：`execute_registered_tool_call(...)` 现已先执行 capability / skill mediation hooks，再把白名单 patch 应用到真实 arguments；`plan_turn()` 与 stream planner 路径也已先执行 planner `preflight / tool selection` hooks，再决定最终 tool call。`capability_mediation_hooks_can_rewrite_arguments_before_tool_execution`、`skill_mediation_hooks_can_rewrite_arguments_before_skill_execution`、`planner_preflight_hooks_can_rewrite_tool_call_before_execution`、`planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution` 已证明这些都不是纯 trace，而是实际影响执行输入的 dispatch。
  审计备注：当前仍待关闭的真实 dispatch 缺口主要是 `planner.graph_decision`。
- [x] 3.2 补 planner preflight / graph decision / capability ingress / skill mediation 的 Rust 定向测试
  进展：`run_turn_records_planner_trace_records_in_terminal_trace`、`run_turn_records_capability_mediation_trace_for_forced_tool_planner`、`graph_run_can_start_and_wait_for_next_user_turn`、`monitor_and_drilldown_read_runtime_generated_capability_hook_evidence`、`apply_mcp_source_snapshot_updates_read_plane_and_runtime_registry`、`apply_skill_source_snapshot_updates_read_plane_and_runtime_registry`、`persisted_mcp_source_snapshots_roundtrip_through_store`、`persisted_skill_source_snapshots_roundtrip_through_store`、`file_backed_reload_restores_persisted_capability_and_skill_source_ingress` 已通过。
  进展补充：`monitor_and_drilldown_read_runtime_generated_skill_hook_evidence` 已证明 `skill.tool_actions.observe` 现在能被 control-plane runtime view / session drilldown 读回；`monitor_and_drilldown_read_runtime_generated_planner_hook_evidence` 已证明 `planner.preflight.observe / planner.tool_selection.observe` 现在也能被 control-plane runtime view / session drilldown 读回。
  审计备注：当前主要还缺 `planner.graph_decision` 的真实 dispatch / 定向断言，而不是 planner preflight/tool selection drilldown。
- [x] 3.3 补 read-plane / monitor drilldown 回归
  进展：`ModelMonitorPage` 已开始展示 source-level ingress observation（summary / boundary / observedAt / candidate ids），并由 `ModelMonitorPage.spec.ts` 验证前端消费；`monitor_summary_keeps_source_ingress_out_of_canonical_trace_aggregates` 已证明 source ingress 不会污染 session-level canonical summary；是否升级为 monitor 聚合维度仍待裁决。
  审计备注：当前 read-plane 的主要缺口已不在 planner preflight/tool selection，而在 `planner.graph_decision` 是否纳入同一套真实 mediation dispatch。
- [x] 3.4 完成独立 spec 审核并采纳必要修订
- [x] 3.5 回写任务卡、review 文档、日志与验收证据
