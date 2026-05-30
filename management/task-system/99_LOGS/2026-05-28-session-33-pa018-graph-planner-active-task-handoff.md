# 2026-05-28 Session 33 PA-018 Graph Planner Active Task Handoff

## 本轮目标

- 继续推进 `PA-018`
- 把 `LongTermMemory` 里的显式当前任务焦点真正接进更深层 graph/planner 消费链
- 补齐对应 Rust 验证与任务文档回写

## 本轮改动

- 更新：
  - `src-tauri/src/agent/graph.rs`
  - `src-tauri/src/agent/planner.rs`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 给 `GraphTurnHandoff` 增加了结构化字段 `active_task_focus`
- `GraphEngine.build_turn_handoff()` 现在会从 `LongTermMemory.entries` 中提取 `project_focus.active_task`
- `DefaultGraphPlanner` 的 continue 摘要现在会消费 `active_task_focus`
- planner 继续摘要不再只知道“有几条长期记忆”，而是开始知道当前明确激活的任务焦点
- 补齐 Rust 测试，覆盖：
  - graph handoff 能从 retrieval 里的长期记忆提取 `PA-018`
  - planner continue 摘要能显示 `当前激活任务：PA-018`

## 验证

已通过：

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib graph::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib planner::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- `graph::tests` 共 `11` 条测试全部通过
- `planner::tests` 共 `14` 条测试全部通过
- Rust `lib` 全量 `109` 条测试全部通过
- `cargo check` 重新通过

## 当前结果

- `PA-018` 在 `A. 结构边界` 上继续前进，因为 `project_focus.active_task` 不再只停留在 memory 存储层
- `PA-018` 在 `C. runtime 接入` 上也有增量证据，因为 graph/planner 已开始消费 retrieval 提取出的结构化项目事实
- retrieval boundary 更接近“稳定事实上浮后即可被更高层默认消费”，而不只是把事实存起来

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 逐项补齐 `PA-018` 收口前的最终验收证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然继续增多，但整体覆盖仍不足
- 现有验证已经很强，但还没有形成足以直接关单的终态证明
