# Context/State Subsystem V1

## 目标

这份文档对应 `PA-018` 的第一阶段交付，目标不是一次性做完长期记忆，而是先把运行时真正依赖的几层信息拆清：

- `TurnContext`
- `SessionContext`
- `RunState`
- `LongTermMemory`
- `TranscriptContext`
- 统一 retrieval boundary

第一阶段重点是让 runtime 和 prompt 构建链路可以消费结构化检索结果，而不是继续直接拼接原始 `history`。

## 当前边界

### TurnContext

职责：

- 表示当前 turn 直接消费的信息
- 承载当前用户消息
- 承载本轮图片输入
- 标记这轮是否引用图像语义

当前代码入口：

- [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)

### SessionContext

职责：

- 表示当前会话稳定可消费的近线信息
- 提供最近一段 history，而不是整段原始历史
- 提供最近附件资产切片
- 提供 summary、title、last referenced file

当前约束：

- 只暴露最近 `12` 条 history
- 只暴露最近 `8` 个 attachment assets

### RunState

职责：

- 表示 graph run 与 execution checkpoint 的结构化状态
- 让上层不必自己从 run/checkpoint 原始对象猜状态

当前字段覆盖：

- `run_id / goal / phase`
- `active_turn_id / last_completed_turn_id`
- `resume_count`
- `last_decision_summary`
- `execution_checkpoint_status / execution_checkpoint_phase`

### LongTermMemory

当前最小状态：

- 已有独立的 session 存储边界：`SessionState.long_term_memory_entries`
- retrieval 会把它映射成 `LongTermMemory`
- 当没有记录时返回 `status = empty`
- 当存在记录时返回 `status = available`
- 已有第一条保守的自动写入策略：只在用户消息里出现明确的长期偏好表达时写入 memory
- 当前已覆盖两类显式偏好：
  - `user_preference.response_language`
  - `user_preference.response_style`
- 当前也已覆盖第三类显式偏好：
  - `user_preference.file_reference_style`
- 当前也已覆盖第四类显式偏好：
  - `user_preference.task_system_sync`
- 当前也已覆盖第五类显式偏好：
  - `user_preference.change_scope`
- 当前也已覆盖第一类显式项目焦点事实：
  - `project_focus.active_task`
  - `project_workflow.acceptance_gate`
- 已有第二条保守的自动写入策略：只在用户消息里出现明确的“记住……”指令时写入显式 note
- 当前显式 note 使用：
  - `user_memory.explicit_note`
- 每条记录都会保存：
  - `source`
  - `updated_at_ms`

这让 runtime 不必再把 session summary、history 或 checkpoint 混称为 memory，同时也没有提前把它扩展成完整的 memory product。

当前关于 `project_focus.active_task` 的约束是：

- 只在用户消息里明确出现“现在开始 / 当前优先推进 / focus on / start with / prioritize”一类表述时写入
- 只有同时识别到任务样式标识时才落地，例如 `PA-018`
- 同类记录按 `kind` 去重更新，只保留当前激活任务，而不是无限追加旧任务焦点

### TranscriptContext

职责：

- 保留 provider-native transcript 的近线切片
- 继续服务 reasoning/provider-native tool flow
- 但通过 retrieval boundary 暴露，而不是让上层直接摸 `SessionSnapshot.provider_native_transcript`

当前约束：

- 只暴露最近 `24` 条 transcript messages

## Retrieval Contract

第一阶段新增了两层契约：

1. `ContextStateQuery`
   用来声明一次检索需要哪些输入：用户消息、图片、session、可选 run、可选 checkpoint。
2. `ContextStateRetriever`
   用来返回 `RetrievedContextState`。

当前默认实现是 `DefaultContextStateRetriever`。

前端侧也开始同步收敛 retrieval 邻接 contract：

- `src/types/runtime.ts` 现在已导出共享的
  - `GraphRunPhase`
  - `GraphDecision`
  - `GraphStep`
  - `GraphTurnHandoff`
  - `GraphRun`
  - `GraphRunCheckpoint`
  - `GraphRunEvent`
  - `GraphRunTurnResponse`
  - `GraphRunControlResponse`
  - `GraphRunStreamStartResponse`
  - `HostInspectionSnapshot`
- `runtime store`、`HomeSidebar`、`HomeWorkspace` 已开始复用这些共享类型，而不是继续各自维护局部 graph/handoff/checkpoint 结构定义
- 这让 `active_task_focus`、`acceptance_focus`、`last_referenced_file`、`long_term_memory_*` 等 handoff 事实更容易保持前后端一致，不再只依赖组件内部的“最小猜测类型”
- `src/types/runtime.ts` 也开始导出共享 helper：
  - `normalizeGraphRunPhase()`
  - `deriveGraphRunFromRunState()`
- `HomeSidebar` 与 `HomeWorkspace` 现在都通过同一个 helper 把 `RetrievedContextState.runState` 派生成最小 `GraphRun`，减少“共享类型一致但共享映射逻辑继续分叉”的风险

## 当前接入点

这轮已经接上的真实消费点有：

1. `runtime.prepare_turn()`
   先调用 `retrieve_context_state()`，再构建 provider request。
2. `DefaultTurnContextBuilder.build_request()`
   改为消费 `RetrievedContextState`，不再直接从 `SessionSnapshot.history` 取原始数据。
3. `runtime.plan_turn()`
   planner 预判阶段改为消费 `retrieved.planner_history()`，而不是直接读 `prepared.session.history`。
4. `runtime.build_graph_turn_handoff()`
   会先构建 `RetrievedContextState`，再交给 graph 组装 handoff。
5. `GraphEngine.build_turn_handoff()`
   现在消费的是 retrieval 结果里的 `SessionContext / RunState / LongTermMemory`，而不是原始 `SessionSnapshot`。
6. `runtime.persist_turn_outcome()`
   持久化后会重新走一次 retrieval，再生成 session summary，减少后续链路对原始 `SessionSnapshot` 的直接拼装依赖。
7. `runtime.resolve_turn_images()`
   现在会先基于“无图片”的 preliminary retrieval 做图片召回判断，再构建最终 retrieval，减少图片召回链路对原始 `SessionSnapshot.history` 的直接判断依赖。
8. `DefaultGraphPlanner`
   现在会消费 `GraphTurnHandoff` 里的结构化字段，例如 `active_task_focus`、`acceptance_focus`、`last_referenced_file` 与 `long_term_memory_entry_count`，而不只是复述原始 session summary。
9. `HostControlPlane.inspect()`
   现在可以在 inspection 响应里直接附带 `RetrievedContextState`，让宿主侧观察入口也能消费统一 retrieval 视图，而不是继续只暴露原始 session/run 原件。
   当只给 `sessionId`、没有显式 `runId` 时，inspection 在 `includeRun / includeRetrieved` 场景下也会优先推断当前非终态 graph run。
10. `load_retrieved_context`
   宿主层现在也有了直接加载 `RetrievedContextState` 的正式查询面，不必必须先通过 inspection 再从复合响应里拆 retrieval 视图。  
   当只给 `sessionId`、没有显式 `runId` 时，这条查询面也会优先推断当前非终态 graph run，把 session 级运行态一起带进 retrieval 结果。
11. `load_session_runtime_view`
   宿主层正在补齐一个更高阶的 runtime 聚合查询面，把：
   - `session snapshot`
   - `retrieved context`
   - `execution checkpoint`
     合并为单次高层响应。  
   这条入口的目标不是重新暴露更多底层原件，而是让 host / adapter 邻接层也能优先消费 retrieval-first runtime 视图，减少前端 store 与宿主调用层分别拼装三段读面的耦合。  
   当前这条链路已经完成一轮宿主到前端的默认读面切换与回归验证，因此它代表的是 `PA-018` 在 host / adapter 邻接层继续向 retrieval-first 收口的稳定入口；但它仍不是整项任务的完成态证明。
12. `execute_graph_run_stream()` 的 turn-result fallback summary
   graph stream 主链在从事件流重建 `TurnResult` 时，如果 terminal event 没带 `session_summary`，现在会回退到 retrieval `session_context.summary`，而不再直接读取原始 `SessionSnapshot.summary`。
13. 前端 `runtime store`
   现在会在 `loadSessionState()` 时默认加载 `RetrievedContextState`，并在 completed / failed / cancelled 终态后刷新 retrieval 视图，给后续 UI / capability 层提供稳定的上层入口。
   store 级 `sessionSummary` 也会优先采用 retrieval `sessionContext.summary`，而不是把原始 `SessionSnapshot.summary` 长期保留成默认事实源。
   会话恢复链里的 store 级 `activeRunId` 现在也会优先采用 retrieval `runState.runId`，避免运行中的 graph run 在恢复后丢失主 run 身份。
14. 前端 `runtime store.submitTurn()`
   当前主提交流程在决定 `start_graph_run_stream / continue_graph_run_stream / resume_graph_run_stream` 时，会优先消费内存里的 `retrievedContext.runState`；如果本地 retrieval facts 不足，会先通过宿主原生 `load_retrieved_context` 刷新一次 retrieval 视图。
   如果刷新后的 retrieval 仍没有活跃 run，`submitTurn()` 就直接按 retrieval 事实启动新的 graph run，而不再把 `inspect_host` 拉原始 run 列表当成默认决策兜底。
15. `HomeSidebar` 状态面板
   现在会直接消费 `retrievedContext`，优先显示 retrieval 中的 `sessionContext.summary`，并展示 `recent_history / recent_attachment_assets / long_term_memory / active_task_focus / last_referenced_file / run goal` 等结构化事实。
16. `HomeSidebar` run 面板
   当前在构造当前会话的 run 列表时，也会优先消费 `retrievedContext.runState` 生成最小 run 视图；如果本地 retrieval facts 不足，会先刷新一次宿主原生 retrieval。
   如果刷新后的 retrieval 仍没有活跃 run，面板就保持空 run 视图，而不再回退到 `inspect_host` 拉取原始 run 列表。
17. `HomeSessionSidebar` 当前上下文卡片
   现在会直接消费 `retrievedContext`，在会话导航侧栏里显示当前 summary、recent history/attachment 计数、long-term memory 状态、active task、run goal 与 last referenced file。
18. `HomeWorkspace` 顶部上下文概览条
   现在会直接消费 `retrievedContext`，在主工作区顶部显示当前 summary、history/attachment 计数、long-term memory 数量、active task、run goal 与 last referenced file。
19. `HomeWorkspace` 顶部 run/checkpoint strip
   同一块顶部区域也开始展示更深层的 retrieval / graph facts，例如 `active_task_focus / phase / checkpoint status / checkpoint phase / resume count / resumable / last decision / active turn / last completed turn`，减少主工作区继续只依赖原始 run/checkpoint 原件自行推断状态的需要。
20. `HomeWorkspace` graph run 刷新
   当前在刷新主工作区的当前 graph run 时，也会优先消费 `retrievedContext.runState` 构造最小 run 视图，并直接用其中的 `runId` 加载 checkpoint；如果本地 retrieval facts 不足，会先刷新一次宿主原生 retrieval。
   如果刷新后的 retrieval 仍没有活跃 run，主工作区就保持空 graph run 视图，而不再回退到 `inspect_host`。

## 当前非目标

这轮还没有做这些事情：

- 向量检索或 RAG
- 跨设备同步的长期记忆
- 完整的 memory write-back pipeline
- MCP / skills 对 retrieval boundary 的消费
- telemetry 对 `RunState` 的聚合消费

这些属于 `PA-018` 后续阶段或 `PA-020 ~ PA-024` 的接续工作。

## 第一阶段验收

代码验收至少满足：

1. 运行时存在公开的结构化 contract，而不是只有 prompt 拼装辅助函数。
2. `build_request()` 消费 `RetrievedContextState`。
3. planner 预判不再直接读取原始 `session.history`。
4. `RunState` 可以从 `GraphRun + ExecutionCheckpoint` 组装。
5. `LongTermMemory` 至少拥有独立存储边界，并能返回 `empty / available` 的真实状态。
6. `LongTermMemory` 至少有一条真实但保守的写入策略，且记录带有审计字段。
7. graph handoff 至少有一条真实链路消费 retrieval 结果，而不是继续只依赖原始 `SessionSnapshot`。
8. 有专门测试覆盖 retrieval boundary 的结构映射、上下文裁剪、最小 memory write/read 和 handoff 接入。

当前验证命令：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture
```

## 下一步

下一步优先做两件事：

1. 让更多 runtime / graph / capability 邻接边界消费 `RetrievedContextState`，而不是继续在不同层重复拼 history。
2. 明确 `LongTermMemory` 的写入时机、更新策略和审计入口，但不提前把它产品化为完整 memory system。
3. 继续把前端具体视图与 capability 入口切到 `runtime store` 持有的 retrieval 视图，而不是继续默认透传原始 `SessionSnapshot`。
4. 把宿主新增的 `load_session_runtime_view` 继续接到前端默认读面，验证 host / adapter 邻接层也能通过单次高层查询优先消费 retrieval-first runtime 视图。
