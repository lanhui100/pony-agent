# Pony Agent Task Board

## Roadmap Layers

- 宿主与入口：`PA-015 / PA-023`
- graph 编排：`PA-012 / PA-013 / PA-014 / PA-019`
- runtime 执行：`PA-010`
- 状态、上下文与记忆：`PA-011 / PA-016 / PA-017 / PA-018`
- 能力接入：`PA-020 / PA-021`
- 生命周期横切：`PA-022`
- 观测与监控：`PA-024`

## Backlog

- `PA-020` MCP capability bridge
  说明：在 `PA-018` 固定 retrieval boundary、`PA-019` 固定 planner 消费边界后，把 MCP 以 capability registry 的方式接入。
- `PA-021` skills registry 与 bridge
  说明：在 `PA-020` 之后，把 skills 作为可编排高层能力单元接入。
- `PA-022` lifecycle hooks pipeline
  说明：在 graph/runtime/memory/capability 的边界稳定后，补统一 hooks 横切机制。
- `PA-024` 模型监控与 telemetry 聚合面
  说明：把现有 `ModelMonitorPage` 占位页接到真实 provider/model/run 指标。

## Ready

- 暂无

## In Progress

- `PA-018` 分层 context/state subsystem 与 retrieval boundary
  说明：第一阶段已落地 `RetrievedContextState / ContextStateRetriever` 合约，`build_request()`、planner preflight、session summary、图片召回判断、graph handoff、planner continue 摘要、host inspection、宿主原生 `load_retrieved_context` 查询面与前端 `runtime store` 已开始消费结构化 retrieval 结果。前端当前按“增量修改”边界推进：保留原有 `HomeSidebar` 的 `状态 / Tools / Trace` 主体结构，只把 retrieval 以紧凑 summary 追加到右栏 `Trace` 面板；`HomeSessionSidebar` 与 `HomeWorkspace` 不再常驻承载 retrieval 卡片或顶部条，避免左栏导航区和中栏对话区混入 observability 信息。`LongTermMemory` 也已有独立 session 存储边界、显式语言/风格/文件引用/任务系统同步/变更范围偏好写入策略、显式当前任务焦点写入策略、显式 note 写入策略和最小审计字段；`GraphTurnHandoff` 与 `DefaultGraphPlanner` 已开始消费 `project_focus.active_task -> active_task_focus`。同时前端 graph/retrieval 邻接 contract 与 `RunState -> 最小 GraphRun` 派生逻辑也都已开始收敛到 `src/types/runtime.ts` 的共享类型/共享 helper，且已形成独立正式验收审计文档 `02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`；下一步继续补齐 capability 消费链路和更完整的稳定事实写入来源。

## Review

- 暂无

## Blocked

- 暂无

## Done

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
