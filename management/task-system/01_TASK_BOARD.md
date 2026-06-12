# Pony Agent Task Board

## Roadmap Layers

- 宿主与入口：`PA-015 / PA-023`
- graph 编排：`PA-012 / PA-013 / PA-014 / PA-019`
- runtime 执行：`PA-010 / PA-025`
- 状态、上下文与记忆：`PA-011 / PA-016 / PA-017 / PA-018`
- 能力接入：`PA-020 / PA-021`
- 生命周期横切：`PA-022`
- 观测与监控：`PA-024`
- core 基础设施边界：`PA-044`

## Backlog

- `PA-022` lifecycle hooks pipeline
  说明：保留为 post-foundation hooks 总入口与分流说明；下一轮已拆成 `PA-038 / PA-039 / PA-040` 三张可执行卡。
- `PA-026` workflow mode 与用户自定义流程编排
  说明：在 agent harness 主线完成并稳定后，基于既有 graph / runtime / checkpoint 底座扩展用户自定义 workflow 模式，支持行业流程节点、条件分支、审批、人机协同、重试与审计恢复；该卡明确属于远期扩展，不进入当前近线主线。

## Ready

- `PA-044` agent core 多端基础设施边界加固
  说明：基于本轮 core 审核新增，目标是把 agent core 明确加固为 Tauri-free、多端可复用的基础设施；Tauri 应作为 first host adapter，而不是 core ownership boundary。OpenSpec change 已建立为 `harden-agent-core-infrastructure-boundary`。

## In Progress

- 暂无

## Review

- 暂无

## Blocked

- 暂无

## Done

- `PA-036` terminal trace envelope 与 monitor 真相源
  说明：已完成 sync failed / streamed cancelled terminal envelope 对齐、reload evidence 保真、monitor canonical truth-source 收紧与前端 raw-trace 防误读约束，并已通过 acceptance audit 与完成态裁定。
- `PA-033` agent hooks pipeline foundation
  说明：已完成 foundation/no-op contract、binding、traceability 与 persisted roundtrip 基础，并已通过独立 acceptance audit；runtime hook dispatch integration 已由 `PA-035` 单独承接。
- `PA-037` session 控制交互面与反馈闭环
  说明：已完成 stop/resume/continue/replay 显式入口、history degrade feedback、统一状态语言与 disabled reason，并已通过 acceptance audit 与前端完成态验证。
- `PA-035` runtime hook dispatch stable-boundary integration
  说明：已完成 stable-boundary runtime hook dispatch、runtime-produced hook trace realtime/persisted/read-plane 闭环，以及 ordering/failure/reload/front-end 验收，并已通过 acceptance audit 与 closeout。
- `PA-034` checkpoint lifecycle boundary implementation
  说明：已完成 runtime checkpoint boundary、persisted evidence、reload/control-plane 投影与前端 runtime store 消费闭环，并已通过 acceptance audit 与完成态验证。
- `PA-032` trace persistence 与 recovery contract
  说明：已完成 recovery contract、submission plan 仲裁、reload/hydration 收口、history degrade 合同与后端/前端恢复仲裁闭环，并已通过 acceptance audit 与完成态裁定。
- `PA-031` turn lifecycle 与 event contract
  说明：已完成 canonical lifecycle/event vocabulary、SSE/event envelope、multi-hop/failed/cancelled 终态语义与前端 canonical 消费收口，并已通过 acceptance audit 与完成态裁定。
- `PA-021` skills registry 与 bridge
  说明：已完成 skill source snapshot ingress、统一 registry、`list_skills / inspect_skill`、tool-only runtime execution、planner normalized skill facts consumption 与 monitor skill lineage 聚合/下钻展示，并已通过 acceptance audit 与完成态 closeout。
- `PA-020` MCP capability bridge
  说明：已完成 capability registry 统一读面、runtime capability bridge、MCP source snapshot 写面、permission/failure 归一化、capability telemetry summary/drilldown，以及 `tool / resource / prompt_template` 的规范化合同与定向验证。
- `PA-040` planner 与 capability-mediation hooks
  说明：已完成 planner `preflight / tool selection / graph decision`、capability `resolve / skill mediation` 与 source ingress 的真实 hook dispatch、白名单 transform、session snapshot / control-plane drilldown 读面闭环，并已通过 acceptance audit 与 closeout。
- `PA-041` history-state hooks 与 restore-boundary contract
  说明：已完成 `history checkout / branch restore / branch fork / branch switch` 四类 boundary 的真实 hook dispatch、persisted audit chain、reload/control-plane/runtime-view/frontend contract 对齐，以及 degrade/non-regression 验收，并已通过 acceptance audit。
- `PA-042` session control audit surface 与 history evidence summary
  说明：已完成 `Session Control Plane audit surface v1`、history-control summary read-model、snapshot/runtime-view/response 统一投影、frontend summary-first explainability 与 acceptance audit/closeout；后续 run-control summary 扩展应由新卡承接。
- `PA-043` run-control audit surface 与 summary-first explainability
  说明：已完成 `Run Control audit surface v1`、`stop / continue / resume / replay(start)` summary contract、snapshot/runtime-view/response 统一投影、frontend summary-first explainability 与 acceptance audit/closeout；普通首轮 `start_graph_run_stream` 排除、truth-source guardrail 与 reload/hydration 回归已收口。
- `PA-039` memory-write hooks 与 persisted side-effect contract
  说明：已完成 `long-term memory write` 的 `intent -> hook -> evidence -> recovery` 真闭环，`memory_write_evidence` 与 `memory_write_hook_trace_records` 已通过 snapshot / history checkout / file roundtrip / checkpoint recovery 验证，并已通过 acceptance audit 与 closeout。
- `PA-038` run hooks 与 execution-control boundary
  说明：已完成 `submission_plan / wait_user / stop_requested / run_resume` 的 canonical boundary persisted evidence、runtime view/session control 统一读面与前端回归，并已通过 acceptance audit 与 closeout。

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
