# 2026-06-05 Session 74 - PA-040 Contract Bootstrap

## 主题

- 启动 `PA-040` 的第一段 contract/scaffolding 实现
- 先把 planner / capability mediation hooks 的规范化 envelope 与 transform 白名单落到 `hooks.rs`

## 本轮结论

1. `PA-040` 已不再停留在只读勘探
   - `hooks.rs` 已开始定义 `PlannerHookPoint / CapabilityMediationHookPoint`
   - `hooks.rs` 已开始定义 `PlannerFactsEnvelope / CapabilityMediationEnvelope`
   - planner / capability mediation 各自的 transform 白名单与只读字段合同已开始落地
   - sync `run_turn(...)` 路径已开始记录 `planner preflight / planner tool selection / capability resolve / skill mediation` evidence

2. 这一刀刻意保持收敛
   - 本轮不接 runtime 真线
   - 本轮不让 planner hooks 或 capability hooks 越过既有 truth-source
   - 本轮只先把“允许改哪里 / 不允许改哪里”固定成可测试合同

3. `PA-040` 的任务系统状态已同步前进
   - `PA-040` 已从 `Ready` 更新为 `In Progress`
   - OpenSpec tasks 中 contract definition 三项已同步标记为开始落地

4. stream path 已补上最小 trace-first evidence 闭环
   - `start_turn_stream_with_control(...)` 现在会生成并传递 `planner preflight / planner tool selection` traces
   - `handle_stream_tool_turn(...)` 现在会把 planner / capability / tool boundary evidence 一并归入 terminal hook trace
   - stream no-tool 完成态现在也会保留 planner evidence，不再只剩 checkpoint/finalize 尾部记录
   - 为了让 forced planner 测试能稳定绕过 provider native tool flow，`ForcedToolPlanner` 预检产物已补最小 `ToolPlan`

5. graph decision evidence 已接回 graph-run 真相源
   - `SessionStore` / `AgentRuntime` 已新增“向既有 turn trace 追加 hook records”的能力
   - `control_plane` 在 graph planner 形成最终 `GraphDecision` 后，现会写入 `planner.graph_decision.observe`
   - 这条 evidence 已同时进入：
     - persisted `turn trace`
     - `GraphRunTurnResponse.turn_result.hook_trace_records`
     - 后续的 session snapshot / control-plane 读面

6. 第二轮独立 spec 收紧审阅已完成并采纳
   - 当前不新增第四张 hooks 近线大卡
   - 已把 `PA-038 / PA-039 / PA-040` 的验收口径进一步收紧到当前真实实现边界

7. capability ingress 已收敛成 source drilldown 事实
   - 本轮明确不把 capability ingress 强塞进 `TurnHookPoint / HookTraceRecord`
   - `CapabilitySourceView / SkillSourceView` 已新增 `last_ingress_observation`
   - `replace_mcp_source_snapshot(...)` 与 `replace_skill_source_snapshot(...)` 现在会自动生成 ingress observation，并同步写入 control-plane registry 与 runtime registry
   - 两条定向测试已通过，证明 source drilldown 与 runtime registry 都能读回同一份 ingress 事实：
     - `apply_mcp_source_snapshot_updates_read_plane_and_runtime_registry`
     - `apply_skill_source_snapshot_updates_read_plane_and_runtime_registry`

8. capability ingress 的 reload 持久化链已经打通
   - `SessionStore` 现已开始持久化 mcp / skill source snapshots
   - `AgentRuntime` 启动时会从 file-backed store 自动回填 capability registry
   - `HostControlPlane` 初始化时会从 runtime 克隆当前 registry，避免 reload 后 control-plane 与 runtime source read-plane 脱节
   - 三条定向测试已通过：
     - `persisted_mcp_source_snapshots_roundtrip_through_store`
     - `persisted_skill_source_snapshots_roundtrip_through_store`
     - `file_backed_reload_restores_persisted_capability_and_skill_source_ingress`

9. monitor/source drilldown 已补最小前端投影
   - 本轮没有把 source ingress 强行并入 session 级 monitor 聚合
   - 改为在 `ModelMonitorPage` 的 capability source 调试卡中直接展示 `last_ingress_observation`
   - 当前可见字段包括：summary、boundary、observedAt、candidate ids
   - `tests/ModelMonitorPage.spec.ts` 已通过，证明前端能消费并展示这条 source ingress 事实

10. monitor canonical summary 的“反误报边界”已补测试
   - 新增 `monitor_summary_keeps_source_ingress_out_of_canonical_trace_aggregates`
   - 该测试证明：source ingress 即使已经持久化并能从 source inspect 读回，也不会因为存在而平白出现在 `load_model_monitor_summary()` 的 capability / skill 聚合中
   - 当前 monitor 的 source ingress 仍明确属于 source inspect 调试读面，而不是 session trace canonical summary

## 本轮回写

- [PA-040 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md)
- [Task Board](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [OpenSpec tasks](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-planner-and-capability-mediation-hooks/tasks.md)

## 下一步动作

- 把 planner preflight / tool selection / graph decision 接到真实 trace-first hook evidence
- 把 capability resolve / skill mediation 接到真实 trace-first hook evidence
- 优先补 `graph decision` evidence，再决定 monitor / control-plane drilldown 的最小切口
- 现在 `graph decision` 已进入 turn truth-source，下一步改为 capability ingress / control-plane drilldown / reload 验证
- 现在 capability ingress 的 control-plane/runtime drilldown 已落地，下一步改为 reload 持久化验证与 monitor 投影取舍
- 现在 capability ingress 的 reload 持久化验证也已落地，下一步收敛到 monitor 投影取舍与更完整的 read-plane 回归矩阵

## 当前验证情况

- `rustfmt src-tauri/src/agent/hooks.rs`：通过
- `rustfmt src-tauri/src/agent/runtime.rs src-tauri/src/agent/turn_flow.rs`：通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::hooks::tests::planner_transform_whitelist_keeps_scheduler_fields_readonly --target-dir src-tauri/target-codex-pa040-contract -- --exact --nocapture`
  - 本轮在本机环境下 5 分钟内未完成，因超时被截断
  - 当前已形成代码与文档层面的 contract 证据，但 Rust 编译级验证仍需下一轮继续补齐
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_records_planner_trace_records_in_terminal_trace --target-dir src-tauri/target-codex-pa040-evidence -- --exact --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::runtime::tests::run_turn_records_capability_mediation_trace_for_forced_tool_planner --target-dir src-tauri/target-codex-pa040-evidence -- --exact --nocapture`
  - 按 `--exact` 跑时出现 0 tests filtered，后续已改为按子串执行
  - 两条定向测试现已在本机通过：
    - `run_turn_records_planner_trace_records_in_terminal_trace`
    - `run_turn_records_capability_mediation_trace_for_forced_tool_planner`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::session::tests::append_turn_trace_hook_records_updates_existing_trace_and_roundtrips -- --exact --nocapture`
  - 通过
  - 证明 graph decision 追加式 hook evidence 可随 turn trace roundtrip 持久化
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::graph_run_can_start_and_wait_for_next_user_turn -- --exact --nocapture`
  - 通过
  - 证明 graph-run 主路径会把 `planner.graph_decision.observe` 同时写回 session truth-source 与 immediate turn result
- `cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::monitor_and_drilldown_read_runtime_generated_capability_hook_evidence -- --exact --nocapture`
  - 通过
  - 证明 runtime 产生的 `capability.resolve.observe` 已可被 session runtime view、monitor session drilldown 与 monitor summary 同时读回
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
  - 通过
  - 仍有既有 warning，以及 Windows 增量编译目录 `拒绝访问 (os error 5)` 的环境警告，但不阻塞本轮实现验证
- `cargo test --manifest-path src-tauri/Cargo.toml --lib apply_mcp_source_snapshot_updates_read_plane_and_runtime_registry -- --nocapture`
  - 通过
  - 证明 mcp source ingress observation 可被 control-plane source drilldown 与 runtime registry 同步读回
- `cargo test --manifest-path src-tauri/Cargo.toml --lib apply_skill_source_snapshot_updates_read_plane_and_runtime_registry -- --nocapture`
  - 通过
  - 证明 skill source ingress observation 可被 control-plane source drilldown 与 runtime registry 同步读回
- `cargo test --manifest-path src-tauri/Cargo.toml --lib persisted_mcp_source_snapshots_roundtrip_through_store -- --nocapture`
  - 通过
  - 证明 mcp source snapshot 与 ingress observation 可经 file-backed session store roundtrip 读回
- `cargo test --manifest-path src-tauri/Cargo.toml --lib persisted_skill_source_snapshots_roundtrip_through_store -- --nocapture`
  - 通过
  - 证明 skill source snapshot 与 ingress observation 可经 file-backed session store roundtrip 读回
- `cargo test --manifest-path src-tauri/Cargo.toml --lib file_backed_reload_restores_persisted_capability_and_skill_source_ingress -- --nocapture`
  - 通过
  - 证明重建 runtime / control-plane 后，source drilldown 仍能读回持久化的 capability / skill ingress 事实与关联对象
- `npm run test:unit -- --run tests/ModelMonitorPage.spec.ts`
  - 通过
  - 证明 monitor 页已能展示 capability source 的 ingress detail，而不影响现有 summary / session drilldown 交互
- `cargo test --manifest-path src-tauri/Cargo.toml --lib monitor_summary_keeps_source_ingress_out_of_canonical_trace_aggregates -- --nocapture`
  - 通过
  - 证明 source ingress 不会污染 canonical monitor summary 聚合，但仍保留在 source inspect 读面
