# PA-018 建立分层 context/state subsystem 与 retrieval boundary

## 状态

- Status: `In Progress`
- Priority: `P2`
- Owner: `Codex`
- Started At: `2026-05-28`

## 目标

在 `session / attachments / transcript / checkpoint` 已有最小闭环的基础上，正式把当前散落在 prompt 注入、session history、run 状态、provider transcript 与附件召回中的信息拆成更清晰的 `context / state / memory / transcript` 结构，建立统一 retrieval boundary，让：

- `TurnContext`
- `SessionContext`
- `RunState`
- `LongTermMemory`

拥有稳定边界，并能被 runtime / planner / 后续 MCP / skills 能力层消费，而不是继续直接拼接原始 `history`。

## 第一阶段交付范围

本阶段先不做完整长期记忆产品化，而是先交付最小可运行边界：

1. 在 Rust 代码中定义第一版公开 contract：
   - `TurnContext`
   - `SessionContext`
   - `RunState`
   - `LongTermMemory`
   - `TranscriptContext`
   - `RetrievedContextState`
2. 定义统一 retrieval contract：
   - `ContextStateQuery`
   - `ContextStateRetriever`
   - `DefaultContextStateRetriever`
3. 把 `DefaultTurnContextBuilder.build_request()` 改成消费 `RetrievedContextState`
4. 把 runtime 的 planner preflight 从直接读取原始 `session.history` 改成消费 retrieval 结果
5. 补齐第一批架构文档和测试

## 输出

- 第一版 `context/state/transcript` 结构化 contract
- 第一版 retrieval boundary 代码实现
- runtime 接入 retrieval 的最小消费链路
- `docs/architecture/context-state-subsystem.md`
- 对应任务系统回写与会话日志

## 验收标准

### A. 结构边界

- 存在公开的 Rust 结构体来表达 `TurnContext / SessionContext / RunState / LongTermMemory`
- `RunState` 能从 `GraphRun + ExecutionCheckpoint` 组装
- `LongTermMemory` 已拥有独立的 session 存储边界；无记录时返回 `empty`，有记录时返回 `available`
- `LongTermMemory` 已具备最小自动写入策略：只记录用户消息里明确表达的长期偏好
- `LongTermMemory` 已具备第二条保守写入策略：只记录用户消息里明确的“记住……”显式 note
- `LongTermMemory` 记录已带有最小审计字段：`source / updated_at_ms`
- `LongTermMemory` 不能继续被原始 `history`、`summary` 或 `checkpoint` 冒充

### B. retrieval boundary

- retrieval 通过统一入口返回 `RetrievedContextState`
- 上层调用者不需要自己直接翻 `history / provider_native_transcript / attachment_assets` 才能拿到可消费结果
- retrieval 返回的是稳定结构，而不是任由上层到处散读底层存储细节

### C. runtime 接入

- `build_request()` 不再直接读取原始 `SessionSnapshot.history`
- planner preflight 不再直接读取原始 `prepared.session.history`
- provider-native transcript 的近线切片通过 retrieval 暴露，而不是在 prompt 构建链路里直接碰原始 transcript
- graph handoff 至少已有一条真实链路消费 `RetrievedContextState`，而不是继续只吃原始 `SessionSnapshot`
- turn 持久化后的 session summary 已重新通过 retrieval 生成，而不是继续直接拼原始 `SessionSnapshot`
- 图片召回判断已开始消费 preliminary retrieval 结果，而不是继续直接读取原始 `SessionSnapshot.history`
- graph planner 的 continue 摘要已开始消费 handoff 中的结构化字段，而不是只复述原始 session summary
- host inspection 已可以直接返回 `RetrievedContextState`，而不是继续只暴露原始 session/run 原件
- 宿主层已提供 `load_retrieved_context` 正式查询面，不必必须先走 inspection 复合响应

### D. 文档与可追踪性

- 有专门的架构边界文档说明第一阶段 contract、当前接入点与非目标
- 任务卡记录当前进展、下一步动作和当前卡点
- 会话日志记录本轮实际改动与验证结果

### E. 验证

当前第一阶段验证命令：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

说明：

- `cargo test --lib` 已在本轮通过，证明 retrieval contract、long-term memory 最小边界和 graph handoff 接入没有打断既有 runtime / graph / session 主线库测试。
- `npm run verify` 已在本轮重新通过，证明前端单测、前端构建和 Rust `cargo check` 仍然成立。
- 真正关闭 `PA-018` 前，仍要确认后续新增的 `LongTermMemory` 写入策略与更深层 retrieval 消费链路也已完成。

## 当前进展

- 已在 `src-tauri/src/agent/context.rs` 中引入：
  - `TurnContext`
  - `SessionContext`
  - `RunState`
  - `LongTermMemory`
  - `TranscriptContext`
  - `RetrievedContextState`
  - `ContextStateQuery`
  - `ContextStateRetriever`
- 已让 `DefaultTurnContextBuilder.build_request()` 消费 `RetrievedContextState`
- 已让 `runtime.plan_turn()` 改为消费 retrieval 结果中的 planner history
- 已在 `src-tauri/src/agent/session.rs` 中为 `LongTermMemory` 增加独立存储字段与最小写入接口
- 已为 `LongTermMemory` 增加第一条保守写入策略：从用户消息中提取明确表达的长期偏好
- 已为 `LongTermMemory` 增加第二条保守写入策略：从用户消息中提取明确的“记住……”显式 note
- 已为 `LongTermMemory` 增加第三条保守写入策略：从用户消息中提取明确的文件引用风格偏好（绝对路径）
- 已为 `LongTermMemory` 增加第四条保守写入策略：从用户消息中提取明确的任务系统同步偏好（更新任务文档 / 回写任务卡）
- 已为 `LongTermMemory` 增加第五条保守写入策略：从用户消息中提取明确的变更范围偏好（不要修改无关文件 / 不要回滚无关改动）
- 已为 `LongTermMemory` 增加第六条保守写入策略：从用户消息中提取明确的当前任务焦点（例如“现在开始 PA-018 / 当前优先推进 PA-018”）
- 已为 `LongTermMemory` 增加第七条保守写入策略：从用户消息中提取明确的交付门槛要求（例如“建立验收标准 / closeout audit / 确保成功完成交付”）
- 已让 `LongTermMemory` 记录保存最小审计字段：`source / updated_at_ms`
- 已让 `runtime.build_graph_turn_handoff()` 与 `GraphEngine.build_turn_handoff()` 开始消费 retrieval 结果中的结构化信息
  - 当前会从 `LongTermMemory.entries` 中提取 `project_focus.active_task`，并把它提升为 `GraphTurnHandoff.active_task_focus`
  - 当前也会从 `LongTermMemory.entries` 中提取 `project_workflow.acceptance_gate`，并把它提升为 `GraphTurnHandoff.acceptance_focus`
- 已让 `runtime.persist_turn_outcome()` 通过 retrieval 重新生成 session summary
- 已让 `runtime.resolve_turn_images()` 先基于 preliminary retrieval 做图片召回判断
- 已让 `DefaultGraphPlanner` 在 continue 摘要中消费 `active_task_focus / acceptance_focus / last_referenced_file / long_term_memory_entry_count`
- 已让 `HostControlPlane.inspect()` 可以附带 `RetrievedContextState`
- 已让宿主层提供 `load_retrieved_context` 直接查询入口
- 已让宿主 retrieval 查询面在只给 `session_id` 时，也能推断当前非终态 graph run
  - `HostControlPlane.load_retrieved_context()` 现在会优先按 `run_id` 解析
  - 如果没有显式 `run_id`，会按 `session_id` 选择当前最新的非终态 run
  - `HostControlPlane.inspect()` 在 `include_run / include_retrieved` 场景下也复用了这条推断逻辑
- 已让 `HostControlPlane.execute_graph_run_stream()` 在 turn-result 缺少 `session_summary` 时回退到 retrieval `session_context.summary`
- 已让前端 `runtime store` 在 `loadSessionState()` 时默认加载 `RetrievedContextState`
- 已让前端 `runtime store` 在 completed / failed / cancelled 终态后刷新 retrieval 视图，减少上层继续只握着原始 `SessionSnapshot` 的概率
- 已让前端 `runtime store.applySessionSnapshot()` 中的 store 级 `sessionSummary` 优先采用 retrieval `sessionContext.summary`
- 已让前端 `runtime store.applySessionSnapshot()` 在恢复会话时优先从 retrieval `runState.runId` 恢复 store 级 `activeRunId`
- 已让前端 `runtime store.submitTurn()` 在 graph run 主提交流程中优先消费 `retrievedContext.runState`
  - 当前会先用 retrieval facts 决定 `start / continue / resume_graph_run_stream`
  - 如果内存里的 retrieval facts 不足，会先通过宿主原生 `load_retrieved_context` 刷新 retrieval 视图
  - 如果刷新后的 retrieval 仍没有活跃 run，则直接按 retrieval 事实启动新的 `start_graph_run_stream`
  - `submitTurn()` 不再把 `inspect_host` 当成默认 run 决策兜底
- 已让前端 `HomeSidebar` 的 run 面板在当前会话 run 列表上优先消费 `retrievedContext.runState`
  - 当前会先用 retrieval facts 构造最小 run 视图
  - 如果内存里的 retrieval facts 不足，会先刷新一次宿主原生 `load_retrieved_context`
  - 如果宿主原生 retrieval 仍不足，则保持空 run 列表，而不再回退 `inspect_host`
- 已让前端 `HomeSidebar` 的状态面板真实消费 `retrievedContext`
  - session summary 已优先显示 retrieval 中的 `sessionContext.summary`
  - sidebar 已显示 retrieval 中的 recent history / recent attachment / long-term memory / 当前任务焦点 / last referenced file / run goal 等结构化事实
- 已让前端 `HomeSessionSidebar` 显示当前会话 retrieval 概览
  - 会话导航侧栏现在会显示 retrieval 中的当前 summary / recent history / recent attachment / long-term memory / 当前任务焦点 / run goal / last referenced file
- 已让前端 `HomeWorkspace` 显示 retrieval 上下文概览
  - 主工作区顶部现在会显示 retrieval 中的当前 summary / history 数量 / attachment 数量 / long-term memory 数量 / 当前任务焦点 / run goal / last referenced file
- 已让前端 `HomeWorkspace` 顶部继续显示更深层的 run/checkpoint retrieval facts
  - 主工作区顶部现在还会显示 retrieval / graph checkpoint 相关的 `active_task_focus / phase / checkpoint status / checkpoint phase / resume count / resumable / last decision / active turn / last completed turn`
- 已让前端局部 `GraphTurnHandoff` 类型与 run 视图 contract 跟进 `active_task_focus`
  - `HomeSidebar` 的 trace-run 组头现在会显示 `lastHandoff.activeTaskFocus`
  - `HomeWorkspace` 的 run/checkpoint strip 现在也会显示 `lastHandoff.activeTaskFocus`
  - 由 retrieval `runState` 派生出来的最小 run 视图也会补上这条事实，减少“只有后端 handoff 有、前端局部 contract 没跟上”的缝隙
- 已让前端 graph/retrieval 邻接类型开始收敛到共享 contract
  - `src/types/runtime.ts` 现在已导出 `GraphTurnHandoff / GraphRun / GraphRunCheckpoint / HostInspectionSnapshot` 等共享类型
  - `runtime store`、`HomeSidebar` 与 `HomeWorkspace` 已开始复用这些共享类型，而不是继续各自维护局部 graph/handoff/checkpoint 结构定义
  - 这让 `active_task_focus`、`last_referenced_file`、`long_term_memory_*` 等结构化事实更容易保持前后端一致
- 已让前端 retrieval `runState -> 最小 GraphRun` 派生逻辑开始收敛到共享 helper
  - `src/types/runtime.ts` 现在已导出 `normalizeGraphRunPhase()` 与 `deriveGraphRunFromRunState()`
  - `HomeSidebar` 与 `HomeWorkspace` 不再各自维护一份手写的最小 run 派生逻辑
  - 这次收敛还顺带补齐了前端最小 `lastHandoff` 缺失的 `recentAttachmentAssetCount / longTermMemoryStatus / longTermMemoryEntryCount`
- 已让前端 `HomeWorkspace` 在刷新当前 graph run 时优先消费 `retrievedContext.runState`
  - 当前会先用 retrieval facts 构造最小 run 视图，并直接用其中的 `runId` 加载 checkpoint
  - 如果内存里的 retrieval facts 不足，会先刷新一次宿主原生 `load_retrieved_context`
  - 如果宿主原生 retrieval 仍不足，则保持空 graph run 视图，而不再回退 `inspect_host`
- 已把 `PA-018` 的正式验收审计固化为独立文档：
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
- 已根据首页分栏边界收回越界 UI 表达：
  - `HomeSessionSidebar` 不再保留 retrieval 当前上下文卡片
  - `HomeWorkspace` 不再保留 `Retrieved Context` 顶部条或常驻 retrieval strip
  - 当前只允许在 `HomeSidebar` 右栏 observability 区做 retrieval 的增量整合，并保留原有 `状态 / Tools / Trace` 主体结构
- 已为首页冻结表面补上最小护栏：
  - 文档层明确 `HomeSessionSidebar / HomeWorkspace / HomeSidebar` 的冻结边界
  - `HomeSessionSidebar.spec.ts` 新增菜单顺序与结构顺序契约测试，防止后续开发再次随手改动左栏菜单与骨架
- 已新增 retrieval 相关单测，并通过最小验证命令
- 已新增架构文档：
  - `docs/architecture/context-state-subsystem.md`
- 已通过更大范围验证：
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`
  - `npm run verify`
- 已通过本轮新增验证：
  - `npm exec vitest run tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - `npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts`
  - `npm run verify`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib context::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_change_scope_preference_into_long_term_memory`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_task_system_sync_preference_into_long_term_memory`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_explicit_file_reference_preference_into_long_term_memory`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_explicit_memory_note_without_overwriting_preferences`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_acceptance_gate_into_long_term_memory -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_does_not_extract_acceptance_gate_from_incidental_acceptance_mentions -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib control_plane::tests -- --nocapture`
  - `cargo test --manifest-path src-tauri/Cargo.toml --lib`
  - `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vue-tsc -- --noEmit`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
  - `npm run build`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm run build`
  - `npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts`
  - `npm exec vitest run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts`
  - `npm exec vue-tsc -- --noEmit`

## 下一步动作

1. 继续把更多前端具体 UI 与 capability 入口切到 `runtime store.retrievedContext` 或宿主原生 `load_retrieved_context`，而不是继续默认只消费 `SessionSnapshot`
2. 评估 `HomeWorkspace` 中更深层的 graph 状态、checkpoint 视图和继续/恢复提示哪些适合进一步收口到 retrieval facts
3. 继续把 `LongTermMemory` 的写入策略从“显式用户偏好”扩展到更完整但仍可审计的稳定事实来源
4. 梳理 `PA-020 / PA-021` 后续要消费的 capability facts，确认它们应该从 `RetrievedContextState` 的哪一层读取
5. 处理更大范围的 Rust 测试与整仓校验，作为后续收口前验证

## 当前卡点

- retrieval boundary 第一阶段已经落地，但 runtime / graph 更深层的消费还没有完全替换
- 图片召回链已经开始切到 retrieval，但附件读取本身仍然属于底层 session store 能力
- planner 已开始消费 handoff 结构化字段，但更深层 capability 消费还没有全部切到 retrieval facts
- host inspection 已开始返回 retrieval 视图，但其它宿主 / capability 入口还没有全部复用这一层
- 宿主层已有 retrieval 原生查询面，前端 `runtime store`、`HomeSidebar` 状态面板、`HomeSessionSidebar` 当前上下文卡片与 `HomeWorkspace` 顶部概览条也已开始消费 retrieval 视图，但其它 UI/capability 还没有系统性迁移到这一入口
- 经过本轮边界校正后，左栏 `HomeSessionSidebar` 与中栏 `HomeWorkspace` 已不再作为 retrieval 的长期承载位置；后续首页 retrieval UI 只能继续往右栏 observability 区收口
- 仓库里存在与本轮改动无关的集成测试装配问题，导致不能直接把 “整仓 cargo test 全绿” 当成本轮完成证据
- `LongTermMemory` 已有最小存储边界、显式语言/风格/文件引用/任务系统同步/变更范围偏好写入策略、显式当前任务焦点/交付门槛写入策略和显式 note 写入策略，但还没有覆盖更完整的稳定事实来源
- 前端 `runtime store.submitTurn()`、`HomeSidebar` run 面板与 `HomeWorkspace` graph run 刷新现在都已补上“先刷新宿主原生 retrieval、再回退 `inspect_host`”的收口，但 capability / bridge 层还没有系统性复用这一模式

## 当前验收审计结论

- 正式审计文档：
  - [2026-05-28-pa018-acceptance-audit.md](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md)
- `A. 结构边界`：基本达成
  - `TurnContext / SessionContext / RunState / LongTermMemory / TranscriptContext / RetrievedContextState` 已存在
  - `LongTermMemory` 已有独立存储边界与多条显式、可审计写入策略，并已开始覆盖显式项目焦点事实 `project_focus.active_task` 与显式交付门槛事实 `project_workflow.acceptance_gate`
  - `project_focus.active_task` 与 `project_workflow.acceptance_gate` 已开始通过 `GraphTurnHandoff.active_task_focus / acceptance_focus` 被更深层 graph/planner 链路消费
  - 未收口点：稳定项目事实来源虽然比之前更多，但仍偏少
- `B. retrieval boundary`：基本达成
  - 已有统一入口 `ContextStateRetriever -> RetrievedContextState`
  - 宿主层已有 `load_retrieved_context`
  - 未收口点：更多 capability / bridge 层还未默认消费这一入口
- `C. runtime 接入`：部分达成但未完成
  - `build_request()`、planner preflight、session summary、图片召回、graph handoff、planner continue 摘要，以及 graph stream turn-result summary fallback 已接入 retrieval
  - `graph handoff` 与 `planner continue` 摘要现在都已开始消费由 retrieval 提取出的 `active_task_focus / acceptance_focus`
- 宿主层 `load_retrieved_context()` 与 `inspect()` 现在都已支持“只给 `session_id` 就推断当前非终态 run”的 retrieval-first 合约，减少上层必须回头枚举原始 run 列表的需要
- 宿主层正在补齐更高一层的 retrieval-first 聚合读面：新增 `load_session_runtime_view`
  - 该入口把 `session snapshot`
  - `retrieved context`
  - `execution checkpoint`
    聚合成单次高层查询响应，减少前端 store 与宿主适配层继续分别拼装底层读面的需要
  - 这一步是在 host / adapter 邻接层继续收口 retrieval-first 默认入口，但目前仍属于进行中增量，不代表 `PA-018` 已完成
  - 前端 `runtime store` 与三个 UI 入口已开始默认消费 retrieval 视图，其中 `runtime store.submitTurn()`、`HomeSidebar` run 面板、`HomeWorkspace` graph run 刷新都已开始优先消费 `retrievedContext.runState`
  - `runtime store.submitTurn()` 在本地 retrieval facts 不足时，会先刷新宿主原生 `load_retrieved_context`；如果刷新后的 retrieval 仍没有活跃 run，就直接启动新的 graph run，而不再默认回退 `inspect_host`
  - `HomeSidebar` run 面板与 `HomeWorkspace` graph run 刷新在 retrieval 仍不足时，现在都会保持空视图，而不再回退 `inspect_host`
  - 这意味着前端生产代码里的默认 run 读取链已不再直接命中原始 `inspect_host`
  - 前端 store 级 `sessionSummary` 也已优先收口到 retrieval `sessionContext.summary`
  - 前端 store 级 `activeRunId` 也开始在会话恢复链里优先对齐 retrieval `runState.runId`
  - `HomeSidebar`、`HomeSessionSidebar` 与 `HomeWorkspace` 也都已开始显示 retrieval 提取出的当前任务焦点事实；同时 `HomeSidebar` trace-run 组头和 `HomeWorkspace` run/checkpoint strip 也已开始消费 `lastHandoff.activeTaskFocus`
  - 前端 graph/retrieval 邻接 contract 也已开始统一到共享类型，降低 retrieval 边界在前端被局部“缩写”或漂移的风险
  - 前端 `runState -> 最小 GraphRun` 的派生逻辑也已开始统一到共享 helper，降低“共享类型一致但共享解释逻辑继续分叉”的风险
  - 未收口点：runtime / graph 更深层链路与更多上层入口仍未完全切换
- `D. 文档与可追踪性`：达成
  - 架构文档、任务卡、Dashboard、Task Board 与会话日志均已持续回写
- `E. 验证`：部分达成但未完成
  - 已有多组前端、定向 Rust、仓库级 `npm run verify` 与 `cargo check` 证据
  - 最新一轮 `graph::tests`、`planner::tests` 与 Rust `lib` 全量 `109` 测试也已通过，证明这次新增的长期记忆结构化事实消费没有破坏既有 graph/planner/runtime 主线
  - 最新一轮前端 run-view contract 补齐验证也已通过，证明 `active_task_focus` 进入局部 handoff 类型与 run/checkpoint 视图后没有破坏前端类型检查、构建与关键单测
  - 最新一轮 acceptance gate 定向验证也已通过，证明新增的显式交付门槛事实可以进入 memory / graph / planner 链路而不破坏现有定向测试
  - 最新一轮共享类型收敛验证也已通过，证明 `runtime store`、`HomeSidebar`、`HomeWorkspace` 切到统一 graph/retrieval 类型后没有破坏前端类型检查、构建与关键回归测试
  - 最新一轮 runtime-store summary 收口验证也已通过，证明 `loadSessionState()` 不再把原始 snapshot summary 当成默认 store 事实源
  - 最新一轮 runtime-store activeRunId 恢复验证也已通过，证明 `initializeSessions()` 后运行中的 graph run 不会再把主 run 身份丢成 `null`
  - 最新一轮共享 run-state 派生 helper 收敛验证也已通过，证明 `HomeSidebar` 与 `HomeWorkspace` 切到统一最小 run 派生逻辑后没有破坏前端类型检查、构建与关键回归测试
  - 最新一轮 `control_plane::tests` 还直接证明了：当 graph stream terminal event 不带 `session_summary` 时，fallback summary 会落到 retrieval，而不是退回原始 snapshot
  - 最新一轮 `control_plane::tests` 还新增证明：`load_retrieved_context(session_id)` 与 `inspect(session_id)` 已能推断当前非终态 graph run
  - 最新一轮 `runtime-store.spec.ts` 还新增证明：`submitTurn()` 在 retrieval 刷新后若仍无活跃 run，会直接新建 graph run，而不是默认回退 `inspect_host`
- 最新一轮 `HomeSidebar.spec.ts` 与 `HomeWorkspace.spec.ts` 还新增证明：当 retrieval 刷新后仍无活跃 run 时，两个组件都会保持空 run 视图，而不是回退 `inspect_host`
- 最新一轮宿主侧又补入 `load_session_runtime_view` 聚合查询面，把 `session snapshot / retrieved context / execution checkpoint` 合并为高层 runtime 视图入口，目标是继续消除 `runtime store` 直接拼装底层读面的耦合
- 该聚合读面现已完成一轮前后端全链路接通：宿主 contract、前端 `SessionRuntimeView` 类型、`runtime store.loadSessionState()` 默认读面与 `runtime-store` / `HomeSidebar` / `HomeWorkspace` 回归验证均已落地
- 最新一轮又把 `initializeSessions()` 的空会话 / checkpoint 恢复链切到统一 `loadSessionRuntimeViewState()` helper，上层 store 不再先单独读取 `load_execution_checkpoint` 再回灌 `loadSessionState()`
- 随后又把 `submitTurn()` 与 `HomeWorkspace` 的“本地 runState 不足时先刷新 retrieval 再推断当前 run”逻辑收敛到 `runtime store.resolveDerivedSessionRun()`；`HomeSidebar` 在验证中暴露 run-group 回归风险后，保持原有已验证路径不动，避免为了抽象统一破坏稳定 UI
- 最新边界修正说明：上一轮曾把 retrieval 扩散到左栏和中栏，这与首页职责分层不符；当前已回收这些越界 UI，恢复为“左栏导航 / 中栏对话 / 右栏 observability”，并明确后续只能做增量整合，不能重写原有 `状态 / Trace / Tools` 实现
- 但这仍只是 `PA-018` 在 host / adapter 邻接层的新增收口证据，不足以把整项任务改判为完成，因此 `PA-018` 继续保持 `In Progress`
- 未收口点：尚未形成可直接支撑最终关单的整体验证闭环，尤其是更广范围的 Rust/整仓验证与验收逐项证据仍需补齐

## 断点续跑提示

继续之前先看：

- [docs/architecture/context-state-subsystem.md](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/context-state-subsystem.md)
- [docs/architecture/terminology.md](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/terminology.md)
- [src-tauri/src/agent/context.rs](C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- [src-tauri/src/agent/runtime.rs](C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [management/task-system/00_DASHBOARD.md](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [management/task-system/01_TASK_BOARD.md](C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
