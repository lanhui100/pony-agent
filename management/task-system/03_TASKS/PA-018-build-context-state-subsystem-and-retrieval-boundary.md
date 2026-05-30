# PA-018 建立分层 context/state subsystem 与 retrieval boundary

## 状态

- Status: `Done`
- Priority: `P2`
- Owner: `Codex`
- Started At: `2026-05-28`
- Completed At: `2026-05-30`

## 目标

在 `session / attachments / transcript / checkpoint` 已有最小闭环的基础上，把散落在 prompt 注入、session history、run 状态、provider transcript 与附件召回中的信息拆成清晰的 `context / state / memory / transcript` 结构，建立统一 retrieval boundary，让：

- `TurnContext`
- `SessionContext`
- `RunState`
- `LongTermMemory`

拥有稳定边界，并被 runtime / graph / planner / 宿主查询面消费，而不是继续默认拼接原始 `history`。

## 最终边界

### In Scope

- retrieval boundary 本体与 Rust contract
- runtime / graph / planner 对 retrieval 结果的真实消费链路
- `LongTermMemory` 的独立存储边界与保守、可审计的稳定事实来源
- 宿主 retrieval-first 查询面与默认应用读面收口

### Out of Scope

- retrieval 观测、trace 展示语义、监控面：转交 `PA-024`
- `RetrievedContextState -> prompt/request` 映射、`Build Context` 解释力、cache-friendly prompt 边界：转交 `PA-025`

## 已完成交付

1. 建立第一版 retrieval contract：
   - `TurnContext`
   - `SessionContext`
   - `RunState`
   - `LongTermMemory`
   - `TranscriptContext`
   - `RetrievedContextState`
   - `ContextStateQuery`
   - `ContextStateRetriever`
   - `DefaultContextStateRetriever`
2. 把 `DefaultTurnContextBuilder.build_request()`、runtime planner preflight、session summary 重建、图片召回判断切到 retrieval 结果。
3. 让 graph handoff 真实消费 `RetrievedContextState`，并把结构化 memory facts 上浮到 `GraphTurnHandoff`。
4. 收窄 planner 对运行态的依赖：
   - `GraphPlanningContext` 不再默认携带完整 `GraphRun`
   - planner 只消费窄化后的 run 视图与 handoff 结构化字段
5. 为 `LongTermMemory` 补齐稳定事实来源，并保持显式、保守、可审计：
   - `user_preference.response_language`
   - `user_preference.response_style`
   - `user_preference.file_reference_style`
   - `user_preference.task_system_sync`
   - `user_preference.change_scope`
   - `user_memory.explicit_note`
   - `project_focus.active_task`
   - `project_workflow.acceptance_gate`
   - `project_dependency.prerequisite`
   - `project_workflow.closeout_requirement`
   - `project_scope.task_boundary`
6. 宿主默认应用读面已收口到：
   - `load_session_runtime_view`
   - `load_retrieved_context`
   同时 Tauri 前端命令面不再暴露 `load_session_snapshot`。
7. 已形成架构文档、任务卡、验收审计与会话日志闭环。

## 验收结论

### A. 结构边界

状态：`达成`

- Rust 公开 contract 已存在并稳定：
  - `TurnContext / SessionContext / RunState / LongTermMemory / TranscriptContext / RetrievedContextState`
- `RunState` 已能稳定组装 `GraphRun + ExecutionCheckpoint` 的 retrieval 视图。
- `LongTermMemory` 已拥有独立 session 存储边界，返回 `empty / available`。
- `LongTermMemory` 已带最小审计字段：`source / updated_at_ms`。
- `LongTermMemory` 已具备显式偏好、显式 note 与项目级稳定事实的保守写入策略。

### B. retrieval boundary

状态：`达成`

- retrieval 统一通过 `ContextStateRetriever -> RetrievedContextState` 暴露。
- 上层默认不必自行翻 `history / provider_native_transcript / attachment_assets` 才能消费上下文。
- retrieval 输出为稳定结构；`LongTermMemory.entries` 也已按稳定键排序，降低写入顺序抖动。

### C. runtime 接入

状态：`达成`

- `build_request()` 不再直接读取原始 `SessionSnapshot.history`。
- planner preflight 不再直接读取原始 `prepared.session.history`。
- provider-native transcript 的近线切片通过 retrieval 暴露。
- graph handoff 已真实消费 `RetrievedContextState`，checkpoint 信息来自 `retrieved.run_state`。
- 持久化后的 session summary 已通过 retrieval 重建。
- 图片召回判断已消费 preliminary retrieval。
- planner continue 摘要已消费结构化 handoff facts：
  - `active_task_focus`
  - `acceptance_focus`
  - `closeout_focus`
  - `last_referenced_file`
  - `long_term_memory_entry_count`
- `HostControlPlane.inspect()` 已可直接返回 `RetrievedContextState`。
- 宿主层已提供 `load_retrieved_context` 正式查询面。
- 默认应用读面已收口到 `load_session_runtime_view / load_retrieved_context`；raw `session/checkpoint` 能力保留为更底层调试面，不再是前端默认入口。

### D. 文档与可追踪性

状态：`达成`

- 已有架构文档：`docs/architecture/context-state-subsystem.md`
- 已有正式验收审计：`management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
- 任务卡、Dashboard、Task Board 与 session log 已完成回写

### E. 验证

状态：`达成`

本轮完成态验证命令：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

结果：

- `planner::tests` 通过：`16 passed`
- `graph::tests` 通过：`11 passed`
- `append_turn_extracts_` 相关 session 定向测试通过：`10 passed`
- Rust `lib` 全量通过：`120 passed`
- `npm run verify` 通过：前端单测、前端构建与 Rust `cargo check` 全部通过

## 收口说明

- `PA-018` 到此关闭，后续不再继续吸收 retrieval 观测与 `Build Context` 优化范围。
- retrieval 观测与 trace 展示转交 `PA-024`。
- cache-friendly prompt 边界与 `RetrievedContextState -> prompt/request` 映射转交 `PA-025`。
