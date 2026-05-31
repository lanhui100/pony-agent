# Context/State Subsystem

## 状态

- 对应任务：`PA-018`
- 当前状态：`Done`
- 完成时间：`2026-05-30`

## 目标

`PA-018` 的目标不是“直接做完整长期记忆产品”，而是先把运行时真正依赖的几层信息拆清，建立统一 retrieval boundary，让 runtime / graph / planner / 宿主查询面默认消费结构化结果，而不是继续直接拼接原始 `history / checkpoint / transcript`。

本子系统的稳定 contract 包括：

- `TurnContext`
- `SessionContext`
- `RunState`
- `LongTermMemory`
- `TranscriptContext`
- `RetrievedContextState`

## 完成后的边界

### TurnContext

职责：

- 表达当前 turn 直接消费的信息
- 承载当前用户消息
- 承载本轮图片输入
- 标记本轮是否引用图像语义

### SessionContext

职责：

- 表达当前会话稳定可消费的近线信息
- 暴露裁剪后的 recent history，而不是整段原始历史
- 暴露 recent attachment assets
- 暴露 `summary / title / last_referenced_file`

当前约束：

- 最近 `12` 条 history
- 最近 `8` 个 attachment assets

### RunState

职责：

- 表达 graph run 与 execution checkpoint 的结构化状态
- 让上层不必再从原始 `GraphRun / ExecutionCheckpoint` 自己猜运行态

当前字段覆盖：

- `run_id / goal / phase`
- `active_turn_id / last_completed_turn_id`
- `resume_count`
- `last_decision_summary`
- `execution_checkpoint_status / execution_checkpoint_phase`

### LongTermMemory

职责：

- 承载 session 级、稳定、可审计的长期事实
- 与 `history / summary / checkpoint` 明确分家

当前边界：

- 独立存储边界：`SessionState.long_term_memory_entries`
- retrieval 映射为 `LongTermMemory`
- 无记录时：`status = empty`
- 有记录时：`status = available`
- 每条记录保留：
  - `source`
  - `updated_at_ms`

当前已覆盖的稳定事实来源：

- 用户偏好
  - `user_preference.response_language`
  - `user_preference.response_style`
  - `user_preference.file_reference_style`
  - `user_preference.task_system_sync`
  - `user_preference.change_scope`
- 显式 note
  - `user_memory.explicit_note`
- 项目级稳定事实
  - `project_focus.active_task`
  - `project_workflow.acceptance_gate`
  - `project_dependency.prerequisite`
  - `project_workflow.closeout_requirement`
  - `project_scope.task_boundary`

写入原则：

- 只接受显式、保守、可审计的事实
- 不把原始 `history`、`summary` 或 `checkpoint` 冒充为 memory
- 同类稳定事实按 `kind` 去重更新，避免无限累积陈旧状态

### TranscriptContext

职责：

- 暴露 provider-native transcript 的近线切片
- 继续服务 reasoning / provider-native tool flow
- 但通过 retrieval boundary 输出，而不是让上层直接读取 `SessionSnapshot.provider_native_transcript`

当前约束：

- 最近 `24` 条 transcript messages

## Retrieval Contract

这一层的核心契约是：

1. `ContextStateQuery`
   说明一次 retrieval 需要哪些输入：用户消息、图片、session、可选 run、可选 checkpoint。
2. `ContextStateRetriever`
   把查询统一转换成 `RetrievedContextState`。
3. `DefaultContextStateRetriever`
   当前默认实现。

补充稳定性约束：

- retrieval 输出是结构化状态，而不是底层存储细节透传
- `LongTermMemory.entries` 会按稳定键排序，降低写入顺序漂移对上层消费的影响

## 当前默认消费链路

已落地的 retrieval-first 消费点：

1. `DefaultTurnContextBuilder.build_request()`
   默认消费 `RetrievedContextState`，不再直接读取原始 `SessionSnapshot.history`。
2. `runtime.plan_turn()`
   planner preflight 默认消费 retrieval history，而不是直接读原始 history。
3. `runtime.persist_turn_outcome()`
   session summary 在持久化后通过 retrieval 重建。
4. `runtime.resolve_turn_images()`
   图片召回判断先消费 preliminary retrieval。
5. `runtime.build_graph_turn_handoff()`
   handoff 组装基于 retrieval 结果。
6. `GraphEngine.build_turn_handoff()`
   `checkpoint_status / checkpoint_phase` 已来自 `retrieved.run_state`，不再由 raw checkpoint 直接灌入。
7. `DefaultGraphPlanner`
   默认消费窄化后的 run 视图与 `GraphTurnHandoff` 结构化字段，而不是完整 `GraphRun`。
8. `HostControlPlane.inspect()`
   可直接返回 `RetrievedContextState`。
9. `load_retrieved_context`
   宿主正式 retrieval 查询面。
10. `load_session_runtime_view`
    宿主高层聚合读面，把：
    - `session snapshot`
    - `retrieved context`
    - `execution checkpoint`
    合并为单次高层查询响应。

## 应用默认读面

完成态下，应用默认宿主读面已经收口到：

- `load_session_runtime_view`
- `load_retrieved_context`

同时：

- Tauri 前端命令面不再暴露 `load_session_snapshot`
- raw `session / checkpoint` 能力仍保留在更底层调试面，但不再是前端默认消费链路

## 与 graph / planner 的关系

`PA-018` 完成后，graph / planner 侧的关键边界是：

- graph handoff 默认消费 retrieval 事实
- planner 不再默认依赖完整 `GraphRun`
- planner continue 摘要会消费：
  - `active_task_focus`
  - `acceptance_focus`
  - `closeout_focus`
  - `last_referenced_file`
  - `long_term_memory_entry_count`

这意味着 planner 的“继续推进依据”已经从原始聊天历史与宽运行态，收口到了稳定 handoff facts。

## 非目标

下列内容不再归本卡继续吸收：

- retrieval 观测、trace 展示语义、监控面
  - 转交 `PA-024`
- `RetrievedContextState -> prompt/request` 映射
- `Build Context` 解释力
- cache-friendly prompt 边界
  - 转交 `PA-025`
- 向量检索、RAG、跨设备长期记忆产品化
- 完整 MCP / skills retrieval 消费层

## 验证

完成态验证命令：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

结果：

- Rust `planner` 定向测试通过
- Rust `graph` 定向测试通过
- Rust `session` long-term memory 提取定向测试通过
- Rust `lib` 全量通过
- 前端单测、前端构建与 Rust `cargo check` 通过

## 代码入口

- retrieval contract 与默认实现：
  [context.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/context.rs)
- long-term memory 存储边界与稳定事实提取：
  [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- runtime 接入：
  [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- graph / planner 收口：
  [graph.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/graph.rs)
  [planner.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/planner.rs)
- 宿主 retrieval-first 查询面：
  [control_plane.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/control_plane.rs)
  [lib.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/lib.rs)
