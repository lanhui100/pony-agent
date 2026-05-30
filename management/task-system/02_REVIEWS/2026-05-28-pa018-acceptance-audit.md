# PA-018 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`
- `docs/architecture/context-state-subsystem.md`
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/graph.rs`
- `src-tauri/src/agent/planner.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/types/runtime.ts`
- `src/stores/runtime.ts`
- `src/components/HomeSidebar.vue`
- `src/components/HomeSessionSidebar.vue`
- `src/components/HomeWorkspace.vue`
- `tests/runtime-store.spec.ts`
- `tests/HomeSidebar.spec.ts`
- `tests/HomeSessionSidebar.spec.ts`
- `tests/HomeWorkspace.spec.ts`

## 审核基线

以 `PA-018` 任务卡中的五类验收标准为准：

1. `A. 结构边界`
2. `B. retrieval boundary`
3. `C. runtime 接入`
4. `D. 文档与可追踪性`
5. `E. 验证`

本审计不判断“看起来差不多”，只判断当前代码与验证证据是否已经足以证明对应项达成。

## 逐项审计

### A. 结构边界

状态：`基本达成，但未收口`

已证明：

- Rust 侧已经存在公开 contract：
  - `TurnContext`
  - `SessionContext`
  - `RunState`
  - `LongTermMemory`
  - `TranscriptContext`
  - `RetrievedContextState`
- `RunState` 已由 retrieval 统一组装，并能承接 `GraphRun + ExecutionCheckpoint` 的稳定视图。
- `LongTermMemory` 已拥有独立 session 存储边界，且能返回 `empty / available`。
- `LongTermMemory` 已有显式、保守、可审计写入策略，至少覆盖：
  - `user_preference.response_language`
  - `user_preference.response_style`
  - `user_preference.file_reference_style`
  - `user_preference.task_system_sync`
  - `user_preference.change_scope`
  - `user_memory.explicit_note`
  - `project_focus.active_task`
  - `project_workflow.acceptance_gate`
- `project_focus.active_task` 已不只停留在 memory 存储层，而是继续进入：
  - `GraphTurnHandoff.active_task_focus`
  - `DefaultGraphPlanner` continue 摘要
  - `HomeSidebar / HomeSessionSidebar / HomeWorkspace` retrieval 视图
  - `HomeSidebar` trace-run 组头与 `HomeWorkspace` run/checkpoint strip
- `project_workflow.acceptance_gate` 也已开始进入：
  - `GraphTurnHandoff.acceptance_focus`
  - `DefaultGraphPlanner` continue 摘要

仍未收口：

- 当前稳定事实来源仍以显式偏好、显式 note、显式当前任务焦点与显式交付门槛为主，覆盖面还偏窄。
- 尚未证明更多项目级稳定事实已经通过同样保守、可审计的方式进入 `LongTermMemory`。

### B. retrieval boundary

状态：`基本达成，但未收口`

已证明：

- `ContextStateRetriever -> RetrievedContextState` 已形成统一 retrieval 入口。
- 宿主层已提供 `load_retrieved_context` 正式查询面。
- `HostControlPlane.inspect()` 也能附带 `RetrievedContextState`。
- 前端 graph/retrieval 邻接类型已开始收敛到共享 contract，避免 retrieval 事实在前端被各组件局部缩写。

仍未收口：

- 更多 capability / bridge 层仍未默认消费 `RetrievedContextState`。
- 宿主与能力接入面还未系统性证明都以 retrieval 作为首选读面。

### C. runtime 接入

状态：`部分达成，但未完成`

已证明：

- `build_request()` 已消费 `RetrievedContextState`。
- planner preflight 已消费 retrieval 历史，而不再直接读原始 `session.history`。
- session summary 已在持久化后通过 retrieval 重建。
- 图片召回判断已开始消费 preliminary retrieval 结果。
- graph handoff 已消费 retrieval，并把 `project_focus.active_task` 上浮为 `active_task_focus`。
- graph handoff 也已把 `project_workflow.acceptance_gate` 上浮为 `acceptance_focus`。
- planner continue 摘要已消费 `active_task_focus / acceptance_focus / last_referenced_file / long_term_memory_entry_count`。
- 前端 `runtime store.applySessionSnapshot()` 里的 store 级 `sessionSummary` 也已优先采用 retrieval `sessionContext.summary`，而不再默认保留原始 snapshot summary。
- 前端 `runtime store.applySessionSnapshot()` 在恢复会话时，也已开始优先从 retrieval `runState.runId` 恢复 store 级 `activeRunId`。
- graph stream 主链在重建 `TurnResult` 的 fallback summary 时，也已改为优先回退 retrieval `session_context.summary`，而不是直接读取原始 snapshot summary。
- 宿主 `load_retrieved_context(session_id)` 现在也已能推断当前非终态 graph run，而不是要求上层必须先显式提供 `run_id`。
- 宿主 `inspect(session_id)` 在 `include_run / include_retrieved` 场景下，也已复用同一条 session-aware run 推断逻辑。
- 宿主侧又开始补齐更高一层的 retrieval-first 聚合读面：`load_session_runtime_view`
  - 该入口把 `session snapshot`
  - `retrieved context`
  - `execution checkpoint`
    聚合成单次高层 runtime 查询响应，减少上层继续分别读取和拼装底层原件
  - 这条链路目前代表 host / adapter 邻接层正在继续向 retrieval-first 收口，但尚未形成前后端全链路完成态
- 前端当前主链已开始默认消费 retrieval：
  - `runtime store.loadSessionState()`
  - `runtime store.submitTurn()`
  - `HomeSidebar` 状态面板
  - `HomeSidebar` run 面板
  - `HomeSessionSidebar` 当前上下文卡片
  - `HomeWorkspace` 顶部 retrieval strip
  - `HomeWorkspace` run/checkpoint strip
  - `HomeWorkspace` graph run 刷新
- 前端 run 相关三处入口还进一步补上了“retrieval 刷新优先于 inspection fallback”：
  - `runtime store.submitTurn()` 会先尝试宿主原生 `load_retrieved_context`；如果刷新后的 retrieval 仍没有活跃 run，就直接新建 graph run，而不再默认回退 `inspect_host`
  - `HomeSidebar` run 面板会先尝试宿主原生 `load_retrieved_context`；如果刷新后的 retrieval 仍没有活跃 run，就保持空 run 视图，而不再回退 `inspect_host`
  - `HomeWorkspace` graph run 刷新会先尝试宿主原生 `load_retrieved_context`；如果刷新后的 retrieval 仍没有活跃 run，就保持空 graph run 视图，而不再回退 `inspect_host`
- 前端还进一步收敛了：
  - graph/retrieval 邻接共享类型
  - `runState -> 最小 GraphRun` 共享 helper

仍未收口：

- runtime / graph 更深层链路仍有未完全切换到 retrieval facts 的部分。
- 更多 UI / capability 入口还未系统性迁移到 retrieval 首选读面。
- 目前仍不能证明“上层默认入口几乎都不再依赖原始 `session/run/checkpoint` 原件”。
- 目前前端生产代码里的默认 run 读取链已不再直接调用 `inspect_host`。
- `load_session_runtime_view` 现已完成宿主 contract、前端 `SessionRuntimeView` 类型、`runtime store.loadSessionState()` 默认读面与对应测试的全链路接通。
- 但这条聚合读面仍只能作为 `C. runtime 接入` 的新增收口证据，不能单独当作 `PA-018` 已整体收口的证明。

### D. 文档与可追踪性

状态：`达成`

已证明：

- 已有专门架构文档：`docs/architecture/context-state-subsystem.md`
- `PA-018` 任务卡已持续回写：
  - 当前进展
  - 下一步动作
  - 当前卡点
  - 当前验收审计结论
- Dashboard / Task Board 已同步当前主线状态。
- 已形成连续 session 日志链，覆盖 retrieval contract、memory facts、planner/handoff 消费、前端 retrieval UI、共享 contract、共享 helper 等关键推进点。
- 本审计文档将验收标准与证据再次独立固化，降低“只有任务卡摘要，没有正式审计载体”的风险。

### E. 验证

状态：`部分达成，但未完成`

已证明：

- Rust 定向验证已通过：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_acceptance_gate_into_long_term_memory -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_does_not_extract_acceptance_gate_from_incidental_acceptance_mentions -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib control_plane::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib retrieved_context_can_infer_active_graph_run_from_session_id -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib inspection_can_infer_session_run_without_explicit_run_id -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib retrieved_context_queries_flow_through_control_plane -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`
  - `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check`
- 其中 `control_plane::tests` 现在还直接覆盖了一条 retrieval fallback 行为：
  - `recording_turn_event_sink_uses_fallback_summary_when_terminal_event_has_none`
  - 这条测试明确证明了：当 graph stream terminal event 不带 `session_summary` 时，`TurnResult` 会消费 fallback summary，而不是默默退回原始 snapshot 读面。
  - 最新一轮还新增两条 session-aware run 验证：
  - `retrieved_context_can_infer_active_graph_run_from_session_id`
  - `inspection_can_infer_session_run_without_explicit_run_id`
- 前端定向验证已通过：
  - `npm exec vitest run tests/runtime-store.spec.ts`
  - 最新一轮还直接覆盖了一条新提交主链行为：刷新 retrieval 后如果仍无活跃 run，`submitTurn()` 应直接启动新的 graph run，而不是默认回退 `inspect_host`
  - 最新一轮 `HomeSidebar.spec.ts` / `HomeWorkspace.spec.ts` 还新增覆盖：刷新 retrieval 后若仍无活跃 run，组件应保持空 run 视图，而不是回退 `inspect_host`
  - `npm exec vitest run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - 最新一轮还直接覆盖了一条恢复链行为：`initializeSessions()` 恢复运行中 graph run 后，`stopTurn()` 应继续走 `stop_graph_run`
  - `npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts`
- 类型与构建验证已通过：
  - `npm exec vue-tsc -- --noEmit`
  - `npm run build`
- 仓库级验证已通过：
  - `npm run verify`

仍未收口：

- 当前没有“整仓 Rust 全量测试闭环”可直接作为最终关单证据。
- 现有验证已经很强，但还没有形成一份逐条映射 A-E 的终态证明包。
- 因此仍不足以支撑“PA-018 已成功完成交付”的最终声明。

## 审核结论

- `PA-018` 已经从“概念性重构任务”进入“有 contract、有真实消费链路、有多轮验证、有持续文档回写”的成熟进行中阶段。
- 当前最强结论不是“完成”，而是：
  - `A / B / D` 基本成立或成立
  - `C / E` 显著推进，但仍未收口
- 最新增量说明：宿主新增的 `load_session_runtime_view` 已完成前后端接通，把 `session snapshot / retrieved context / execution checkpoint` 聚合成更高层 retrieval-first 入口；这进一步强化了 `C. runtime 接入` 的收口方向，但 `PA-018` 仍必须保持 `In Progress`。
- 进一步增量说明：前端 store 又把 `initializeSessions()` 的恢复链切到统一 `loadSessionRuntimeViewState()` helper，减少了对原始 `load_execution_checkpoint` 读面的默认依赖，说明 host / adapter 邻接层正在继续向单一聚合入口收敛。
- 最新验证补充：`submitTurn()` 与 `HomeWorkspace` 的 retrieval refresh / run 派生逻辑已进一步收敛到 `runtime store.resolveDerivedSessionRun()`；`HomeSidebar` 在抽取同一路径时暴露 run-group 回归，因此暂时保留原有稳定实现。这说明 `PA-018` 当前更适合沿“保守收敛、逐点验证”推进，而不是一次性追求全量抽象统一。
- 最新边界修正补充：上一轮把 retrieval UI 扩散到 `HomeSessionSidebar` 与 `HomeWorkspace` 的做法不符合首页分栏职责，现已回收；当前有效实现是保留左栏导航区和中栏对话区原结构，只在右栏 `HomeSidebar` 的 observability 体系内增量整合 retrieval。
- 现阶段最合理的下一步不是重写验收标准，而是继续围绕两类缺口推进：
  1. 更多 capability / UI 入口默认消费 retrieval
  2. 更丰富但仍可审计的稳定事实来源

## 建议后续动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口。
2. 扩展 `LongTermMemory` 的其他保守稳定事实来源，但继续坚持显式、可审计策略。
3. 在接近收口时，再做一轮正式 closeout audit，专门判断是否足以把 `PA-018` 从 `In Progress` 切到 `Done`。
