# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式重构项目
- 当前主线：`Phase 4 / Graph Runtime Mainline`
- 当前阶段：`Mainline Stabilizing`
- 总体状态：`In Progress`

## 当前主线结论

- `PA-018` 已完成并通过完成态验证
- retrieval 观测与 trace 展示语义已正式拆到 `PA-024`
- `PA-025` 已完成 `Build Context` 与 cache-friendly prompt 边界收口
- `PA-027` 已把 OpenSpec 接入仓库与任务系统，复杂任务现在有正式 spec-first 流程
- `PA-028` 已完成历史节点管理实现、前端交互与定向验证
- `PA-020 / PA-021` 现在不再被 `PA-018` 阻塞
- 当前已正式启动 `PA-031 / PA-032 / PA-033` 三卡串联，用于收口 turn lifecycle、trace persistence/recovery 与 hooks foundation
- `PA-034` 已补入近线主视图，负责把 `checkpointing` 从 contract 名词推进成 runtime / persistence / reload / frontend 共同可见的真实 lifecycle boundary

## 当前重点

1. `PA-018` 已关闭
   已落地 `RetrievedContextState / ContextStateRetriever` contract、runtime / graph / planner / 宿主默认查询面的 retrieval-first 消费链路、`LongTermMemory` 独立边界与项目级稳定事实来源。
2. `PA-029` 已完成并通过定向验证
   `PA-025` 之后缺失的 call-level cache telemetry、首请求 / follow-up 分离、prefix mutation reasons 与第一版 stable prefix 收窄已经落地，缓存命中现在具备可解释的工程口径。
3. `PA-024` 已完成监控读面承接
   `PA-029` 交付的 `provider_call_records`、`requestKind`、`prefixMutationReasons` 与三层 build-context observation 已有正式 monitor read-plane 消费入口。
4. `PA-028` 已完成
   core 历史图、分支游标、历史 checkout/fork/restore/switch branch 与前端历史交互均已落地，且已通过 Rust 与前端定向测试。
5. `PA-020` 已完成并通过定向验证
   capability registry 统一读面、runtime bridge、MCP source snapshot 写面、permission/failure 归一化、monitor capability summary/drilldown 与 `tool / resource / prompt_template` 规范化合同均已落地。
6. `PA-021` 已关闭
   已落地 skill source snapshot ingress、统一 registry、`list_skills / inspect_skill`、tool-only runtime resolution、planner normalized skill facts consumption，以及 monitor summary/drilldown skill lineage 聚合与展示，并已完成 acceptance audit 与 closeout。
7. `PA-030` 已完成并通过前端验收
   已建立并完成 `add-trace-panel-call-model-observability` change，trace 面板 `call_model` 现已补齐 cache hit / TTFT、工具调用与消息输出保真，以及多 hop 输出归因修正。
7. 复杂开发任务治理已升级为 OpenSpec + 任务系统双轨
   `openspec/` 承载 proposal/spec/design/tasks，`management/task-system/` 承载状态、审计、日志与断点续跑。
8. turn lifecycle / recovery / hooks 新主线已完成第一批近线收口
   `PA-031 / PA-032 / PA-033 / PA-034 / PA-035 / PA-036 / PA-037` 已全部完成并关闭，分别收口了 canonical lifecycle/event contract、trace persistence/recovery contract、hooks foundation、checkpoint lifecycle boundary、stable-boundary runtime hook dispatch、terminal truth-source 与 session control UX 闭环。
9. 当前近线主线已从“补 contract / 补闭环”切到“归档首批 / 定下一轮”
   当前 review 队列已清空；`PA-021 / PA-030 / PA-031 / PA-032 / PA-033 / PA-034 / PA-035 / PA-036 / PA-037` 对应的 OpenSpec changes 已同步进 `openspec/specs/` 并归档到 `openspec/changes/archive/`，下一步更适合决定是否把 `PA-022`/后续扩展正式提到近线。
10. `PA-022` 已不再保留为模糊大卡
   当前已把 post-foundation hooks 范围拆成 `PA-038 / PA-039 / PA-040` 三张近线卡，而且三张卡都已完成 closeout：`PA-038` 收口了 run/execution-control boundary evidence，`PA-039` 收口了 memory-write persisted side-effect contract，`PA-040` 收口了 planner/capability mediation hooks。
11. hooks post-foundation 近线三卡已完成第一轮工业化闭环
   当前 `run hooks`、`memory-write hooks`、`planner/capability hooks` 都已经具备各自的稳定边界、persisted/read-plane 证据链与 acceptance audit，后续扩展更适合以新卡承接，而不是回灌到同一批 closeout 卡。
12. `PA-038 / PA-039 / PA-040` 对应的 OpenSpec changes 已归档
   三张 change 的 delta specs 已同步到 `openspec/specs/`，并已迁入 `openspec/changes/archive/2026-06-05-*`；当前活跃 `openspec/changes/` 已重新清空，只保留 `archive/`。
13. hooks post-foundation 近线第四卡也已完成
   `PA-041` 已完成 `history checkout / branch restore / branch fork / branch switch` 四类 history-state control boundary 的 hook dispatch、persisted audit chain、reload/read-plane/front-end contract 对齐，以及 degrade truth-source non-regression。
14. `PA-042` 已完成并通过验收审计
   `Session Control Plane audit surface v1` 已完成 history-control summary contract、snapshot/runtime-view/response 统一投影、reload roundtrip、truth-source guardrail 与前端 summary-first explainability，当前这轮近线主线已完成收口。
15. `PA-043` 已完成并通过验收审计
   `Run Control audit surface v1` 已完成 `stop / continue / resume / replay(start)` summary contract、snapshot/runtime-view/response 统一投影、普通首轮 `start_graph_run_stream` 排除、reload/hydration guardrail 与前端 summary-first explainability，当前近线主线已进一步完成 run-control 收口。

## 远期扩展

- 在当前 agent harness 主线完成并稳定后，路线图将扩展一条 `workflow mode` 支线。
- 该支线目标不是替代 agentic 模式，而是在既有 graph 底座上补用户可定义流程，服务行业 SOP、审批流、分工协作与可审计工作场景。
- 预期复用的底座包括：`graph run`、checkpoint / resume、trace、tool capability registry 与 human-in-the-loop 边界。
- 这一方向当前不进入近线主线范围，待 harness 收口后再正式拆卡。

## 当前代码证据

- retrieval contract 与默认实现：
  [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- long-term memory 独立边界与稳定事实来源：
  [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- runtime 接入：
  [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- graph handoff 与 planner 收口：
  [graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs)
  [planner.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/planner.rs)
- 宿主 retrieval-first 读面：
  [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
  [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
- 架构边界文档：
  [context-state-subsystem.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)

## 当前验证

`PA-029` 本轮完成态验证：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml file_backend_roundtrip_restores_turn_trace_history
cargo test --manifest-path src-tauri/Cargo.toml run_turn_accumulates_token_usage_across_tool_followups
cargo test --manifest-path src-tauri/Cargo.toml start_turn_stream_accumulates_token_usage_across_tool_followups
cargo test --manifest-path src-tauri/Cargo.toml build_request_keeps_image_and_truncation_notes_out_of_stable_prefix
cargo test --manifest-path src-tauri/Cargo.toml build_context_observation_fallback_keeps_dynamic_system_and_developer_text_out_of_stable_prefix
npm run test:unit -- --run tests/runtime-store.spec.ts
npm run test:unit -- --run tests/HomeSidebar.spec.ts
```

结果：

- Rust / session trace roundtrip 定向测试通过
- Rust / runtime sync 与 stream cache telemetry 定向测试通过
- Rust / context stable-prefix fallback 定向测试通过
- 前端 `runtime-store` 定向测试通过：`39 passed`
- 前端 `HomeSidebar` 定向测试通过：`7 passed`

## 下一步最小动作

1. 当前 `PA-042 / PA-043` 已把 history-control 与 run-control 的 summary family 都收口到 `Session Control Plane`，下一步更适合评估是否要继续补细 monitor/drilldown，而不是回灌已关闭任务。
2. 保持“spec 审核 -> 实现 -> acceptance -> 归档”的整批闭环节奏，把后续 control-plane 扩展继续压在 stable boundary 上。
3. 若后续继续扩展 replay/control family，应新开任务承接，不在 `PA-043` 上继续叠加 scope。

## 新近线候选

1. 基于 `Session Control Plane` 的 monitor / drilldown 读面扩展
   在 `PA-042 / PA-043` 已完成 summary family 收口的前提下，评估是否需要新增更细的 run-control / history-control 审计下钻，而不是回灌已关闭任务。

## 关联入口

- 任务板：[01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- OpenSpec 目录：[openspec](/C:/Users/HUAWEI/Documents/pony-agent/openspec)
- 已完成任务卡：[PA-018](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md>)
- 已完成任务卡：[PA-027](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-027-integrate-openspec-into-task-system.md>)
- 已完成任务卡：[PA-028](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md>)
- 正式验收审计：[PA-018 Acceptance Audit](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md>)
- 文档索引：[docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- 会话日志目录：[99_LOGS](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS)
