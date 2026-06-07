# PA-021 Acceptance Audit

## 审核范围

- [PA-021 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-021-build-skills-registry-and-bridge.md)
- [skills-registry-bridge/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/specs/skills-registry-bridge/spec.md)
- [skills-registry-bridge/tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-skills-registry-bridge/tasks.md)
- [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/capability_bridge.rs)
- [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- [planner.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/planner.rs)
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
- [ModelMonitorPage.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/ModelMonitorPage.vue)

## 验收结论

`PA-021` 的 `v1` 范围已完成，可以从 `Review` 关闭到 `Done`。

本卡已经证明：

- skills 作为 capability-composition layer 接入 unified registry，而不是第二执行通道
- planner 读取的是 normalized skill facts，而不是 raw manifest / host 私有实现
- runtime `v1` 只执行 tool-composed skills，并对非 tool 组合显式 unsupported
- skill lineage 已进入现有 monitor summary / drilldown 读面，而不是新 telemetry 栈

## Requirement-by-requirement 审核

1. skills 通过 unified capability-registry boundary 进入系统
   - 证据：
     [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/capability_bridge.rs)
     中的 `SkillSourceSnapshot / replace_skill_source_snapshot / normalize_skill_source_snapshot`
   - 结论：通过

2. composed capability kinds 语义被保留，且 `v1` 只执行 tool-composed skills
   - 证据：
     [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/capability_bridge.rs)
     中的 `composed_capability_kinds / executable_in_v1 / resolve_skill_tool_actions`
   - 结论：通过

3. planner 只消费 normalized skill facts
   - 证据：
     [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
     中 `build_request(..., planner_skills)` 与 `render_planner_skills_note(...)`
   - 结论：通过

4. host/control-plane 保持 host-agnostic read/write surface
   - 证据：
     [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
     中 `apply_skill_source_snapshot / list_skills / inspect_skill`
   - 结论：通过

5. failure / permission / host-mediation 沿用统一语义
   - 证据：
     [capability_bridge.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/capability_bridge.rs)
     中 `SkillFailureLayer`、权限聚合与 failure mapping
   - 结论：通过

6. skill usage 进入现有 monitor summary/drilldown 链路
   - 证据：
     [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
     中 `append_skill_aggregates(...)`
     与 [ModelMonitorPage.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/ModelMonitorPage.vue)
     中的 skill summary / drilldown 展示
   - 结论：通过

## 定向验证

Rust 定向测试通过：

- `cargo test planner_can_select_explicit_skill_by_normalized_label --lib`
- `cargo test build_request_includes_normalized_planner_skill_summary --lib`
- `cargo test runtime_executes_registered_skill_by_tool_name --lib`
- `cargo test load_model_monitor_summary_aggregates_capability_usage_dimensions --lib`
- `cargo test capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state --lib`

前端定向测试通过：

- `npm run test:unit -- --run tests/ModelMonitorPage.spec.ts`
- `npm run test:unit -- --run tests/runtime-store.spec.ts`

前端构建通过：

- `npm run build`

## 残余风险

- `PA-021` 的 `v1` 已完成，但更强的 skill selection policy、mixed composition execution、宿主刷新入口扩展应另拆后续任务，不应继续挤在本卡里。

## 最终裁定

`PA-021` 已满足任务卡定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. skills 已作为 capability-composition layer 接入 unified registry，而不是形成第二 scheduler。
2. planner / runtime / control-plane / monitor 已具备真实、闭环且可审计的消费链路。
3. OpenSpec `tasks.md` 已全部勾选完成，本卡 `v1` 范围不存在未交付项。
4. 定向 Rust 测试、前端单测与前端构建均已通过，足以支撑完成态裁定。
5. 更高阶扩展边界已明确外拆，不再阻塞本卡关闭。
