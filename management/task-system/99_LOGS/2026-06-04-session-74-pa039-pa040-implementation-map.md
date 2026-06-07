# 2026-06-04 Session 74 - PA-039 / PA-040 Implementation Map

## 主题

- 基于已完成的 spec review，为 `PA-039` 与 `PA-040` 补一轮只读实现入口勘探
- 把“下一张卡怎么开工”从抽象 spec 推进到精确模块/函数级 implementation map

## 本轮结论

1. `PA-039` 的首个真实 truth-source 已收敛
   - memory-write hooks 不应先挂在 planner、graph arbitration 或 capability bridge
   - 首个真实接线点优先落在 [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs) 的 `update_long_term_memory_from_user_message(...)`
   - persisted evidence / reload / recovery 应沿：
     - `SessionSnapshot.long_term_memory_entries`
     - `FileSessionBackend` roundtrip
     - `HostControlPlane::load_execution_checkpoint(...)` / recovery read-plane
     这条链路收口
   - 合同层优先补：
     - `MemoryWriteIntent`
     - `MemoryWriteHookPoint`
     - `PersistedEffectEvidence`

2. `PA-040` 的 planner / capability mediation 挂点已收敛
   - planner hooks 首选挂点：
     - `Runtime::decide_graph_after_turn_with_planner(...)`
     - `LocalTurnPlanner::preflight_decision(...)`
     - `LocalTurnPlanner::select_tool_call(...)`
   - capability mediation hooks 首选挂点：
     - `CapabilityRegistry::resolve_tool_call(...)`
     - `CapabilityRegistry::resolve_skill_tool_actions(...)`
   - ingress / read-plane 辅助挂点：
     - `HostControlPlane::apply_mcp_source_snapshot(...)`
     - `HostControlPlane::apply_skill_source_snapshot(...)`
   - 合同层优先补：
     - `PlannerFactsEnvelope`
     - `CapabilityMediationHookEnvelope`
     - planner/capability 分离的 transform 白名单

3. 三张卡的边界进一步收紧
   - `PA-038` 管 run / execution control：怎么跑、怎么停、怎么续
   - `PA-039` 管 memory write / persisted effect / replay_required：写了什么、持久化了什么、如何恢复
   - `PA-040` 管 planner / capability mediation：如何形成高层决策、如何走 capability/skill 中介
   - 本轮再次确认：
     - `PA-039` 不得改 `resolve_graph_run_submission_plan(...)` 的仲裁职责
     - `PA-040` 不得新增 `RunHookPoint` 或替代 run-level truth-source
     - `PA-040` 最小闭环阶段不定义 `persisted_effect / replay_required`

## 任务系统回写

- 已更新 [PA-039](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-039-build-memory-write-hooks-and-persisted-side-effect-contract.md)
- 已更新 [PA-040](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md)
- 已更新 [Task Board](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- 已更新 [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)

## 下一步建议

1. 先把 `PA-038` 的 `stop_requested / run_resume` 命令边界显式化，继续收口 run-level 审计链。
2. 随后启动 `PA-039`，先做 `hooks.rs` 合同层，再接 `session.rs` 的 memory truth-source。
3. `PA-040` 保持 `Ready`，待 `PA-038` 再稳定一段后，可与 `PA-039` 串并行推进。
