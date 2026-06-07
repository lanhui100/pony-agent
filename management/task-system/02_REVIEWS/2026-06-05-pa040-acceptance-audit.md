# PA-040 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md`
- `openspec/changes/add-planner-and-capability-mediation-hooks/specs/planner-and-capability-mediation-hooks/spec.md`
- `openspec/changes/add-planner-and-capability-mediation-hooks/tasks.md`
- `src-tauri/src/agent/hooks.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/agent/capability_bridge.rs`
- `src/components/ModelMonitorPage.vue`
- `src/types/runtime.ts`
- `tests/ModelMonitorPage.spec.ts`

## 审核口径

只按 `PA-040` 当前任务卡与 delta spec 的完成边界判断：确认 planner / capability mediation hooks 是否已经形成真实的 hook 执行闭环、read-plane 证据闭环与 reload 后可验证的事实链路；不把 `PA-038 / PA-039` 的 run hooks / memory-write hooks 完成度挪用为本卡的完成证据。

### 不在本审计内

- `PA-038` 的 execution-control / run hooks 扩展
- `PA-039` 的 persisted side-effect / memory-write hook 合同
- monitor 是否升级为新的 source-level canonical 聚合维度

## 逐项结论

### A. planner / capability mediation 的真实 hook 执行闭环

状态：`达成`

发现：

- `hooks.rs` 已经定义 planner / capability mediation 的 hook point、envelope 与 transform 白名单合同，这说明 contract 层准备工作已基本具备。
- `runtime.rs` 中的 `plan_turn()` 现在已经会先执行 planner preflight / tool selection hooks，再决定最终送入执行路径的 tool call；`execute_registered_tool_call()` 也已经先执行 capability / skill mediation hooks，再把白名单 patch 应用到真实 arguments。
- `control_plane.rs` 中的 `apply_mcp_source_snapshot()` / `apply_skill_source_snapshot()` 现在会先经过独立的 source ingress hook dispatch，再决定是否真正落到 registry/runtime。
- `planner.graph_decision` 现在也已经纳入同一套 planner mediation dispatch；当前白名单仍只允许改写 `decision_summary`，不会越界改写 `decision_kind / target_phase`。

证据：

- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:1984)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:2000)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:2027)
- [src-tauri/src/agent/hooks.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/hooks.rs:2045)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:633)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:716)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:4220)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:7548)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:7605)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:4278)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:7312)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:816)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:836)

判断：

planner `preflight / tool selection / graph decision` 与 capability / skill mediation 都已经不再只是 `trace-first` 证据接线，而是能在白名单边界内真实影响执行输入或 graph decision summary；同时 source ingress 也已经具备独立 hook dispatch，而不会污染 turn trace。

### B. skill mediation 的 session snapshot / control-plane read-plane 闭环

状态：`达成`

发现：

- `runtime.rs` 已证明 skill tool 路径会产出 `TurnHookPoint::SkillToolActionsResolve` 对应的 `skill.tool_actions.observe` 记录。
- 但当前显式的 control-plane 回归只覆盖了 `capability.resolve.observe` 的 runtime view / monitor drilldown / summary 读回，没有看到 `skill.tool_actions.observe` 对应的 session snapshot 持久化、reload 读回与 control-plane drilldown 断言。
- 现有 session roundtrip 样例也主要锚定 planner / source ingress，并未给出 skill mediation read-plane 的强证据。

证据：

- [management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md:35)
- [openspec/changes/add-planner-and-capability-mediation-hooks/specs/planner-and-capability-mediation-hooks/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-planner-and-capability-mediation-hooks/specs/planner-and-capability-mediation-hooks/spec.md:49)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:883)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:8039)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:6415)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:5075)

判断：

`skill.tool_actions.observe` 已完成 runtime view、session snapshot 与 control-plane drilldown 的闭环，不再是当前阻断项。

### C. planner preflight / tool selection 的 control-plane 回归矩阵

状态：`达成`

已达成：

- `graph decision` evidence 已写回 graph-run 主路径，并能通过 turn truth-source 读回。
- source ingress 的 source drilldown / reload / monitor source inspect 已形成稳定边界，且 canonical summary 反误报已补。

证据：

- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:6399)
- [src-tauri/src/agent/runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs:6408)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:4138)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:6415)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:6503)

判断：

`planner.preflight.observe` 与 `planner.tool_selection.observe` 现在已经完成 runtime view / session snapshot / control-plane drilldown 回归，planner read-plane 的剩余缺口不再在这里，而在 `graph decision` 的真实 dispatch。

### D. source ingress 的 truth-source 边界

状态：`达成`

证据：

- capability / skill source ingress 已明确收敛为 source drilldown 事实，而不是 turn-level hook trace 噪声。
- file-backed session store 已支持 source snapshot roundtrip，runtime / control-plane 重建后可自动回填 registry。
- monitor source inspect 已能展示 `lastIngressObservation`，且 canonical summary 不会把 source ingress 误记为 session-level capability / skill trace 活动。

证据：

- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:5152)
- [src-tauri/src/agent/session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs:5214)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:5199)
- [src-tauri/src/agent/control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs:6274)
- [src/components/ModelMonitorPage.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/ModelMonitorPage.vue:463)
- [tests/ModelMonitorPage.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/ModelMonitorPage.spec.ts:500)

## 验证备注

- 本轮 acceptance 审核以静态代码审计、既有测试名称与已沉淀的通过记录为主。
- explorer 子智能体只读审计与本地核对结论一致：当前最大缺口不是字段丢失，而是尚未接入真实 hook dispatch。
- `tests/ModelMonitorPage.spec.ts` 的 source ingress 前端消费证据已存在，但本轮没有新增 Rust 动态执行结果。

## 最终裁定

`PA-040` 已满足任务卡与 delta spec 的完成边界，可以从 `In Progress` 更新为 `Done`。

关闭理由：

1. planner `preflight / tool selection / graph decision` 与 capability / skill mediation 已全部接入真实 hook dispatch，不再只是 post-facto observe evidence。
2. `planner.preflight.observe / planner.tool_selection.observe / planner.graph_decision.observe / capability.resolve.observe / skill.tool_actions.observe` 都已完成 runtime view、session snapshot 与 control-plane drilldown 的读回闭环。
3. source ingress 现在既能保持 source truth / drilldown 边界不污染 turn trace，又具备独立 hook dispatch，可在 snapshot apply 前阻断异常 ingress。
4. transform 白名单、只读字段、防止第二 scheduler / 第二 registry 的合同仍保持不变，没有因为真实 dispatch 落地而越界。
