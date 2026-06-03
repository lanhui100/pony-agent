# Pony Agent Task Board

## Roadmap Layers

- 宿主与入口：`PA-015 / PA-023`
- graph 编排：`PA-012 / PA-013 / PA-014 / PA-019`
- runtime 执行：`PA-010 / PA-025`
- 状态、上下文与记忆：`PA-011 / PA-016 / PA-017 / PA-018`
- 能力接入：`PA-020 / PA-021`
- 生命周期横切：`PA-022`
- 观测与监控：`PA-024`

## Backlog

- `PA-022` lifecycle hooks pipeline
  说明：在 graph/runtime/memory/capability 的边界稳定后，补统一 hooks 横切机制。
- `PA-026` workflow mode 与用户自定义流程编排
  说明：在 agent harness 主线完成并稳定后，基于既有 graph / runtime / checkpoint 底座扩展用户自定义 workflow 模式，支持行业流程节点、条件分支、审批、人机协同、重试与审计恢复；该卡明确属于远期扩展，不进入当前近线主线。

## Ready

- 暂无

## In Progress

- `PA-021` skills registry 与 bridge
  说明：已启动 OpenSpec change `add-skills-registry-bridge`，当前聚焦 skill manifest / registry / invocation boundary，以及和 `PA-020` capability bridge、`PA-022` hooks、planner/runtime 的清晰分界。

## Review

- 暂无

## Blocked

- 暂无

## Done

- `PA-020` MCP capability bridge
  说明：已完成 capability registry 统一读面、runtime capability bridge、MCP source snapshot 写面、permission/failure 归一化、capability telemetry summary/drilldown，以及 `tool / resource / prompt_template` 的规范化合同与定向验证。

- `PA-024` 模型监控与 telemetry 聚合面
  说明：已完成 monitor summary / session drill-down 的 Tauri 聚合读面、`ModelMonitorPage` 真实数据页、trace / build-context 下钻展示与前后端定向测试。
- `PA-029` 缓存命中 telemetry 与第一版前缀稳定化
  说明：已完成 call-level cache telemetry、`initial_request / tool_followup` request kind、`PrefixMutationReason` 与第一版 stable prefix 收窄；后端持久化、前端 store 保真与定向测试均已完成，当前可作为 `PA-024` 的底层监控输入。
- `PA-030` trace 面板 call model 可观测性补强
  说明：已完成 `call_model` 的 cache hit / TTFT 展示补齐、工具调用与消息输出保真、多 hop 归因修正，以及完整前端测试与构建验证。
- `PA-028` 历史节点管理、撤销恢复与分支化运行
  说明：已完成 core 历史图、checkout/restore/fork/switch branch、`nodeId` 历史读面与 Tauri 前端历史管理交互，并通过 Rust 与前端定向测试验证。
- `PA-027` OpenSpec 接入任务系统
  说明：已引入 `@fission-ai/openspec`、初始化 `openspec/` 与 Codex workflow skills，并把“复杂开发任务默认先走 OpenSpec”正式写入仓库规范与任务系统规则。
- `PA-025` Build Context 与 cache-friendly prompt 边界
  说明：已完成 `RetrievedContextState -> prompt/request` 三层观测收口；`BuildContextObservation` 现可区分 stable prefix / semi-stable context / volatile input，前后端 trace 展示与回归测试已补齐，并已通过定向 `cargo test`、`session_regression`、前端单测与 `npm run build` 验证。
- `PA-018` 分层 context/state subsystem 与 retrieval boundary
  说明：已完成 retrieval boundary contract、runtime / graph / planner / 宿主默认查询面的 retrieval-first 消费链路、`LongTermMemory` 独立边界与项目级稳定事实来源，并已通过 `cargo test --lib` 与 `npm run verify` 完成态验证。
- `PA-023` 统一 run-stream 正式入口与前端主提交链路
  说明：后端 `start/continue/resume_graph_run_stream`、Tauri run-stream 入口、前端 `submitTurn()` 统一提交流程与 `Trace -> Run -> Turn` 归组链路已经收口。
- `PA-019` graph planner 与计划决策策略
  说明：已落地 `GraphPlanner / GraphPlanningContext / DefaultGraphPlanner`，graph run 可以基于稳定 handoff 输出 `continue / wait_user`。
- `PA-017` 附件生命周期、检索与管理面
  说明：已落地附件生命周期状态、最小检索、显式 cleanup 边界与前端附件中心集成。
- `PA-016` 附件中心索引与资产目录底座
  说明：已落地 `AttachmentAsset / AttachmentReference` 与跨会话附件索引。
- `PA-015` 宿主控制面与统一控制命令
  说明：已落地 `HostControlPlane`、`inspect_host` 与统一控制入口。
- `PA-014` graph stop / resume / checkpoint 与 stop-condition 矩阵
  说明：已落地 `GraphRunStopReason / GraphRunCheckpoint / stop_graph_run / resume_graph_run / load_graph_run_checkpoint`。
- `PA-013` 最小 graph run orchestrator
  说明：已落地 `GraphRunStore / GraphRunner / GraphRunEvent / GraphRunTurnResponse`。
- `PA-012` graph run contract 与 runtime handoff 边界
  说明：已落地 `GraphRun / GraphDecision / GraphTurnHandoff` 第一版 contract。
- `PA-011` 多模态会话记忆与附件生命周期
  说明：已完成附件元数据持久化、recent-image recall、历史附件展示与 review 收口。
- `PA-010` runtime execution control substrate
  说明：已完成 `stop_turn / load_execution_checkpoint / cooperative cancel / turn:cancelled`。

## Dropped

- 暂无
