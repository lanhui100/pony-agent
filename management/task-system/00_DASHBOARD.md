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
6. `PA-021` 已进入 spec-first 启动阶段
   已建立 `add-skills-registry-bridge` change，当前聚焦 skills 作为 capability-composition layer 的 registry / invocation / observability 边界，不吸收 hooks、workflow 或 planner redesign 范围。
7. `PA-030` 已完成并通过前端验收
   已建立并完成 `add-trace-panel-call-model-observability` change，trace 面板 `call_model` 现已补齐 cache hit / TTFT、工具调用与消息输出保真，以及多 hop 输出归因修正。
7. 复杂开发任务治理已升级为 OpenSpec + 任务系统双轨
   `openspec/` 承载 proposal/spec/design/tasks，`management/task-system/` 承载状态、审计、日志与断点续跑。

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

1. 回到 `PA-021`，继续推进 skills registry / bridge 的最小实现与验证。
2. 若继续做缓存优化或 trace 展示深化，应基于 `PA-029`、`PA-024` 与 `PA-030` 的现有观测口径再拆增量卡，而不是回滚当前边界。
3. 保持当前阶段不提前实现完整 auto-compaction；该工作保留到 Phase 4/5 的后续卡。

## 关联入口

- 任务板：[01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- OpenSpec 目录：[openspec](/C:/Users/HUAWEI/Documents/pony-agent/openspec)
- 已完成任务卡：[PA-018](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md>)
- 已完成任务卡：[PA-027](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-027-integrate-openspec-into-task-system.md>)
- 已完成任务卡：[PA-028](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md>)
- 正式验收审计：[PA-018 Acceptance Audit](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md>)
- 文档索引：[docs/INDEX.md](/C:/Users/HUAWEI/Documents/pony-agent/docs/INDEX.md)
- 会话日志目录：[99_LOGS](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/99_LOGS)
