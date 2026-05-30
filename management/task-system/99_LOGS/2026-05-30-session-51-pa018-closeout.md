# 2026-05-30 Session 51 - PA-018 Closeout

## 背景

- 目标：把 `PA-018` 的剩余缺口收口到可交付状态，并完成正式 closeout
- 范围约束：
  - 保留在 `PA-018`：retrieval boundary、本体消费链路、`LongTermMemory` 稳定事实来源
  - 转交 `PA-024`：retrieval observability / trace 展示语义
  - 转交 `PA-025`：`Build Context` 与 cache-friendly prompt 边界

## 本轮代码收口

### 1. planner 依赖收窄

- `src-tauri/src/agent/planner.rs`
  - 新增 `GraphPlanningRunView`
  - `GraphPlanningContext` 不再默认携带完整 `GraphRun`
  - planner continue 摘要补充消费 `closeout_focus`

### 2. graph / runtime retrieval-first 收口

- `src-tauri/src/agent/graph.rs`
  - `build_turn_handoff()` 的 checkpoint 信息来自 `retrieved.run_state`
  - graph 测试同步改为显式设置 retrieval run state
- `src-tauri/src/agent/runtime.rs`
  - runtime handoff 构建不再传 raw checkpoint 给 graph

### 3. LongTermMemory 稳定事实来源补齐

- `src-tauri/src/agent/session.rs`
  - 新增：
    - `project_dependency.prerequisite`
    - `project_workflow.closeout_requirement`
    - `project_scope.task_boundary`
  - 任务号提取改为大小写归一，避免 `pa-024 / pa-025` 这类输入漏记

### 4. retrieval 稳定性与默认读面

- `src-tauri/src/agent/context.rs`
  - long-term memory 读取测试改为检查事实存在，而不是依赖写入顺序
- `src-tauri/src/lib.rs`
  - 前端命令面继续维持不暴露 `load_session_snapshot`

## 验证

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib append_turn_extracts_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
npm run verify
```

结果：

- planner 定向测试通过：`16 passed`
- graph 定向测试通过：`11 passed`
- session long-term memory 提取定向测试通过：`10 passed`
- Rust `lib` 全量通过：`120 passed`
- `npm run verify` 通过：前端单测、前端构建、Rust `cargo check` 全部通过

## 任务系统回写

- `PA-018` 任务卡改为 `Done`
- `PA-018 Acceptance Audit` 改为完成态裁定
- `01_TASK_BOARD.md` 已把 `PA-018` 移入 `Done`
- `00_DASHBOARD.md` 已把下一条建议主线切到 `PA-025`

## 结论

`PA-018` 已达到可交付状态并完成关闭。后续如继续推进：

1. `PA-025` 负责 cache-friendly prompt 与 `Build Context` 边界
2. `PA-024` 负责 retrieval observability 与 trace 语义
